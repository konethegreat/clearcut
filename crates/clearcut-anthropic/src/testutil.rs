// Written by Kone Tshivhinda & Claude
//! A stub Anthropic server for tests: a tiny local server that answers as told,
//! so every status, header, limit and failure can be produced on demand without
//! a key, a network or any money.

use axum::extract::State;
use axum::http::{HeaderMap, HeaderName, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::Router;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpListener;

/// A realistic-looking key for tests (made up; not issued to anyone).
pub(crate) const TEST_KEY: &str =
    "sk-ant-api03-TeStKeYnOtReAl0123456789_-TeStKeYnOtReAl0123456789_-TeStKeYnOtReAl0123456789_-ABCD";

/// One request the stub received.
#[derive(Debug, Clone)]
pub(crate) struct Seen {
    pub method: Method,
    pub path: String,
    pub query: Option<String>,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

/// One scripted answer.
#[derive(Debug, Clone)]
pub(crate) struct Answer {
    pub status: u16,
    pub headers: Vec<(&'static str, String)>,
    pub body: Vec<u8>,
    pub delay: Duration,
}

impl Answer {
    pub fn json(status: u16, body: &str) -> Answer {
        Answer {
            status,
            headers: vec![("content-type", "application/json".to_string())],
            body: body.as_bytes().to_vec(),
            delay: Duration::ZERO,
        }
    }

    pub fn with_header(mut self, name: &'static str, value: &str) -> Answer {
        self.headers.push((name, value.to_string()));
        self
    }

    pub fn after(mut self, delay: Duration) -> Answer {
        self.delay = delay;
        self
    }
}

#[derive(Clone)]
struct Script {
    answers: Arc<Vec<Answer>>,
    next: Arc<AtomicUsize>,
    seen: Arc<Mutex<Vec<Seen>>>,
}

pub(crate) struct Stub {
    /// `http://127.0.0.1:port`
    pub url: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Stub {
    /// Starts a server. The nth request gets the nth answer; the last answer repeats.
    pub async fn start(answers: Vec<Answer>) -> Stub {
        assert!(!answers.is_empty());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let script = Script {
            answers: Arc::new(answers),
            next: Arc::new(AtomicUsize::new(0)),
            seen: seen.clone(),
        };
        let app = Router::new().fallback(respond).with_state(script);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Stub { url, seen }
    }

    pub fn requests(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

async fn respond(
    State(script): State<Script>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    script.seen.lock().unwrap().push(Seen {
        method,
        path: uri.path().to_string(),
        query: uri.query().map(str::to_string),
        headers,
        body: body.to_vec(),
    });
    let index = script
        .next
        .fetch_add(1, Ordering::SeqCst)
        .min(script.answers.len() - 1);
    let answer = script.answers[index].clone();
    tokio::time::sleep(answer.delay).await;
    let mut response = (StatusCode::from_u16(answer.status).unwrap(), answer.body).into_response();
    for (name, value) in answer.headers {
        response
            .headers_mut()
            .insert(HeaderName::from_static(name), value.parse().unwrap());
    }
    response
}
