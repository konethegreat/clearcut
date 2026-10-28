// Written by Kone Tshivhinda & Claude
//! The calls to the ClearCut REST API. Every rule (scopes, tenants, limits) lives in the server;
//! this only sends the request with the key and turns an answer into something an agent can use.

use crate::config::{Config, SecretKey};
use reqwest::redirect::Policy;
use reqwest::{Client, Method};
use serde_json::Value;
use std::time::Duration;

/// The longest any answer may be (an export can be big; nothing here should be bigger).
const MAX_ANSWER_BYTES: usize = 64 * 1024 * 1024;

pub struct Api {
    base: String,
    key: SecretKey,
    http: Client,
}

/// What the server answered.
pub struct Reply {
    pub status: u16,
    pub content_type: String,
    /// How long the server asked the caller to wait (`Retry-After`), in whole seconds.
    pub retry_after: Option<u64>,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
}

impl Api {
    /// `None` when no key is configured.
    pub fn new(config: &Config) -> Option<Api> {
        let key = config.api_key.clone()?;
        // No redirects (the key would follow them), no proxy from the environment, a bounded wait.
        let http = Client::builder()
            .redirect(Policy::none())
            .no_proxy()
            .timeout(Duration::from_secs(120))
            .user_agent(concat!("clearcut-mcp/", env!("CARGO_PKG_VERSION")))
            .build()
            .ok()?;
        Some(Api {
            base: config.base_url.clone(),
            key,
            http,
        })
    }

    pub async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<(Vec<u8>, &str)>,
    ) -> Result<Reply, String> {
        let mut request = self
            .http
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(self.key.expose());
        if let Some((bytes, content_type)) = body {
            request = request.header("content-type", content_type).body(bytes);
        }
        let response = request.send().await.map_err(|_| {
            "could not reach the ClearCut server; is it running, and is CLEARCUT_URL right?".to_string()
        })?;
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse().ok());
        let bytes = response
            .bytes()
            .await
            .map_err(|_| "the ClearCut server's answer was cut off".to_string())?;
        if bytes.len() > MAX_ANSWER_BYTES {
            return Err("the ClearCut server's answer is too large".into());
        }
        Ok(Reply {
            status,
            content_type,
            retry_after,
            body: bytes.to_vec(),
        })
    }

    /// A call that answers JSON; a refusal becomes a readable error.
    pub async fn json(
        &self,
        method: Method,
        path: &str,
        body: Option<(Vec<u8>, &str)>,
    ) -> Result<Value, String> {
        let reply = self.send(method, path, body).await?;
        if reply.ok() {
            Ok(reply.json())
        } else {
            Err(explain(&reply))
        }
    }
}

/// A server refusal in plain words. The key is never part of it.
pub fn explain(reply: &Reply) -> String {
    let body = reply.json();
    let said = body.get("error").and_then(Value::as_str).unwrap_or("");
    match reply.status {
        401 => "ClearCut did not accept the API key: check CLEARCUT_API_KEY".into(),
        403 => format!(
            "this ClearCut key is not allowed to do that{}",
            body.get("required_scope")
                .and_then(Value::as_str)
                .map(|scope| format!(" (it needs the {scope} scope)"))
                .unwrap_or_default()
        ),
        404 => "not found (no such document, or it belongs to another account)".into(),
        // A page that cannot be confirmed yet: the places are what the agent needs to fix it.
        422 => match body.get("refusal") {
            Some(refusal) => format!("{said}: {refusal}"),
            None => said.to_string(),
        },
        status => {
            if said.is_empty() {
                format!("the ClearCut server answered {status}")
            } else {
                format!("{said} (status {status})")
            }
        }
    }
}
