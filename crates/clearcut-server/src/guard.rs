// Written by Kone Tshivhinda & Claude
//! What stands in front of the API before a key is checked.
//!
//! - A per-address request limit and a failed-authentication budget. Both are checked in
//!   [`crate::auth::Authed`] before the authenticator is called, so a flood of wrong keys costs
//!   a map lookup rather than a database lookup. At the failure budget the address waits
//!   [`GuardConfig::block`] before it is heard again.
//! - A concurrency cap and a request timeout, applied to every route.
//! - The response headers the page and API were missing, a request id, and one access line per
//!   request.
//!
//! The address is the socket peer, or the last `X-Forwarded-For` entry when
//! [`GuardConfig::trust_proxy`] is on. Caddy appends the address it saw, so the last entry is
//! the real one even when a client sent fake earlier entries. The setting is off by default:
//! a server reachable directly must never trust a header the client controls.
//!
//! The maps live in this process. That is exact for the one server `docs/DEPLOY.md` describes;
//! running several servers would need a shared store (the scale track).

use crate::auth::AppState;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, Extensions, HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::num::{NonZeroU32, NonZeroUsize};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tracing::Level;
use uuid::Uuid;

/// How long one address window lasts.
const WINDOW: Duration = Duration::from_secs(60);

/// The most addresses remembered at once. Beyond this the oldest window is dropped, so a
/// distributed flood cannot grow the map without bound.
const MAX_ADDRESSES: usize = 65_536;

/// The request id header, in both directions.
const REQUEST_ID: &str = "x-request-id";

/// The longest request id accepted from a caller.
const REQUEST_ID_MAX: usize = 64;

/// What the limits are. Built once at startup from the settings.
#[derive(Debug, Clone, Copy)]
pub struct GuardConfig {
    /// Requests an address may make in a minute, before authentication.
    pub per_ip_per_minute: NonZeroU32,
    /// Failed credentials an address may present in a minute before it is blocked.
    pub failures_per_minute: NonZeroU32,
    /// How long a blocked address waits.
    pub block: Duration,
    /// Requests in flight before the server sheds load.
    pub max_concurrent: NonZeroUsize,
    /// The longest a request may take before it is cancelled.
    pub timeout: Duration,
    /// Trust the last `X-Forwarded-For` entry. Only behind a proxy that appends it.
    pub trust_proxy: bool,
}

/// Where the request came from, if the transport or the proxy said.
#[derive(Debug, Clone, Copy)]
pub struct ClientAddr(pub Option<IpAddr>);

/// One address's current minute.
struct Address {
    started: Instant,
    requests: u32,
    failures: u32,
    blocked_until: Option<Instant>,
}

/// The limits, the permits and the remembered addresses.
pub struct Guard {
    config: GuardConfig,
    permits: Semaphore,
    addresses: Mutex<HashMap<IpAddr, Address>>,
}

impl Guard {
    pub fn new(config: GuardConfig) -> Self {
        Self {
            permits: Semaphore::new(config.max_concurrent.get()),
            config,
            addresses: Mutex::new(HashMap::new()),
        }
    }

    /// One request from `ip` at `now`: `Ok` if it may go on to authentication, `Err(seconds)`
    /// if the address is blocked or has used its minute.
    pub fn before_auth(&self, ip: IpAddr, now: Instant) -> Result<(), u64> {
        let mut addresses = self.lock();
        if !addresses.contains_key(&ip) && addresses.len() >= MAX_ADDRESSES {
            evict_oldest(&mut addresses);
        }
        let address = addresses.entry(ip).or_insert(Address {
            started: now,
            requests: 0,
            failures: 0,
            blocked_until: None,
        });
        if let Some(until) = address.blocked_until {
            if until > now {
                return Err(seconds_until(until, now));
            }
            address.blocked_until = None;
        }
        if now.saturating_duration_since(address.started) >= WINDOW {
            *address = Address {
                started: now,
                requests: 0,
                failures: 0,
                blocked_until: None,
            };
        }
        if address.requests >= self.config.per_ip_per_minute.get() {
            return Err(seconds_until(address.started + WINDOW, now));
        }
        address.requests += 1;
        Ok(())
    }

