// Written by Kone Tshivhinda & Claude
//! The HTTP client for the Python document worker (ADR-0009).
//!
//! What it will and will not do, each with a test:
//!
//! - **Plain `http://` only, no credentials in the address.** The worker lives on
//!   a private network next to this service; the bearer token travels in a header
//!   on that network. (If the worker is ever reached across an untrusted network,
//!   TLS is added here, deliberately, not by accident.)
//! - **Never follows a redirect and never uses a proxy from the environment**, so
//!   the bearer token cannot be handed to anyone but the configured worker.
//! - **Limits what it reads back.** A worker's answer is a few hundred bytes; up
//!   to 64 KiB is read and no more, so a misbehaving worker cannot make this
//!   service buffer megabytes.
//! - **Maps every outcome onto a [`ProcessError`]** without repeating anything
//!   the server said: only fixed reasons and numeric limits.
//! - **Never prints the token or the address.**

use crate::{DocumentKind, DocumentProblem, DocumentProcessor, Inspection, ProcessError};
use bytes::Bytes;
use reqwest::header::{HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use reqwest::redirect::Policy;
use reqwest::{Client, Url};
use serde::Deserialize;
use serde_json::Value;
use std::fmt;
use std::time::Duration;

/// How long a call may take before the client gives up. The worker gives up on a
/// document after 30 s plus a 2 s grace, so the client waits longer than that.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(45);
/// The most of an answer that is read. (The real ones are a few hundred bytes.)
pub const MAX_RESPONSE_BYTES: usize = 64 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Why an [`HttpWorker`] could not be set up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    #[error("the worker address is not a valid URL")]
    BadUrl,
    #[error("the worker address must start with http://")]
    NotHttp,
    #[error("the worker address must not contain a user name or password")]
    HasCredentials,
    #[error("the worker address must not contain a query or fragment")]
    HasQuery,
    #[error("the worker token must be visible characters with no spaces")]
    BadToken,
    #[error("the timeout must be more than zero")]
    BadTimeout,
    #[error("the HTTP client could not be built")]
    Client,
}

/// Talks to the Python worker.
#[derive(Clone)]
pub struct HttpWorker {
    client: Client,
    endpoint: Url,
    authorization: HeaderValue,
    timeout: Duration,
}

impl fmt::Debug for HttpWorker {
    /// The address and the token are not shown.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpWorker")
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

impl HttpWorker {
    /// `base_url` is where the worker listens (`http://worker:8090`, optionally
    /// with a path prefix); `token` is the shared secret (`CLEARCUT_WORKER_TOKEN`).
    pub fn new(base_url: &str, token: &str) -> Result<Self, ConfigError> {
        let base = Url::parse(base_url).map_err(|_| ConfigError::BadUrl)?;
        if base.cannot_be_a_base() {
            return Err(ConfigError::BadUrl);
        }
        if base.scheme() != "http" {
            return Err(ConfigError::NotHttp);
        }
        if !base.username().is_empty() || base.password().is_some() {
            return Err(ConfigError::HasCredentials);
        }
        if base.query().is_some() || base.fragment().is_some() {
            return Err(ConfigError::HasQuery);
        }
        if token.is_empty() || !token.bytes().all(|byte| (0x21..=0x7e).contains(&byte)) {
            return Err(ConfigError::BadToken);
        }

        let mut endpoint = base.clone();
        endpoint.set_path(&format!("{}/v1/inspect", base.path().trim_end_matches('/')));
        let mut authorization =
            HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| ConfigError::BadToken)?;
        // Marked sensitive, so it is redacted if a request is ever debug-printed.
        authorization.set_sensitive(true);

        let client = Client::builder()
            .user_agent(concat!("clearcut-docproc/", env!("CARGO_PKG_VERSION")))
            .redirect(Policy::none())
            .no_proxy()
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .map_err(|_| ConfigError::Client)?;
        Ok(HttpWorker {
            client,
            endpoint,
            authorization,
            timeout: DEFAULT_TIMEOUT,
        })
    }

