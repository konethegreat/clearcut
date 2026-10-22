// Written by Kone Tshivhinda & Claude
use super::*;
use crate::testutil::{Answer, Stub, TEST_KEY};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn key() -> ApiKey {
    ApiKey::parse(TEST_KEY).unwrap()
}

fn client_for(stub: &Stub) -> AnthropicHttp {
    AnthropicHttp::new(&AnthropicConfig::new(&stub.url).unwrap(), &key()).unwrap()
}

const BIG: usize = 1024 * 1024;

#[tokio::test]
async fn a_request_carries_the_key_the_version_and_nothing_secret_elsewhere() {
    let stub = Stub::start(vec![Answer::json(200, "{}")]).await;
    let reply = client_for(&stub)
        .get("/v1/models", &[("limit", "1000")], BIG)
        .await
        .unwrap();
    assert_eq!(reply.status, 200);

    let seen = &stub.requests()[0];
    assert_eq!(seen.method, "GET");
    assert_eq!(seen.path, "/v1/models");
    assert_eq!(seen.query.as_deref(), Some("limit=1000"));
    assert_eq!(
        seen.headers["authorization"],
        format!("Bearer {TEST_KEY}").as_str()
    );
    assert_eq!(seen.headers["anthropic-version"], ANTHROPIC_VERSION);
    assert_eq!(seen.headers["accept"], "application/json");
    assert!(seen.headers["user-agent"]
        .to_str()
        .unwrap()
        .starts_with("clearcut/"));
    assert!(!seen.headers.contains_key("x-api-key"));
    assert!(!seen.headers.contains_key("anthropic-workspace-id"));
    // The key is in one header and nowhere else.
    assert!(!seen.path.contains("sk-ant") && !seen.query.clone().unwrap_or_default().contains("sk-ant"));
}

#[tokio::test]
async fn a_workspace_is_named_only_when_configured() {
    let stub = Stub::start(vec![Answer::json(200, "{}")]).await;
    let config = AnthropicConfig::new(&stub.url)
        .unwrap()
        .with_workspace_id("wrkspc_011CZkZaBF1tNoB5wlCeusgy")
        .unwrap();
    let http = AnthropicHttp::new(&config, &key()).unwrap();
    http.get("/v1/models", &[], BIG).await.unwrap();
    assert_eq!(
        stub.requests()[0].headers["anthropic-workspace-id"],
        "wrkspc_011CZkZaBF1tNoB5wlCeusgy"
    );
}

#[tokio::test]
async fn query_values_are_encoded_not_pasted_in() {
    let stub = Stub::start(vec![Answer::json(200, "{}")]).await;
    client_for(&stub)
        .get("/v1/models", &[("after_id", "a b&c=d")], BIG)
        .await
        .unwrap();
    assert_eq!(stub.requests()[0].query.as_deref(), Some("after_id=a+b%26c%3Dd"));
}

#[test]
fn the_key_header_is_marked_sensitive_and_the_client_prints_no_secret() {
    let config = AnthropicConfig::production();
    let http = AnthropicHttp::new(&config, &key()).unwrap();
    assert!(http.authorization.is_sensitive());
    let shown = format!("{http:?}");
    for fragment in ["sk-ant", "TeStKeY", "ABCD", "Bearer"] {
        assert!(!shown.contains(fragment), "{fragment} in {shown}");
    }
    assert!(shown.contains("redacted"));
}

#[tokio::test]
async fn a_redirect_is_not_followed_so_the_key_cannot_be_handed_on() {
    let elsewhere = Stub::start(vec![Answer::json(200, "{}")]).await;
    let stub = Stub::start(vec![
        Answer::json(302, "").with_header("location", &format!("{}/steal", elsewhere.url))
    ])
    .await;
    let reply = client_for(&stub).get("/v1/models", &[], BIG).await.unwrap();
    assert_eq!(reply.status, 302);
    assert!(
        elsewhere.requests().is_empty(),
        "the redirect target must never be contacted"
    );
}

