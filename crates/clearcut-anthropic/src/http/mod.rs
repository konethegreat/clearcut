// Written by Kone Tshivhinda & Claude
//! A narrow HTTPS client for the Anthropic API, built the way `HttpWorker` is
//! (ADR-0009) because it carries a more valuable secret.
//!
//! What it will and will not do, each with a test:
//!
//! - The key travels in **one header**, marked sensitive, and nowhere else (never
//!   in the path or the query, never printed).
//! - It **never follows a redirect and never uses a proxy from the environment**,
//!   so the key can only go to the configured address.
//! - Over `https`, it will **not fall back to plain `http`**.
//! - It **limits what it reads back** and how long it waits.
//! - It reports **fixed facts only**: a status, a `retry-after` in whole seconds,
//!   and a sanitised request id. Nothing the server wrote is repeated, so a
//!   hostile or buggy server cannot put text into our errors or logs.

use crate::config::{AnthropicConfig, ConfigError, ANTHROPIC_VERSION};
use crate::key::ApiKey;
use bytes::{Bytes, BytesMut};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, ACCEPT, AUTHORIZATION, USER_AGENT};
use reqwest::redirect::Policy;
use reqwest::{Client, Url};
use std::fmt;
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_REQUEST_ID_LEN: usize = 100;

/// What came back, reduced to what the program may rely on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    /// Seconds to wait, from a `retry-after` header holding a whole number.
    pub retry_after: Option<u64>,
    /// Anthropic's `request-id`, if it looks like one (letters, digits, `_`, `-`).
    pub request_id: Option<String>,
    pub body: Bytes,
}

/// Why no answer could be read at all. Fixed sentences.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TransportError {
    #[error("Anthropic could not be reached")]
    Unreachable,
    #[error("Anthropic took too long to answer")]
    Timeout,
    #[error("the connection to Anthropic was cut while the answer was coming")]
    Interrupted,
    #[error("Anthropic's answer is larger than the {max_bytes} bytes that are read")]
    TooLarge { max_bytes: usize },
    #[error("the HTTP client failed")]
    Client,
}

/// Talks to the Claude API with one user's key.
#[derive(Clone)]
pub struct AnthropicHttp {
    client: Client,
    base: Url,
    authorization: HeaderValue,
    workspace: Option<HeaderValue>,
}

impl fmt::Debug for AnthropicHttp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnthropicHttp")
            .field("host", &self.base.host_str())
            .field("key", &"[redacted]")
            .finish()
    }
}

impl AnthropicHttp {
    pub fn new(config: &AnthropicConfig, key: &ApiKey) -> Result<AnthropicHttp, ConfigError> {
        let mut authorization =
            HeaderValue::from_str(&format!("Bearer {}", key.expose())).map_err(|_| ConfigError::Client)?;
        authorization.set_sensitive(true);
        let workspace = match config.workspace_id() {
            Some(id) => Some(HeaderValue::from_str(id).map_err(|_| ConfigError::Client)?),
            None => None,
        };
        let client = Client::builder()
            .redirect(Policy::none())
            .no_proxy()
            .https_only(config.base_url().scheme() == "https")
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(config.timeout())
            .build()
            .map_err(|_| ConfigError::Client)?;
        Ok(AnthropicHttp {
            client,
            base: config.base_url().clone(),
            authorization,
            workspace,
        })
    }

    /// A GET of `path` (such as `/v1/models`) with `query` encoded properly. At
    /// most `max_bytes` of the answer are read; more is an error.
    pub async fn get(
        &self,
        path: &str,
        query: &[(&str, &str)],
        max_bytes: usize,
    ) -> Result<Reply, TransportError> {
        let mut url = self.base.clone();
        url.set_path(path);
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        let mut request = self
            .client
            .get(url)
            .header(AUTHORIZATION, self.authorization.clone())
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header(ACCEPT, "application/json")
            .header(USER_AGENT, concat!("clearcut/", env!("CARGO_PKG_VERSION")));
        if let Some(workspace) = &self.workspace {
            request = request.header(
                HeaderName::from_static("anthropic-workspace-id"),
                workspace.clone(),
            );
        }
        let mut response = request.send().await.map_err(classify)?;

        let status = response.status().as_u16();
        let retry_after = retry_after(response.headers());
        let request_id = request_id(response.headers());
        let mut body = BytesMut::new();
        while let Some(chunk) = response.chunk().await.map_err(classify_body)? {
            if body.len() + chunk.len() > max_bytes {
                return Err(TransportError::TooLarge { max_bytes });
            }
            body.extend_from_slice(&chunk);
        }
        Ok(Reply {
            status,
            retry_after,
            request_id,
            body: body.freeze(),
        })
    }
}

fn classify(error: reqwest::Error) -> TransportError {
    if error.is_timeout() {
        TransportError::Timeout
    } else if error.is_connect() {
        TransportError::Unreachable
    } else if error.is_builder() {
        TransportError::Client
    } else {
        TransportError::Interrupted
    }
}

fn classify_body(error: reqwest::Error) -> TransportError {
    if error.is_timeout() {
        TransportError::Timeout
    } else {
        TransportError::Interrupted
    }
}

/// Seconds, if the header holds nothing but a whole number. (The HTTP layer has
/// already stripped any spaces around the value.)
fn retry_after(headers: &HeaderMap) -> Option<u64> {
    headers.get("retry-after")?.to_str().ok()?.parse().ok()
}

fn request_id(headers: &HeaderMap) -> Option<String> {
    let value = headers.get("request-id")?.to_str().ok()?;
    let plain = !value.is_empty()
        && value.len() <= MAX_REQUEST_ID_LEN
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    plain.then(|| value.to_string())
}

#[cfg(test)]
mod tests;
