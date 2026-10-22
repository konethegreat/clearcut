// Written by Kone Tshivhinda & Claude
//! The client must ignore proxy settings in the environment, so the user's API
//! key can only ever go to the address it was configured for.
//!
//! Environment variables belong to the whole process, so this lives in a test
//! file of its own (one test, no neighbours running in parallel).

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::any;
use axum::Router;
use clearcut_anthropic::{check_key_with, AnthropicConfig, ApiKey};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::net::TcpListener;

const KEY: &str =
    "sk-ant-api03-TeStKeYnOtReAl0123456789_-TeStKeYnOtReAl0123456789_-TeStKeYnOtReAl0123456789_-ABCD";

async fn serve(status: StatusCode, body: &'static str) -> (String, Arc<AtomicUsize>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .fallback(any(move |State(hits): State<Arc<AtomicUsize>>| async move {
            hits.fetch_add(1, Ordering::SeqCst);
            (status, body)
        }))
        .with_state(hits.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, hits)
}

#[tokio::test]
async fn proxy_settings_in_the_environment_are_ignored() {
    let (proxy_url, proxy_hits) = serve(StatusCode::OK, "I am a proxy and I saw your key").await;
    let (api_url, api_hits) = serve(StatusCode::OK, r#"{"data":[],"has_more":false}"#).await;

    for name in [
        "HTTP_PROXY",
        "http_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
    ] {
        std::env::set_var(name, &proxy_url);
    }
    std::env::remove_var("NO_PROXY");
    std::env::remove_var("no_proxy");

    let config = AnthropicConfig::new(&api_url).unwrap();
    check_key_with(&config, &ApiKey::parse(KEY).unwrap())
        .await
        .unwrap();

    assert_eq!(
        api_hits.load(Ordering::SeqCst),
        1,
        "the request must go to the configured address"
    );
    assert_eq!(
        proxy_hits.load(Ordering::SeqCst),
        0,
        "and never through the environment's proxy"
    );
}