#[tokio::test]
async fn retry_after_is_read_only_when_it_is_a_whole_number_of_seconds() {
    for (header, expected) in [
        ("17", Some(17)),
        ("0", Some(0)),
        (" 5 ", Some(5)),
        ("Wed, 21 Oct 2026 07:28:00 GMT", None),
        ("-5", None),
        ("abc", None),
        ("1.5", None),
        ("", None),
    ] {
        let stub = Stub::start(vec![Answer::json(429, "{}").with_header("retry-after", header)]).await;
        let reply = client_for(&stub).get("/v1/models", &[], BIG).await.unwrap();
        assert_eq!(reply.retry_after, expected, "{header:?}");
    }
    let none = Stub::start(vec![Answer::json(429, "{}")]).await;
    assert_eq!(
        client_for(&none)
            .get("/v1/models", &[], BIG)
            .await
            .unwrap()
            .retry_after,
        None
    );
}

#[tokio::test]
async fn the_request_id_is_kept_only_if_it_looks_like_one() {
    for (header, expected) in [
        (
            "req_011CSHoEeqs5C35K2UUqR7Fy",
            Some("req_011CSHoEeqs5C35K2UUqR7Fy"),
        ),
        ("req_a-b_C", Some("req_a-b_C")),
        ("req_<script>", None),
        ("has space", None),
        ("", None),
    ] {
        let stub = Stub::start(vec![Answer::json(200, "{}").with_header("request-id", header)]).await;
        let reply = client_for(&stub).get("/v1/models", &[], BIG).await.unwrap();
        assert_eq!(reply.request_id.as_deref(), expected, "{header:?}");
    }
    let long = "r".repeat(101);
    let stub = Stub::start(vec![Answer::json(200, "{}").with_header("request-id", &long)]).await;
    assert_eq!(
        client_for(&stub)
            .get("/v1/models", &[], BIG)
            .await
            .unwrap()
            .request_id,
        None
    );
}

#[tokio::test]
async fn an_answer_is_read_only_up_to_the_limit() {
    let stub = Stub::start(vec![Answer::json(200, "0123456789")]).await;
    let http = client_for(&stub);
    assert_eq!(http.get("/x", &[], 10).await.unwrap().body.len(), 10);
    assert_eq!(
        http.get("/x", &[], 9).await.unwrap_err(),
        TransportError::TooLarge { max_bytes: 9 }
    );
    assert_eq!(
        http.get("/x", &[], 5).await.unwrap_err(),
        TransportError::TooLarge { max_bytes: 5 }
    );
}

#[tokio::test]
async fn a_slow_server_is_given_up_on() {
    let stub = Stub::start(vec![Answer::json(200, "{}").after(Duration::from_millis(600))]).await;
    let config = AnthropicConfig::new(&stub.url)
        .unwrap()
        .with_timeout(Duration::from_millis(100))
        .unwrap();
    let http = AnthropicHttp::new(&config, &key()).unwrap();
    let started = Instant::now();
    assert_eq!(
        http.get("/v1/models", &[], BIG).await.unwrap_err(),
        TransportError::Timeout
    );
    assert!(started.elapsed() < Duration::from_millis(500));
}

#[tokio::test]
async fn nobody_listening_is_unreachable() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let http = AnthropicHttp::new(&AnthropicConfig::new(&url).unwrap(), &key()).unwrap();
    assert_eq!(
        http.get("/v1/models", &[], BIG).await.unwrap_err(),
        TransportError::Unreachable
    );
}

#[tokio::test]
async fn a_connection_cut_in_the_middle_of_an_answer_is_interrupted() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buffer = [0u8; 2048];
        let _ = socket.read(&mut buffer).await;
        socket
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 1000\r\n\r\npartial")
            .await
            .unwrap();
        // ...and the connection is dropped here.
    });
    let http = AnthropicHttp::new(&AnthropicConfig::new(&url).unwrap(), &key()).unwrap();
    assert_eq!(
        http.get("/v1/models", &[], BIG).await.unwrap_err(),
        TransportError::Interrupted
    );
}

#[test]
fn transport_errors_are_plain_sentences() {
    for (error, words) in [
        (TransportError::Unreachable, "reach"),
        (TransportError::Timeout, "too long"),
        (TransportError::Interrupted, "cut"),
        (TransportError::TooLarge { max_bytes: 5 }, "5"),
        (TransportError::Client, "client"),
    ] {
        assert!(error.to_string().contains(words), "{error}");
    }
}