    /// A missing, malformed or rejected credential. Enough of them in one minute block the
    /// address for [`GuardConfig::block`].
    pub fn record_failure(&self, ip: IpAddr, now: Instant) {
        let mut addresses = self.lock();
        if !addresses.contains_key(&ip) && addresses.len() >= MAX_ADDRESSES {
            evict_oldest(&mut addresses);
        }
        let address = addresses.entry(ip).or_insert(Address {
            started: now,
            requests: 0,
            failures: 0,
            blocked_until: None,
        });
        if now.saturating_duration_since(address.started) >= WINDOW {
            *address = Address {
                started: now,
                requests: 0,
                failures: 0,
                blocked_until: None,
            };
        }
        address.failures += 1;
        if address.failures >= self.config.failures_per_minute.get() {
            address.blocked_until = Some(now + self.config.block);
        }
    }

    /// The client's address: the last forwarded entry when the proxy is trusted, else the
    /// socket peer. `None` when neither is known.
    pub fn client_ip(&self, headers: &HeaderMap, extensions: &Extensions) -> Option<IpAddr> {
        if self.config.trust_proxy {
            if let Some(ip) = forwarded_for(headers) {
                return Some(ip);
            }
        }
        extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|connect| connect.0.ip())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<IpAddr, Address>> {
        self.addresses
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[cfg(test)]
    fn tracked(&self) -> usize {
        self.lock().len()
    }

    #[cfg(test)]
    fn tracks(&self, ip: IpAddr) -> bool {
        self.lock().contains_key(&ip)
    }
}

/// The middleware: address, request id, permit, timeout, headers and the access line.
pub(crate) async fn guard_request(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    let guard = state.guard();
    let ip = guard.and_then(|guard| guard.client_ip(request.headers(), request.extensions()));
    request.extensions_mut().insert(ClientAddr(ip));

    let id = request_id(&request);
    request.headers_mut().insert(
        HeaderName::from_static(REQUEST_ID),
        HeaderValue::from_str(&id).expect("a UUID is a valid header value"),
    );

    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let started = Instant::now();

    let mut response = match guard {
        None => next.run(request).await,
        Some(guard) => match guard.permits.try_acquire() {
            Err(_) => busy(),
            Ok(_permit) => match tokio::time::timeout(guard.config.timeout, next.run(request)).await {
                Ok(response) => response,
                Err(_) => timed_out(),
            },
        },
    };

    add_security_headers(&mut response);
    response.headers_mut().insert(
        HeaderName::from_static(REQUEST_ID),
        HeaderValue::from_str(&id).expect("a UUID is a valid header value"),
    );

    let status = response.status();
    let millis = started.elapsed().as_millis();
    let level = if status.is_server_error() {
        Level::ERROR
    } else if status.is_client_error() {
        Level::WARN
    } else if path == "/health" {
        Level::DEBUG
    } else {
        Level::INFO
    };
    let ip = ip.map_or_else(|| "-".to_string(), |ip| ip.to_string());
    // `event!` wants a level it can fold into a static callsite, so the shared fields are
    // written once and the level is chosen by a literal in each arm.
    macro_rules! access {
        ($level:expr) => {
            tracing::event!(
                $level,
                method = %method,
                path = %path,
                status = status.as_u16(),
                ms = millis,
                ip = %ip,
                id = %id,
                "request"
            )
        };
    }
    match level {
        Level::ERROR => access!(Level::ERROR),
        Level::WARN => access!(Level::WARN),
        Level::DEBUG => access!(Level::DEBUG),
        _ => access!(Level::INFO),
    }

    response
}

/// The id to use: a well-formed one from the caller, else a fresh UUID. A caller cannot put
/// arbitrary text into the log through this header.
fn request_id(request: &Request) -> String {
    request
        .headers()
        .get(REQUEST_ID)
        .and_then(|value| value.to_str().ok())
        .filter(|value| {
            !value.is_empty()
                && value.len() <= REQUEST_ID_MAX
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
        .map(str::to_string)
        .unwrap_or_else(|| Uuid::new_v4().to_string())
}

fn add_security_headers(response: &mut Response) {
    for (name, value) in [
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (header::REFERRER_POLICY, "no-referrer"),
        (
            HeaderName::from_static("permissions-policy"),
            "camera=(), microphone=(), geolocation=(), payment=(), usb=()",
        ),
        (
            HeaderName::from_static("cross-origin-opener-policy"),
            "same-origin",
        ),
        (
            HeaderName::from_static("cross-origin-resource-policy"),
            "same-origin",
        ),
    ] {
        response
            .headers_mut()
            .entry(name)
            .or_insert(HeaderValue::from_static(value));
    }
}

fn busy() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(header::RETRY_AFTER, "1")],
        "the server is busy; try again shortly",
    )
        .into_response()
}