    /// Changes how long a call may take.
    pub fn with_timeout(mut self, timeout: Duration) -> Result<Self, ConfigError> {
        if timeout.is_zero() {
            return Err(ConfigError::BadTimeout);
        }
        self.timeout = timeout;
        Ok(self)
    }
}

impl DocumentProcessor for HttpWorker {
    async fn inspect(&self, document: Bytes) -> Result<Inspection, ProcessError> {
        let response = self
            .client
            .post(self.endpoint.clone())
            .header(AUTHORIZATION, self.authorization.clone())
            .header(CONTENT_TYPE, "application/octet-stream")
            .body(document)
            .timeout(self.timeout)
            .send()
            .await
            .map_err(classify)?;
        let status = response.status().as_u16();
        let (body, complete) = read_limited(response).await?;
        if status == 200 {
            if !complete {
                return Err(ProcessError::BadResponse("the answer was too large"));
            }
            return parse_inspection(&body);
        }
        Err(map_status(status, &body))
    }
}

/// A failure to send or to read: said in terms of what the caller should do.
fn classify(error: reqwest::Error) -> ProcessError {
    if error.is_timeout() {
        ProcessError::Timeout
    } else if error.is_connect() {
        ProcessError::Unavailable
    } else {
        ProcessError::Interrupted
    }
}

/// Reads at most [`MAX_RESPONSE_BYTES`] of the body. The flag says whether that
/// was all of it.
async fn read_limited(mut response: reqwest::Response) -> Result<(Vec<u8>, bool), ProcessError> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(classify)? {
        let room = MAX_RESPONSE_BYTES - body.len();
        if chunk.len() > room {
            body.extend_from_slice(&chunk[..room]);
            return Ok((body, false));
        }
        body.extend_from_slice(&chunk);
    }
    Ok((body, true))
}

/// The worker's success answer. Fields we do not know yet are ignored.
#[derive(Deserialize)]
struct Wire {
    media_type: String,
    pages: u64,
    pages_with_text: u64,
    has_text_layer: bool,
}

fn parse_inspection(body: &[u8]) -> Result<Inspection, ProcessError> {
    let wire: Wire =
        serde_json::from_slice(body).map_err(|_| ProcessError::BadResponse("not the expected shape"))?;
    let kind =
        DocumentKind::parse(&wire.media_type).ok_or(ProcessError::BadResponse("an unknown document type"))?;
    let count =
        |value: u64| u32::try_from(value).map_err(|_| ProcessError::BadResponse("a count out of range"));
    let inspection = Inspection::new(kind, count(wire.pages)?, count(wire.pages_with_text)?)
        .map_err(|_| ProcessError::BadResponse("inconsistent page counts"))?;
    if inspection.has_text_layer() != wire.has_text_layer {
        return Err(ProcessError::BadResponse("inconsistent text layer"));
    }
    Ok(inspection)
}

/// An error answer, understood by its status first and its message second. The
/// message is only ever compared with known texts, never repeated.
fn map_status(status: u16, body: &[u8]) -> ProcessError {
    let json: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let message = json.get("error").and_then(Value::as_str).unwrap_or("");
    let limit = |name: &str| json.get(name).and_then(Value::as_u64);
    match status {
        400 if message == "the file is empty" => ProcessError::Document(DocumentProblem::Empty),
        401 | 403 => ProcessError::Auth,
        413 => ProcessError::Document(DocumentProblem::TooLarge {
            max_bytes: limit("max_bytes").unwrap_or(0),
        }),
        415 => ProcessError::Document(DocumentProblem::UnsupportedType),
        422 => ProcessError::Document(match message {
            "the document is encrypted" => DocumentProblem::Encrypted,
            "too many pages" => match limit("max_pages").and_then(|n| u32::try_from(n).ok()) {
                Some(max_pages) if max_pages > 0 => DocumentProblem::TooManyPages { max_pages },
                _ => DocumentProblem::Unreadable,
            },
            _ => DocumentProblem::Unreadable,
        }),
        500..=599 => ProcessError::Server { status },
        _ => ProcessError::Unexpected { status },
    }
}

#[cfg(test)]
mod tests;
