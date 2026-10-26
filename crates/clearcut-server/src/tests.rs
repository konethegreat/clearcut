// Written by Kone Tshivhinda & Claude
//! Most tests use a fake authenticator, so they need no database. The last
//! group runs the real `Store` against Postgres (`DATABASE_URL`).

use super::*;
use axum::body::Body;
use axum::http::{header, HeaderMap, Request, Response, StatusCode};
use axum::routing::get;
use clearcut_core::auth::{Principal, Scope};
use http_body_util::BodyExt;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;
use uuid::Uuid;

// ---------- test doubles ----------

pub(crate) const TENANT: u128 = 0xA;
pub(crate) const READER: &str = "ccut_reader";
pub(crate) const WRITER: &str = "ccut_writer";
pub(crate) const ADMIN_ONLY: &str = "ccut_admin_only";
const FLAKY: &str = "ccut_db_is_down";

fn principal(key: u128, scopes: &[Scope]) -> Principal {
    Principal::new(
        Uuid::from_u128(TENANT),
        Uuid::from_u128(key),
        scopes.iter().copied(),
    )
}

/// An authenticator that knows a few fixed keys and records what it was asked.
pub(crate) struct FakeAuth {
    principals: HashMap<String, Principal>,
    unavailable: HashSet<String>,
    asked: Mutex<Vec<String>>,
}

impl FakeAuth {
    pub(crate) fn new() -> Arc<Self> {
        let mut principals = HashMap::new();
        principals.insert(
            READER.to_string(),
            principal(1, &[Scope::DocumentsRead, Scope::EventsRead]),
        );
        principals.insert(WRITER.to_string(), principal(2, &[Scope::DocumentsWrite]));
        principals.insert(ADMIN_ONLY.to_string(), principal(3, &[Scope::KeysAdmin]));
        Arc::new(Self {
            principals,
            unavailable: HashSet::from([FLAKY.to_string()]),
            asked: Mutex::new(Vec::new()),
        })
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

impl Authenticator for FakeAuth {
    fn authenticate<'a>(
        &'a self,
        presented: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Principal, AuthFailure>> + Send + 'a>> {
        Box::pin(async move {
            self.asked.lock().unwrap().push(presented.to_string());
            if self.unavailable.contains(presented) {
                return Err(AuthFailure::Unavailable);
            }
            self.principals
                .get(presented)
                .cloned()
                .ok_or(AuthFailure::Rejected)
        })
    }
}

/// The real routes plus a few test-only ones that exercise scope checks and
/// what a handler can see of the request.
fn router_with(fake: &Arc<FakeAuth>) -> Router {
    async fn needs_write(auth: Authed) -> Result<&'static str, ApiProblem> {
        auth.require(Scope::DocumentsWrite)?;
        Ok("written")
    }
    async fn needs_read(auth: Authed) -> Result<&'static str, ApiProblem> {
        auth.require(Scope::DocumentsRead)?;
        Ok("read")
    }
    async fn needs_admin(auth: Authed) -> Result<&'static str, ApiProblem> {
        auth.require(Scope::KeysAdmin)?;
        Ok("administered")
    }
    async fn show_headers(headers: HeaderMap) -> String {
        let sensitive = headers
            .get(header::AUTHORIZATION)
            .is_some_and(|v| v.is_sensitive());
        format!("sensitive={sensitive}; debug={headers:?}")
    }
    let routes = routes()
        .route("/test/write", get(needs_write))
        .route("/test/read", get(needs_read))
        .route("/test/admin", get(needs_admin))
        .route("/test/headers", get(show_headers));
    finish(routes, AppState::shared(fake.clone()))
}

fn app_with_fake() -> (Router, Arc<FakeAuth>) {
    let fake = FakeAuth::new();
    (router_with(&fake), fake)
}

