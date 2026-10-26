// Written by Kone Tshivhinda & Claude
//! The HTTP rules for the tenant's key, against a fake (no database, no Anthropic). What the key
//! rules do is tested in `clearcut-keys`.

use super::*;
use crate::tests::{bearer, json as parse, send, FakeAuth, ADMIN_ONLY, READER, WRITER};
use crate::{finish, routes, AppState};
use axum::body::Body;
use axum::http::{header, Request};
use axum::Router;
use clearcut_keys::{KeyMode, KeySource};
use std::sync::{Arc, Mutex};

const SENTINEL: &str = "sk-ant-api03-SENTINEL-do-not-leak-0123456789abcdefghij";

#[derive(Default)]
struct Fake {
    /// What `set` answers; the texts it was handed are kept.
    answer: Mutex<Option<KeysError>>,
    received: Mutex<Vec<String>>,
}

impl KeyAdmin for Arc<Fake> {
    fn status<'a>(&'a self, _tenant: TenantId) -> KeysFuture<'a, KeyView> {
        Box::pin(async move {
            Ok(KeyView {
                configured: true,
                last4: Some("ghij".into()),
                set_at: Some(OffsetDateTime::UNIX_EPOCH),
                checked_at: None,
                check_ok: Some(true),
                source: KeySource::Tenant,
                tenant_keys_enabled: true,
                mode: KeyMode::TenantOnly,
            })
        })
    }

    fn set<'a>(&'a self, _by: &'a Principal, text: &'a str) -> KeysFuture<'a, KeySet> {
        Box::pin(async move {
            self.received.lock().unwrap().push(text.to_string());
            match self.answer.lock().unwrap().take() {
                Some(error) => Err(error),
                None => Ok(KeySet {
                    last4: "ghij".into(),
                    models: vec!["claude-haiku-5-5".into()],
                    checked_at: OffsetDateTime::UNIX_EPOCH,
                }),
            }
        })
    }

    fn remove<'a>(&'a self, _by: &'a Principal) -> KeysFuture<'a, bool> {
        Box::pin(async move { Ok(true) })
    }
}

fn app(fake: &Arc<Fake>) -> Router {
    finish(
        routes(),
        AppState::shared(FakeAuth::new()).with_keys(fake.clone()),
    )
}

fn request(method: &str, token: Option<&str>, body: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri("/v1/anthropic-key");
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, bearer(token));
    }
    if body.is_some() {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
    }
    builder.body(Body::from(body.unwrap_or("").to_string())).unwrap()
}

fn put(token: Option<&str>, body: &str) -> Request<Body> {
    request("PUT", token, Some(body))
}

#[tokio::test]
async fn every_call_needs_keys_admin_and_nothing_reaches_the_key_code_without_it() {
    let fake = Arc::new(Fake::default());
    let app = app(&fake);
    let body = format!("{{\"key\":\"{SENTINEL}\"}}");
    for method in ["GET", "PUT", "DELETE"] {
        let with_body = (method == "PUT").then_some(body.as_str());
        assert_eq!(
            send(&app, request(method, None, with_body)).await.0,
            StatusCode::UNAUTHORIZED
        );
        for token in [READER, WRITER] {
            assert_eq!(
                send(&app, request(method, Some(token), with_body)).await.0,
                StatusCode::FORBIDDEN,
                "{method} {token}"
            );
        }
    }
    assert!(fake.received.lock().unwrap().is_empty());
}

#[tokio::test]
async fn setting_a_key_answers_what_it_can_do_and_the_key_is_in_no_response() {
    let fake = Arc::new(Fake::default());
    let app = app(&fake);
    let body = format!("{{\"key\":\"{SENTINEL}\"}}");

    let (status, headers, text) = send(&app, put(Some(ADMIN_ONLY), &body)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(parse(&text)["last4"], "ghij");
    assert_eq!(parse(&text)["models"][0], "claude-haiku-5-5");
    assert_eq!(fake.received.lock().unwrap()[0], SENTINEL);

    let (status, _, text) = send(&app, request("GET", Some(ADMIN_ONLY), None)).await;
    assert_eq!(status, StatusCode::OK);
    let view = parse(&text);
    assert_eq!(
        (
            view["configured"].as_bool(),
            view["source"].as_str(),
            view["mode"].as_str()
        ),
        (Some(true), Some("tenant"), Some("tenant-only"))
    );

    let (status, _, removed) = send(&app, request("DELETE", Some(ADMIN_ONLY), None)).await;
    assert_eq!(
        (status, parse(&removed)["removed"].as_bool()),
        (StatusCode::OK, Some(true))
    );

    // The key is not in anything that came back, headers included.
    assert!(!text.contains("SENTINEL") && !removed.contains("SENTINEL"));
    assert!(headers
        .values()
        .all(|value| !value.to_str().unwrap_or("").contains("SENTINEL")));
}

#[tokio::test]
async fn a_key_that_cannot_be_used_is_answered_in_fixed_sentences_that_repeat_none_of_it() {
    let fake = Arc::new(Fake::default());
    let app = app(&fake);
    let body = format!("{{\"key\":\"{SENTINEL}\"}}");
    for (error, expected) in [
        (
            KeysError::BadKey(clearcut_anthropic::KeyError::AdminKey),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            KeysError::Refused("the key was rejected".into()),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            KeysError::CheckUnavailable("could not reach Anthropic".into()),
            StatusCode::BAD_GATEWAY,
        ),
        (KeysError::NotConfigured, StatusCode::SERVICE_UNAVAILABLE),
    ] {
        *fake.answer.lock().unwrap() = Some(error);
        let (status, _, text) = send(&app, put(Some(ADMIN_ONLY), &body)).await;
        assert_eq!(status, expected);
        assert!(!text.contains("SENTINEL"), "{text}");
    }

    // A body that is not a key object is refused before the key code, and says nothing of what was sent.
    let before = fake.received.lock().unwrap().len();
    for bad in [
        "not json",
        "{}",
        "{\"key\": 5}",
        &format!("{{\"key\":\"{SENTINEL}\",\"extra\":1}}"),
        &format!("[\"{SENTINEL}\"]"),
    ] {
        let (status, _, text) = send(&app, put(Some(ADMIN_ONLY), bad)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
        assert!(!text.contains("SENTINEL"), "{text}");
    }
    assert_eq!(fake.received.lock().unwrap().len(), before);
}