fn timed_out() -> Response {
    (StatusCode::REQUEST_TIMEOUT, "the request took too long").into_response()
}

/// The last address in the last `X-Forwarded-For` header line. Caddy appends, so this is the
/// address it saw. A malformed entry is ignored rather than trusted.
fn forwarded_for(headers: &HeaderMap) -> Option<IpAddr> {
    headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .filter_map(|value| value.rsplit(',').next())
        .next_back()
        .map(str::trim)
        .and_then(|entry| entry.parse().ok())
}

fn evict_oldest(addresses: &mut HashMap<IpAddr, Address>) {
    if let Some(oldest) = addresses
        .iter()
        .min_by_key(|(_, address)| address.started)
        .map(|(ip, _)| *ip)
    {
        addresses.remove(&oldest);
    }
}

/// Whole seconds until `until`, rounded up, so a caller never retries early.
fn seconds_until(until: Instant, now: Instant) -> u64 {
    let wait = until.saturating_duration_since(now);
    wait.as_secs() + u64::from(wait.subsec_nanos() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn config(per_ip: u32, failures: u32) -> GuardConfig {
        GuardConfig {
            per_ip_per_minute: NonZeroU32::new(per_ip).unwrap(),
            failures_per_minute: NonZeroU32::new(failures).unwrap(),
            block: Duration::from_secs(60),
            max_concurrent: NonZeroUsize::new(64).unwrap(),
            timeout: Duration::from_secs(5),
            trust_proxy: false,
        }
    }

    fn ip(last: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, last))
    }

    #[test]
    fn allows_exactly_the_address_limit_and_refuses_the_next() {
        let guard = Guard::new(config(3, 100));
        let now = Instant::now();
        for _ in 0..3 {
            assert_eq!(guard.before_auth(ip(1), now), Ok(()));
        }
        assert!(guard.before_auth(ip(1), now).is_err());
    }

    #[test]
    fn the_refusal_says_when_the_window_starts_again() {
        let guard = Guard::new(config(1, 100));
        let start = Instant::now();
        assert_eq!(guard.before_auth(ip(1), start), Ok(()));
        assert_eq!(guard.before_auth(ip(1), start + Duration::from_secs(15)), Err(45));
        assert_eq!(
            guard.before_auth(ip(1), start + Duration::from_millis(59_500)),
            Err(1),
            "part of a second still waits a whole one"
        );
    }

    #[test]
    fn a_new_window_allows_requests_again() {
        let guard = Guard::new(config(1, 100));
        let start = Instant::now();
        assert_eq!(guard.before_auth(ip(1), start), Ok(()));
        assert!(guard.before_auth(ip(1), start + Duration::from_secs(59)).is_err());
        assert_eq!(guard.before_auth(ip(1), start + Duration::from_secs(60)), Ok(()));
    }

    #[test]
    fn each_address_has_its_own_window() {
        let guard = Guard::new(config(1, 100));
        let now = Instant::now();
        assert_eq!(guard.before_auth(ip(1), now), Ok(()));
        assert!(guard.before_auth(ip(1), now).is_err());
        assert_eq!(guard.before_auth(ip(2), now), Ok(()));
    }

    #[test]
    fn enough_failures_block_the_address_until_the_wait_is_over() {
        let guard = Guard::new(config(1000, 2));
        let start = Instant::now();
        guard.record_failure(ip(1), start);
        guard.record_failure(ip(1), start + Duration::from_secs(5));
        assert_eq!(
            guard.before_auth(ip(1), start + Duration::from_secs(6)),
            Err(59),
            "blocked for a minute from the second failure"
        );
        assert_eq!(
            guard.before_auth(ip(1), start + Duration::from_secs(65)),
            Ok(()),
            "the block and the window are over"
        );
    }

    #[test]
    fn a_successful_authentication_does_not_clear_failures() {
        let guard = Guard::new(config(1000, 2));
        let start = Instant::now();
        guard.record_failure(ip(1), start);
        assert_eq!(
            guard.before_auth(ip(1), start),
            Ok(()),
            "a good request in between"
        );
        guard.record_failure(ip(1), start + Duration::from_secs(1));
        assert!(guard.before_auth(ip(1), start + Duration::from_secs(2)).is_err());
    }

    #[test]
    fn the_forwarded_header_is_only_trusted_when_configured() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("1.1.1.1, 203.0.113.7"),
        );
        let extensions = Extensions::new();
        let trusted = Guard::new(GuardConfig {
            trust_proxy: true,
            ..config(100, 100)
        });
        assert_eq!(
            trusted.client_ip(&headers, &extensions),
            Some("203.0.113.7".parse().unwrap()),
            "the last entry is the one Caddy appended"
        );
        let direct = Guard::new(config(100, 100));
        assert_eq!(
            direct.client_ip(&headers, &extensions),
            None,
            "a direct client cannot choose its address"
        );
    }

    #[test]
    fn the_socket_address_is_used_when_no_forwarded_one_is_trusted() {
        let mut extensions = Extensions::new();
        extensions.insert(ConnectInfo(SocketAddr::from(([10, 0, 0, 9], 443))));
        let guard = Guard::new(config(100, 100));
        assert_eq!(
            guard.client_ip(&HeaderMap::new(), &extensions),
            Some(IpAddr::from([10, 0, 0, 9]))
        );
    }

    #[test]
    fn an_unparseable_forwarded_entry_falls_back_to_the_socket() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("not-an-ip"));
        let mut extensions = Extensions::new();
        extensions.insert(ConnectInfo(SocketAddr::from(([10, 0, 0, 8], 443))));
        let guard = Guard::new(GuardConfig {
            trust_proxy: true,
            ..config(100, 100)
        });
        assert_eq!(
            guard.client_ip(&headers, &extensions),
            Some(IpAddr::from([10, 0, 0, 8]))
        );
    }

    #[test]
    fn the_oldest_address_is_evicted_when_the_map_is_full() {
        let guard = Guard::new(config(1_000_000, 1000));
        let now = Instant::now();
        // A distinct start per address makes "the oldest" unambiguous; one microsecond apart
        // keeps them all inside the same window.
        for i in 1..=MAX_ADDRESSES as u32 {
            let at = now + Duration::from_micros(u64::from(i));
            assert!(guard.before_auth(IpAddr::V4(Ipv4Addr::from(i)), at).is_ok());
        }
        assert_eq!(guard.tracked(), MAX_ADDRESSES);
        let new_address = IpAddr::V4(Ipv4Addr::from(0));
        assert!(guard.before_auth(new_address, now).is_ok());
        assert_eq!(guard.tracked(), MAX_ADDRESSES, "the map stays bounded");
        assert!(guard.tracks(new_address));
        assert!(
            !guard.tracks(IpAddr::V4(Ipv4Addr::from(1))),
            "the first window made room"
        );
    }
}