pub(crate) async fn send(app: &Router, request: Request<Body>) -> (StatusCode, HeaderMap, String) {
    let response: Response<Body> = app.clone().oneshot(request).await.unwrap();
    let (parts, body) = response.into_parts();
    let bytes = body.collect().await.unwrap().to_bytes();
    (
        parts.status,
        parts.headers,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

fn get_with(path: &str, authorization: Option<&str>) -> Request<Body> {
    let mut builder = Request::get(path);
    if let Some(value) = authorization {
        builder = builder.header(header::AUTHORIZATION, value);
    }
    builder.body(Body::empty()).unwrap()
}

pub(crate) fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

pub(crate) fn json(body: &str) -> serde_json::Value {
    serde_json::from_str(body).unwrap()
}

const INVOICE: &str = r#"{"number":"INV-1","currency":"ZAR","status":"draft","lines":[
    {"description":"Consulting","quantity_milli":10000,
     "unit_price":{"minor":150000,"currency":"ZAR"},"tax":"standard"}]}"#;

// ---------- public and protected routes ----------

#[tokio::test]
async fn health_is_public_and_never_asks_the_authenticator() {
    let (app, fake) = app_with_fake();
    let (status, _, body) = send(&app, get_with("/health", None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json(&body)["status"], "ok");
    assert!(fake.asked().is_empty());
}

#[tokio::test]
async fn whoami_tells_a_caller_who_its_key_is_without_any_secret() {
    let (app, _) = app_with_fake();
    let (status, _, body) = send(&app, get_with("/v1/whoami", Some(&bearer(READER)))).await;
    assert_eq!(status, StatusCode::OK);
    let v = json(&body);
    assert_eq!(v["tenant"], Uuid::from_u128(TENANT).to_string());
    assert_eq!(v["key_id"], Uuid::from_u128(1).to_string());
    assert_eq!(v["scopes"], serde_json::json!(["documents:read", "events:read"]));
    assert!(!body.contains(READER), "the key must never be echoed back");
}

#[tokio::test]
async fn the_token_reaches_the_authenticator_exactly_as_sent() {
    let (app, fake) = app_with_fake();
    send(&app, get_with("/v1/whoami", Some(&bearer(READER)))).await;
    assert_eq!(fake.asked(), [READER]);
}

#[tokio::test]
async fn the_bearer_scheme_name_is_not_case_sensitive() {
    let (app, _) = app_with_fake();
    for scheme in ["bearer", "BEARER", "Bearer", "bEaReR"] {
        let (status, ..) = send(&app, get_with("/v1/whoami", Some(&format!("{scheme} {READER}")))).await;
        assert_eq!(status, StatusCode::OK, "{scheme}");
    }
}

#[tokio::test]
async fn calculating_needs_a_valid_key_but_no_particular_scope() {
    let (app, _) = app_with_fake();
    for token in [READER, WRITER, ADMIN_ONLY] {
        let request = Request::post("/v1/invoices/calculate")
            .header(header::AUTHORIZATION, bearer(token))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(INVOICE))
            .unwrap();
        let (status, _, body) = send(&app, request).await;
        assert_eq!(status, StatusCode::OK, "{token}");
        assert_eq!(json(&body)["totals"]["gross"]["minor"], 1_725_000);
    }
}

#[tokio::test]
async fn an_empty_invoice_is_still_unprocessable_for_an_authenticated_caller() {
    let (app, _) = app_with_fake();
    let request = Request::post("/v1/invoices/calculate")
        .header(header::AUTHORIZATION, bearer(READER))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            r#"{"number":"X","currency":"ZAR","status":"draft","lines":[]}"#,
        ))
        .unwrap();
    assert_eq!(send(&app, request).await.0, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn a_line_total_given_by_the_caller_is_used_as_given_and_older_callers_are_unchanged() {
    // The pair in docs/integration/timelex.md: 1.5 hours at R333.33 is computed as R500.00, and a
    // caller that has priced the line itself at R499.99 is not rounded again.
    let (app, _) = app_with_fake();
    let calculate = |line_total: &str| {
        let body = format!(
            r#"{{"number":"T","currency":"ZAR","status":"draft","lines":[{{"description":"Advice","quantity_milli":1500,"unit_price":{{"minor":33333,"currency":"ZAR"}},"tax":"standard"{line_total}}}]}}"#
        );
        Request::post("/v1/invoices/calculate")
            .header(header::AUTHORIZATION, bearer(READER))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body))
            .unwrap()
    };
    let (status, _, body) = send(&app, calculate("")).await;
    assert_eq!(status, StatusCode::OK);
    let totals = &json(&body)["totals"];
    assert_eq!(
        (totals["net"]["minor"].as_i64(), totals["gross"]["minor"].as_i64()),
        (Some(50_000), Some(57_500))
    );

    let given = r#","line_total":{"minor":49999,"currency":"ZAR"}"#;
    let (status, _, body) = send(&app, calculate(given)).await;
    assert_eq!(status, StatusCode::OK);
    let totals = &json(&body)["totals"];
    assert_eq!(
        (
            totals["net"]["minor"].as_i64(),
            totals["vat"]["minor"].as_i64(),
            totals["gross"]["minor"].as_i64()
        ),
        (Some(49_999), Some(7_500), Some(57_499))
    );
}

