// Written by Kone Tshivhinda & Claude
//! A per-account request limit, so one signed-in caller cannot use the whole
//! server (THREAT-MODEL threat 11).
//!
//! A fixed window of one minute: an account may make `per_minute` requests in
//! a minute; the next one is refused with the number of seconds until the
//! window starts again. A refused request does not count and does not extend
//! the wait.
//!
//! The map is in this process's memory, so the limits apply per server. The
//! deployment runs one server; if that changes, this becomes per server and
//! the limit should move to the shared database or the edge.

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// How long one window lasts.
const WINDOW: Duration = Duration::from_secs(60);

/// Requests used in the current window, and when that window started.
struct Window {
    started: Instant,
    used: u32,
}

/// Counts each account's requests per minute.
pub struct RateLimiter {
    per_minute: NonZeroU32,
    windows: Mutex<HashMap<Uuid, Window>>,
}

impl RateLimiter {
    /// Allows `per_minute` requests per account in each minute.
    pub fn new(per_minute: NonZeroU32) -> Self {
        Self {
            per_minute,
            windows: Mutex::new(HashMap::new()),
        }
    }

    /// Counts one request from `tenant` at `now`. `Ok(())` if it is allowed;
    /// `Err(seconds)` if the account must wait that many whole seconds.
    pub fn check(&self, tenant: Uuid, now: Instant) -> Result<(), u64> {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limiter(per_minute: u32) -> RateLimiter {
        RateLimiter::new(NonZeroU32::new(per_minute).unwrap())
    }

    fn tenant(id: u128) -> Uuid {
        Uuid::from_u128(id)
    }

    #[test]
    fn allows_exactly_the_limit_and_refuses_the_next() {
        let limiter = limiter(3);
        let now = Instant::now();
        for _ in 0..3 {
            assert_eq!(limiter.check(tenant(1), now), Ok(()));
        }
        assert!(limiter.check(tenant(1), now).is_err());
    }

    #[test]
    fn the_refusal_says_when_the_window_starts_again() {
        let limiter = limiter(1);
        let start = Instant::now();
        assert_eq!(limiter.check(tenant(1), start), Ok(()));
        assert_eq!(
            limiter.check(tenant(1), start + Duration::from_secs(15)),
            Err(45)
        );
        assert_eq!(
            limiter.check(tenant(1), start + Duration::from_millis(59_500)),
            Err(1),
            "part of a second still has to wait a whole second"
        );
    }

    #[test]
    fn a_new_window_allows_requests_again() {
        let limiter = limiter(1);
        let start = Instant::now();
        assert_eq!(limiter.check(tenant(1), start), Ok(()));
        assert!(limiter.check(tenant(1), start + Duration::from_secs(59)).is_err());
        assert_eq!(
            limiter.check(tenant(1), start + Duration::from_secs(60)),
            Ok(())
        );
    }

    #[test]
    fn each_account_has_its_own_window() {
        let limiter = limiter(1);
        let now = Instant::now();
        assert_eq!(limiter.check(tenant(1), now), Ok(()));
        assert!(limiter.check(tenant(1), now).is_err());
        assert_eq!(limiter.check(tenant(2), now), Ok(()));
    }

    #[test]
    fn a_refused_request_does_not_count_or_extend_the_wait() {
        let limiter = limiter(1);
        let start = Instant::now();
        assert_eq!(limiter.check(tenant(1), start), Ok(()));
        for _ in 0..5 {
            assert!(limiter.check(tenant(1), start + Duration::from_secs(30)).is_err());
        }
        assert_eq!(
            limiter.check(tenant(1), start + Duration::from_secs(60)),
            Ok(())
        );
    }

    #[test]
    fn a_window_is_not_extended_by_later_requests() {
        let limiter = limiter(2);
        let start = Instant::now();
        assert_eq!(limiter.check(tenant(1), start), Ok(()));
        assert_eq!(
            limiter.check(tenant(1), start + Duration::from_secs(50)),
            Ok(())
        );
        assert_eq!(
            limiter.check(tenant(1), start + Duration::from_secs(60)),
            Ok(()),
            "the window starts when it started, not at the last request"
        );
    }
}
