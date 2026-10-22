// Written by Kone Tshivhinda & Claude
//! The HTTP client against a stub worker (a tiny server that answers as told),
//! so every status, limit and failure can be produced on demand. The real
//! worker is exercised separately by `tests/live.rs`.

use super::*;
use crate::{DocumentKind, DocumentProblem, DocumentProcessor, ProcessError};
use axum::body::Bytes as BodyBytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::Router;
use bytes::Bytes;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::net::TcpListener;

const TOKEN: &str = "t0ken-0123456789-0123456789-0123456789";

// ---------- a stub worker ----------

#[derive(Debug, Clone)]
struct Seen {
    method: Method,
    path: String,
    headers: HeaderMap,
    body: Vec<u8>,
}

#[derive(Clone)]
struct Script {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    delay: Duration,
    seen: Arc<Mutex<Vec<Seen>>>,
}

struct Stub {
    url: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Stub {
    fn requests(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

async fn answer(
    State(script): State<Script>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: BodyBytes,
) -> Response {
    script.seen.lock().unwrap().push(Seen {
        method,
        path: uri.path().to_string(),
        headers,
        body: body.to_vec(),
    });
    tokio::time::sleep(script.delay).await;
    let mut response = (StatusCode::from_u16(script.status).unwrap(), script.body.clone()).into_response();
    for (name, value) in &script.headers {
        response.headers_mut().insert(
            axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
            value.parse().unwrap(),
        );
    }
    response
}

async fn serve(script: Script) -> Stub {
    let seen = script.seen.clone();
    let app = Router::new().fallback(answer).with_state(script);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Stub { url, seen }
}

fn script(status: u16, body: impl Into<Vec<u8>>) -> Script {
    Script {
        status,
        headers: Vec::new(),
        body: body.into(),
        delay: Duration::ZERO,
        seen: Arc::new(Mutex::new(Vec::new())),
    }
}

async fn stub(status: u16, body: impl Into<Vec<u8>>) -> Stub {
    serve(script(status, body)).await
}

const GOOD: &str = r#"{"media_type":"application/pdf","pages":4,"pages_with_text":2,"has_text_layer":true}"#;

fn worker(stub: &Stub) -> HttpWorker {
    HttpWorker::new(&stub.url, TOKEN).unwrap()
}

async fn inspect(stub: &Stub, document: &'static [u8]) -> Result<crate::Inspection, ProcessError> {
    worker(stub).inspect(Bytes::from_static(document)).await
}

// ---------- what is sent ----------

#[tokio::test]
async fn it_posts_the_bytes_as_they_are_with_the_token_and_nothing_else_of_note() {
    const DOCUMENT: &[u8] = b"%PDF-1.7 the document bytes \x00\xff";
    let s = stub(200, GOOD).await;
    inspect(&s, DOCUMENT).await.unwrap();
    let requests = s.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method, Method::POST);
    assert_eq!(request.path, "/v1/inspect");
    assert_eq!(request.body, DOCUMENT);
    assert_eq!(request.headers["authorization"], format!("Bearer {TOKEN}"));
    assert_eq!(request.headers["content-type"], "application/octet-stream");
    assert!(request.headers["user-agent"]
        .to_str()
        .unwrap()
        .starts_with("clearcut-docproc/"));
    assert_eq!(
        request.headers["content-length"],
        DOCUMENT.len().to_string().as_str()
    );
    assert!(request.headers.get("cookie").is_none());
}

#[tokio::test]
async fn a_base_url_with_a_path_prefix_or_trailing_slash_works() {
    for suffix in ["/worker", "/worker/", "/a/b"] {
        let s = stub(200, GOOD).await;
        let url = format!("{}{suffix}", s.url);
        HttpWorker::new(&url, TOKEN)
            .unwrap()
            .inspect(Bytes::from_static(b"x"))
            .await
            .unwrap();
        let expected = format!("{}/v1/inspect", suffix.trim_end_matches('/'));
        assert_eq!(s.requests()[0].path, expected, "{suffix}");
    }
}

// ---------- a good answer ----------

#[tokio::test]
async fn a_good_answer_becomes_an_inspection() {
    let s = stub(200, GOOD).await;
    let result = inspect(&s, b"x").await.unwrap();
    assert_eq!(result.kind(), DocumentKind::Pdf);
    assert_eq!((result.pages(), result.pages_with_text()), (4, 2));
    assert!(result.has_text_layer());
}

#[tokio::test]
async fn fields_the_client_does_not_know_yet_are_ignored() {
    let body = r#"{"media_type":"application/pdf","pages":1,"pages_with_text":0,"has_text_layer":false,"language":"en","future":{"a":1}}"#;
    let s = stub(200, body).await;
    let result = inspect(&s, b"x").await.unwrap();
    assert_eq!((result.pages(), result.has_text_layer()), (1, false));
}

#[tokio::test]
async fn an_answer_that_is_wrong_is_a_bad_response_not_a_guess() {
    let bad_bodies = [
        "",
        "not json",
        "[]",
        "null",
        r#"{}"#,
        r#"{"pages":1,"pages_with_text":0,"has_text_layer":false}"#,
        r#"{"media_type":"application/pdf","pages":"1","pages_with_text":0,"has_text_layer":false}"#,
        r#"{"media_type":"application/pdf","pages":-1,"pages_with_text":0,"has_text_layer":false}"#,
        r#"{"media_type":"application/pdf","pages":1.5,"pages_with_text":0,"has_text_layer":false}"#,
        r#"{"media_type":"application/pdf","pages":5000000000,"pages_with_text":0,"has_text_layer":false}"#,
        r#"{"media_type":"image/png","pages":1,"pages_with_text":0,"has_text_layer":false}"#,
        r#"{"media_type":"application/pdf","pages":1,"pages_with_text":2,"has_text_layer":true}"#,
        r#"{"media_type":"application/pdf","pages":3,"pages_with_text":2,"has_text_layer":false}"#,
        r#"{"media_type":"application/pdf","pages":3,"pages_with_text":0,"has_text_layer":true}"#,
        r#"{"media_type":"application/pdf","pages":3,"pages_with_text":2}"#,
    ];
    for body in bad_bodies {
        let s = stub(200, body).await;
        let error = inspect(&s, b"x").await.unwrap_err();
        assert!(
            matches!(error, ProcessError::BadResponse(_)),
            "{body:?} gave {error:?}"
        );
    }
}

#[tokio::test]
async fn a_bad_response_never_repeats_what_the_server_said() {
    let s = stub(
        200,
        r#"{"media_type":"application/pdf","pages":"SECRET-FROM-SERVER"}"#,
    )
    .await;
    let error = inspect(&s, b"x").await.unwrap_err();
    assert!(!error.to_string().contains("SECRET"), "{error}");
    assert!(!format!("{error:?}").contains("SECRET"), "{error:?}");
}

// ---------- every status ----------

#[tokio::test]
async fn each_status_the_worker_can_give_maps_to_a_named_error() {
    use DocumentProblem::*;
    let cases: Vec<(u16, &str, ProcessError)> = vec![
        (
            400,
            r#"{"error":"the file is empty"}"#,
            ProcessError::Document(Empty),
        ),
        (
            413,
            r#"{"error":"file too large","max_bytes":100}"#,
            ProcessError::Document(TooLarge { max_bytes: 100 }),
        ),
        (
            415,
            r#"{"error":"unsupported file type"}"#,
            ProcessError::Document(UnsupportedType),
        ),
        (
            422,
            r#"{"error":"the document is encrypted"}"#,
            ProcessError::Document(Encrypted),
        ),
        (
            422,
            r#"{"error":"the document could not be read"}"#,
            ProcessError::Document(Unreadable),
        ),
        (
            422,
            r#"{"error":"too many pages","max_pages":3}"#,
            ProcessError::Document(TooManyPages { max_pages: 3 }),
        ),
        (401, r#"{"error":"authentication failed"}"#, ProcessError::Auth),
        (403, "", ProcessError::Auth),
        (
            404,
            r#"{"error":"not found"}"#,
            ProcessError::Unexpected { status: 404 },
        ),
        (
            405,
            r#"{"error":"method not allowed"}"#,
            ProcessError::Unexpected { status: 405 },
        ),
        (429, "", ProcessError::Unexpected { status: 429 }),
        (
            500,
            r#"{"error":"internal error"}"#,
            ProcessError::Server { status: 500 },
        ),
        (502, "", ProcessError::Server { status: 502 }),
        (503, "", ProcessError::Server { status: 503 }),
        (
            504,
            r#"{"error":"the document took too long to inspect"}"#,
            ProcessError::Server { status: 504 },
        ),
    ];
    for (status, body, expected) in cases {
        let s = stub(status, body).await;
        let got = inspect(&s, b"x").await.unwrap_err();
        assert_eq!(
            format!("{got:?}"),
            format!("{expected:?}"),
            "status {status} body {body:?}"
        );
    }
}

#[tokio::test]
async fn an_error_the_client_does_not_recognise_is_still_mapped_by_its_status() {
    for (status, body) in [
        (422, "garbage, not json"),
        (422, r#"{"error":"something new"}"#),
        (422, ""),
    ] {
        let s = stub(status, body).await;
        assert!(matches!(
            inspect(&s, b"x").await,
            Err(ProcessError::Document(DocumentProblem::Unreadable))
        ));
    }
    // A limit that is missing or silly does not become a made-up number.
    for body in [
        r#"{"error":"too many pages"}"#,
        r#"{"error":"too many pages","max_pages":"lots"}"#,
        r#"{"error":"too many pages","max_pages":-1}"#,
    ] {
        let s = stub(422, body).await;
        assert!(
            matches!(
                inspect(&s, b"x").await,
                Err(ProcessError::Document(DocumentProblem::Unreadable))
            ),
            "{body}"
        );
    }
    let s = stub(413, r#"{"error":"file too large"}"#).await;
    assert!(matches!(
        inspect(&s, b"x").await,
        Err(ProcessError::Document(DocumentProblem::TooLarge { max_bytes: 0 }))
    ));
}

#[tokio::test]
async fn an_error_never_repeats_what_the_server_said() {
    let s = stub(500, r#"{"error":"SECRET-SERVER-DETAIL"}"#).await;
    let error = inspect(&s, b"x").await.unwrap_err();
    assert!(!error.to_string().contains("SECRET"));
    assert!(!format!("{error:?}").contains("SECRET"));
}

// ---------- hostile or broken servers ----------

#[tokio::test]
async fn a_redirect_is_not_followed_so_the_token_goes_nowhere_else() {
    let elsewhere = stub(200, GOOD).await;
    let mut redirecting = script(302, "");
    redirecting
        .headers
        .push(("location".into(), format!("{}/v1/inspect", elsewhere.url)));
    let s = serve(redirecting).await;
    let error = inspect(&s, b"x").await.unwrap_err();
    assert!(
        matches!(error, ProcessError::Unexpected { status: 302 }),
        "{error:?}"
    );
    assert!(
        elsewhere.requests().is_empty(),
        "the redirect target must never be contacted"
    );
}

#[tokio::test]
async fn an_enormous_answer_is_refused_without_reading_it_all() {
    let started = Instant::now();
    let s = stub(200, vec![b' '; 10 * 1024 * 1024]).await;
    let error = inspect(&s, b"x").await.unwrap_err();
    assert!(matches!(error, ProcessError::BadResponse(_)), "{error:?}");
    assert!(started.elapsed() < Duration::from_secs(10));

    // An enormous error body does not stop the status from being understood.
    let s = stub(422, vec![b'x'; 10 * 1024 * 1024]).await;
    assert!(matches!(
        inspect(&s, b"x").await,
        Err(ProcessError::Document(DocumentProblem::Unreadable))
    ));
}

#[tokio::test]
async fn a_worker_that_takes_too_long_is_a_timeout() {
    let mut slow = script(200, GOOD);
    slow.delay = Duration::from_secs(3);
    let s = serve(slow).await;
    let started = Instant::now();
    let error = HttpWorker::new(&s.url, TOKEN)
        .unwrap()
        .with_timeout(Duration::from_millis(300))
        .unwrap()
        .inspect(Bytes::from_static(b"x"))
        .await
        .unwrap_err();
    assert!(matches!(error, ProcessError::Timeout), "{error:?}");
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn a_worker_that_is_not_there_is_unavailable() {
    // A port that was free a moment ago and has nothing listening now.
    let port = {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        listener.local_addr().unwrap().port()
    };
    let error = HttpWorker::new(&format!("http://127.0.0.1:{port}"), TOKEN)
        .unwrap()
        .inspect(Bytes::from_static(b"x"))
        .await
        .unwrap_err();
    assert!(matches!(error, ProcessError::Unavailable), "{error:?}");
}

#[tokio::test]
async fn a_connection_dropped_in_the_middle_is_interrupted_not_unavailable() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            // Read a little of the request, then slam the door.
            let mut buffer = [0u8; 64];
            let _ = tokio::io::AsyncReadExt::read(&mut socket, &mut buffer).await;
            drop(socket);
        }
    });
    let error = HttpWorker::new(&url, TOKEN)
        .unwrap()
        .inspect(Bytes::from_static(b"x"))
        .await
        .unwrap_err();
    assert!(matches!(error, ProcessError::Interrupted), "{error:?}");
}

// ---------- setting it up ----------

#[test]
fn only_plain_http_urls_without_credentials_are_accepted() {
    for good in [
        "http://127.0.0.1:8090",
        "http://worker:8090",
        "http://worker:8090/",
        "http://worker/prefix",
    ] {
        assert!(HttpWorker::new(good, TOKEN).is_ok(), "{good}");
    }
    for (bad, why) in [
        ("", ConfigError::BadUrl),
        ("not a url", ConfigError::BadUrl),
        ("worker:8090", ConfigError::BadUrl),
        ("https://worker:8090", ConfigError::NotHttp),
        ("ftp://worker", ConfigError::NotHttp),
        ("file:///etc/passwd", ConfigError::NotHttp),
        ("http://user:pass@worker:8090", ConfigError::HasCredentials),
        ("http://user@worker:8090", ConfigError::HasCredentials),
        ("http://:secret@worker:8090", ConfigError::HasCredentials),
        ("http://worker:8090/?token=abc", ConfigError::HasQuery),
        ("http://worker:8090/#frag", ConfigError::HasQuery),
    ] {
        let got = HttpWorker::new(bad, TOKEN).unwrap_err();
        assert_eq!(got, why, "{bad:?}");
    }
}

#[test]
fn the_token_must_be_visible_ascii_so_it_cannot_smuggle_a_header() {
    for bad in [
        "",
        " ",
        "a b",
        "tab\there",
        "line\nbreak",
        "cr\rreturn",
        "é-not-ascii",
        "nul\0byte",
    ] {
        assert_eq!(
            HttpWorker::new("http://w:1", bad).unwrap_err(),
            ConfigError::BadToken,
            "{bad:?}"
        );
    }
    assert!(HttpWorker::new("http://w:1", "a").is_ok());
}

#[test]
fn the_timeout_must_be_more_than_zero() {
    let w = HttpWorker::new("http://w:1", TOKEN).unwrap();
    assert_eq!(
        w.clone().with_timeout(Duration::ZERO).unwrap_err(),
        ConfigError::BadTimeout
    );
    assert!(w.with_timeout(Duration::from_millis(1)).is_ok());
}

#[test]
fn the_default_timeout_outlasts_the_workers_own_limits() {
    // The worker gives up after 30 s plus a 2 s grace; the client must not give up first.
    assert!(DEFAULT_TIMEOUT >= Duration::from_secs(40));
    assert_eq!(MAX_RESPONSE_BYTES, 64 * 1024);
}

#[test]
fn the_token_and_the_address_are_never_printed() {
    let w = HttpWorker::new("http://secret-host.internal:8090", TOKEN).unwrap();
    for shown in [format!("{w:?}"), format!("{w:#?}")] {
        assert!(!shown.contains("t0ken"), "{shown}");
        assert!(!shown.contains("0123456789"), "{shown}");
    }
    for error in [
        ConfigError::BadUrl,
        ConfigError::NotHttp,
        ConfigError::HasCredentials,
        ConfigError::HasQuery,
        ConfigError::BadToken,
        ConfigError::BadTimeout,
    ] {
        assert!(!error.to_string().is_empty());
    }
    let from_secret = HttpWorker::new("http://user:SECRET@host", "SECRET TOKEN").unwrap_err();
    assert!(!from_secret.to_string().contains("SECRET"));
}

// ---------- more guarantees ----------

#[tokio::test]
async fn an_answer_exactly_at_the_size_limit_is_read_in_full_and_one_byte_more_is_refused() {
    let mut exact = GOOD.as_bytes().to_vec();
    exact.resize(MAX_RESPONSE_BYTES, b' ');
    let s = stub(200, exact.clone()).await;
    assert!(
        inspect(&s, b"x").await.is_ok(),
        "a body of exactly the limit is fine"
    );

    exact.push(b' ');
    let s = stub(200, exact).await;
    assert!(matches!(
        inspect(&s, b"x").await,
        Err(ProcessError::BadResponse(_))
    ));
}

#[tokio::test]
async fn a_400_that_is_not_about_an_empty_file_is_unexpected_not_the_documents_fault() {
    for body in [
        r#"{"error":"invalid request"}"#,
        "",
        "garbage",
        r#"{"error":"the file is empty!"}"#,
    ] {
        let s = stub(400, body).await;
        assert!(
            matches!(
                inspect(&s, b"x").await,
                Err(ProcessError::Unexpected { status: 400 })
            ),
            "{body:?}"
        );
    }
}

#[test]
fn the_authorization_header_is_marked_sensitive_so_it_is_redacted_in_debug_output() {
    let w = HttpWorker::new("http://w:1", TOKEN).unwrap();
    assert!(w.authorization.is_sensitive());
    assert_eq!(format!("{:?}", w.authorization), "Sensitive");
}

#[test]
fn the_edges_of_the_visible_ascii_range_decide_which_tokens_are_allowed() {
    for ok in ["!", "~", "a~b!c", "0", "Z"] {
        assert!(HttpWorker::new("http://w:1", ok).is_ok(), "{ok:?}");
    }
    for bad in ["\u{20}", "a\u{20}b", "\u{7f}", "a\u{7f}", "\u{1f}"] {
        assert_eq!(
            HttpWorker::new("http://w:1", bad).unwrap_err(),
            ConfigError::BadToken,
            "{bad:?}"
        );
    }
}

#[test]
fn the_endpoint_is_built_from_the_base_with_no_doubled_or_missing_slash() {
    let endpoint = |base: &str| HttpWorker::new(base, TOKEN).unwrap().endpoint.to_string();
    assert_eq!(endpoint("http://h:1"), "http://h:1/v1/inspect");
    assert_eq!(endpoint("http://h:1/"), "http://h:1/v1/inspect");
    assert_eq!(endpoint("http://h:1/p"), "http://h:1/p/v1/inspect");
    assert_eq!(endpoint("http://h:1/p/"), "http://h:1/p/v1/inspect");
}

#[test]
fn a_timeout_can_be_set_and_is_used() {
    let w = HttpWorker::new("http://w:1", TOKEN).unwrap();
    assert_eq!(w.timeout, DEFAULT_TIMEOUT);
    let w = w.with_timeout(Duration::from_millis(250)).unwrap();
    assert_eq!(w.timeout, Duration::from_millis(250));
}

#[test]
fn the_documented_limits_are_the_agreed_numbers() {
    assert_eq!(DEFAULT_TIMEOUT, Duration::from_secs(45));
    assert_eq!(MAX_RESPONSE_BYTES, 65_536);
}

#[tokio::test]
async fn a_422_that_names_no_usable_page_limit_is_just_unreadable() {
    for body in [
        r#"{"error":"too many pages","max_pages":0}"#,
        r#"{"error":"too many pages","max_pages":4294967296}"#,
    ] {
        let s = stub(422, body).await;
        assert!(
            matches!(
                inspect(&s, b"x").await,
                Err(ProcessError::Document(DocumentProblem::Unreadable))
            ),
            "{body}"
        );
    }
    let s = stub(422, r#"{"error":"too many pages","max_pages":4294967295}"#).await;
    assert!(matches!(
        inspect(&s, b"x").await,
        Err(ProcessError::Document(DocumentProblem::TooManyPages {
            max_pages: u32::MAX
        }))
    ));
}

#[tokio::test]
async fn the_whole_5xx_range_is_the_workers_failure() {
    for status in [500u16, 501, 598, 599] {
        let s = stub(status, "").await;
        assert!(
            matches!(inspect(&s, b"x").await, Err(ProcessError::Server { status: got }) if got == status),
            "{status}"
        );
    }
    for status in [499u16, 600] {
        let s = stub(status, "").await;
        assert!(
            matches!(inspect(&s, b"x").await, Err(ProcessError::Unexpected { status: got }) if got == status),
            "{status}"
        );
    }
}