#[tokio::test]
async fn authentication_happens_before_the_body_is_looked_at() {
    // Garbage with no credentials must be a 401, not a 400/415/422 that tells
    // an anonymous caller about the API's parsing.
    let (app, fake) = app_with_fake();
    for (content_type, body) in [
        ("application/json", "{not json"),
        ("text/plain", "hello"),
        ("application/json", ""),
    ] {
        let request = Request::post("/v1/invoices/calculate")
            .header(header::CONTENT_TYPE, content_type)
            .body(Body::from(body))
            .unwrap();
        assert_eq!(send(&app, request).await.0, StatusCode::UNAUTHORIZED);
    }
    assert!(fake.asked().is_empty());
}

#[tokio::test]
async fn unknown_routes_are_not_found_not_a_way_around_authentication() {
    let (app, _) = app_with_fake();
    assert_eq!(
        send(&app, get_with("/v1/nope", None)).await.0,
        StatusCode::NOT_FOUND
    );
}

// ---------- every authentication failure looks the same ----------

#[tokio::test]
async fn every_way_of_failing_to_authenticate_gets_the_identical_response() {
    let (app, _) = app_with_fake();
    let two_headers = Request::get("/v1/whoami")
        .header(header::AUTHORIZATION, bearer(READER))
        .header(header::AUTHORIZATION, bearer(READER))
        .body(Body::empty())
        .unwrap();
    let attempts: Vec<(&str, Request<Body>)> = vec![
        ("no header", get_with("/v1/whoami", None)),
        ("wrong scheme", get_with("/v1/whoami", Some("Basic dXNlcjpwYXNz"))),
        ("no token", get_with("/v1/whoami", Some("Bearer"))),
        ("empty token", get_with("/v1/whoami", Some("Bearer "))),
        (
            "token with a space in it",
            get_with("/v1/whoami", Some("Bearer two words")),
        ),
        (
            "two spaces",
            get_with("/v1/whoami", Some(&format!("Bearer  {READER}"))),
        ),
        ("token only, no scheme", get_with("/v1/whoami", Some(READER))),
        ("unknown key", get_with("/v1/whoami", Some("Bearer ccut_unknown"))),
        ("two Authorization headers", two_headers),
    ];
    let mut seen = Vec::new();
    for (what, request) in attempts {
        let (status, headers, body) = send(&app, request).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{what}");
        assert_eq!(headers.get(header::WWW_AUTHENTICATE).unwrap(), "Bearer", "{what}");
        assert_eq!(
            json(&body),
            serde_json::json!({ "error": "authentication failed" }),
            "{what}"
        );
        seen.push((status, headers.get(header::WWW_AUTHENTICATE).cloned(), body));
    }
    assert!(
        seen.windows(2).all(|pair| pair[0] == pair[1]),
        "responses must be byte-for-byte alike"
    );
}

#[tokio::test]
async fn a_header_that_is_not_text_is_just_another_failed_authentication() {
    let (app, _) = app_with_fake();
    let request = Request::get("/v1/whoami")
        .header(
            header::AUTHORIZATION,
            axum::http::HeaderValue::from_bytes(b"Bearer \xff\xfe").unwrap(),
        )
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&app, request).await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_malformed_header_never_reaches_the_authenticator() {
    let (app, fake) = app_with_fake();
    send(&app, get_with("/v1/whoami", Some("Basic abc"))).await;
    send(&app, get_with("/v1/whoami", Some("Bearer"))).await;
    send(&app, get_with("/v1/whoami", Some("Bearer "))).await;
    send(&app, get_with("/v1/whoami", None)).await;
    // Whitespace inside or before the token is malformed, not "an unknown key".
    send(&app, get_with("/v1/whoami", Some("Bearer two words"))).await;
    send(&app, get_with("/v1/whoami", Some(&format!("Bearer  {READER}")))).await;
    send(&app, get_with("/v1/whoami", Some(&format!("Bearer {READER} ")))).await;
    send(&app, get_with("/v1/whoami", Some(&format!("Bearer	{READER}")))).await;
    assert!(fake.asked().is_empty(), "asked: {:?}", fake.asked());
}

#[tokio::test]
async fn an_unavailable_authenticator_is_a_500_not_a_401() {
    // A database outage must not look like "your key is wrong", and must not
    // reveal anything about the failure.
    let (app, _) = app_with_fake();
    let (status, headers, body) = send(&app, get_with("/v1/whoami", Some(&bearer(FLAKY)))).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(headers.get(header::WWW_AUTHENTICATE).is_none());
    assert_eq!(json(&body), serde_json::json!({ "error": "internal error" }));
}

// ---------- scopes ----------

#[tokio::test]
async fn a_route_that_needs_a_scope_lets_in_only_keys_that_hold_it() {
    let (app, _) = app_with_fake();
    let (status, _, body) = send(&app, get_with("/test/write", Some(&bearer(WRITER)))).await;
    assert_eq!((status, body.as_str()), (StatusCode::OK, "written"));

    let (status, _, body) = send(&app, get_with("/test/write", Some(&bearer(READER)))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        json(&body),
        serde_json::json!({ "error": "forbidden", "required_scope": "documents:write" })
    );
}

#[tokio::test]
async fn no_scope_implies_another_even_key_administration() {
    let (app, _) = app_with_fake();
    for path in ["/test/write", "/test/read"] {
        let (status, ..) = send(&app, get_with(path, Some(&bearer(ADMIN_ONLY)))).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}");
    }
    let (status, ..) = send(&app, get_with("/test/admin", Some(&bearer(ADMIN_ONLY)))).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn without_credentials_a_scoped_route_is_401_not_403() {
    // 403 would tell an anonymous caller which scope a route needs.
    let (app, _) = app_with_fake();
    for path in ["/test/write", "/test/read", "/test/admin"] {
        let (status, _, body) = send(&app, get_with(path, None)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{path}");
        assert!(!body.contains("scope"), "{body}");
    }
}

// ---------- the key never leaks ----------

#[tokio::test]
async fn the_authorization_header_is_marked_sensitive_so_it_prints_as_redacted() {
    let secret_token = "ccut_super_secret_token_value";
    let mut fake_principals = HashMap::new();
    fake_principals.insert(secret_token.to_string(), principal(9, &[Scope::DocumentsRead]));
    let fake = Arc::new(FakeAuth {
        principals: fake_principals,
        unavailable: HashSet::new(),
        asked: Mutex::new(Vec::new()),
    });
    let app = router_with(&fake);
    let (status, _, body) = send(&app, get_with("/test/headers", Some(&bearer(secret_token)))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.starts_with("sensitive=true"), "{body}");
    assert!(
        !body.contains(secret_token),
        "the header value must be redacted in Debug output: {body}"
    );
}

/// Captures what `tracing` would print.
///
/// `tracing` remembers, per log statement, whether anyone was listening the
/// first time it ran. A per-test subscriber is therefore unreliable: if another
/// test running in parallel reaches the same statement first, it is cached as
/// "nobody is listening" and this test sees nothing. So one subscriber is
/// installed for the whole test process, and it writes into a buffer that is
/// private to the current thread (each test runs on its own thread).
#[derive(Clone, Copy)]
struct LogCapture;

thread_local! {
    static CAPTURED: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
}

impl std::io::Write for LogCapture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        CAPTURED.with(|captured| captured.borrow_mut().extend_from_slice(buf));
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogCapture {
    type Writer = LogCapture;
    fn make_writer(&'a self) -> Self::Writer {
        *self
    }
}

/// Starts a fresh capture for the calling test's thread.
fn capture_logs() -> LogCapture {
    static INSTALL: std::sync::Once = std::sync::Once::new();
    INSTALL.call_once(|| {
        let subscriber = tracing_subscriber::fmt()
            .with_writer(LogCapture)
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .finish();
        // Another test binary component may have installed one; ours then simply goes unused.
        let _ = tracing::subscriber::set_global_default(subscriber);
    });
    CAPTURED.with(|captured| captured.borrow_mut().clear());
    LogCapture
}

impl LogCapture {
    fn text(&self) -> String {
        CAPTURED.with(|captured| String::from_utf8_lossy(&captured.borrow()).into_owned())
    }
}

#[tokio::test]
async fn failed_authentication_is_logged_without_the_key() {
    let logs = capture_logs();
    let (app, _) = app_with_fake();
    let presented = "ccut_0123456789abcdef0123456789abcdef_ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
    send(&app, get_with("/v1/whoami", Some(&bearer(presented)))).await;
    send(&app, get_with("/v1/whoami", Some("Basic abc"))).await;
    let text = logs.text();
    assert!(
        text.contains("authentication failed"),
        "a failure should be logged: {text}"
    );
    assert!(!text.contains(presented), "{text}");
    assert!(!text.contains("ffffffffffff"), "{text}");
    assert!(!text.contains("Basic abc"), "{text}");
}

#[tokio::test]
async fn an_unavailable_authenticator_is_logged_without_the_key() {
    let logs = capture_logs();
    let (app, _) = app_with_fake();
    send(&app, get_with("/v1/whoami", Some(&bearer(FLAKY)))).await;
    let text = logs.text();
    assert!(text.to_lowercase().contains("unavailable"), "{text}");
    assert!(!text.contains(FLAKY), "{text}");
}

// ---------- with the real store ----------

pub(crate) mod with_postgres {
    use super::*;
    use clearcut_store::{NewApiKey, Store};
    use sqlx::PgPool;

    /// The service's view of the database: the restricted role.
    pub(crate) async fn service_store(pool: &PgPool) -> Store {
        let options = pool.connect_options().as_ref().clone().username("clearcut_app");
        let service_pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .connect_with(options)
            .await
            .unwrap();
        Store::new(service_pool)
    }

    fn real_app(store: Store) -> Router {
        app(AppState::new(store))
    }

    pub(crate) async fn first_key(pool: &PgPool, scopes: &[Scope]) -> (Uuid, String, Uuid) {
        let owner = Store::new(pool.clone());
        let tenant = owner.create_tenant("Acme").await.unwrap();
        let created = owner
            .create_api_key_as_operator(
                tenant,
                &NewApiKey {
                    name: "test".into(),
                    scopes: scopes.to_vec(),
                    lifetime_days: None,
                },
            )
            .await
            .unwrap();
        (tenant.0, created.token.expose().to_string(), created.id)
    }

    #[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
    async fn a_real_key_authenticates_and_a_revoked_one_stops_working(pool: PgPool) {
        let (tenant, token, key_id) = first_key(&pool, &[Scope::DocumentsRead]).await;
        let app = real_app(service_store(&pool).await);

        let (status, _, body) = send(&app, get_with("/v1/whoami", Some(&bearer(&token)))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json(&body)["tenant"], tenant.to_string());
        assert_eq!(json(&body)["scopes"], serde_json::json!(["documents:read"]));

        Store::new(pool.clone())
            .revoke_api_key_as_operator(clearcut_store::TenantId(tenant), key_id)
            .await
            .unwrap();
        let (status, ..) = send(&app, get_with("/v1/whoami", Some(&bearer(&token)))).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
    async fn a_wrong_secret_for_a_real_key_is_just_a_401(pool: PgPool) {
        let (_, token, _) = first_key(&pool, &[Scope::DocumentsRead]).await;
        let app = real_app(service_store(&pool).await);
        let wrong = format!("{}{}", &token[..token.len() - 4], "0000");
        let (status, headers, body) = send(&app, get_with("/v1/whoami", Some(&bearer(&wrong)))).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(headers.get(header::WWW_AUTHENTICATE).unwrap(), "Bearer");
        assert_eq!(
            json(&body),
            serde_json::json!({ "error": "authentication failed" })
        );
    }

    #[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
    async fn a_database_outage_is_a_500_not_a_401(pool: PgPool) {
        let (_, token, _) = first_key(&pool, &[Scope::DocumentsRead]).await;
        let store = service_store(&pool).await;
        store.pool().close().await; // the database "goes away"
        let app = real_app(store);
        let (status, _, body) = send(&app, get_with("/v1/whoami", Some(&bearer(&token)))).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(json(&body), serde_json::json!({ "error": "internal error" }));
    }
}
