// Written by Kone Tshivhinda & Claude
//! Checking a key without spending a token: `GET /v1/models` (ADR-0011).
//!
//! A good key answers with the list of models it may call and what each can do
//! (structured outputs, image and PDF input, batch...), so ClearCut learns what
//! to use instead of guessing. A bad key is told apart from a key whose owner
//! has hit a limit, because the two need different actions:
//!
//! | answer | meaning | who must act |
//! |---|---|---|
//! | 401 | the key is wrong, revoked or expired | the owner |
//! | 402 | a billing problem | the owner |
//! | 403 | the key may not use this | the owner |
//! | 429 + `enforced_spend_limit_reached` | the monthly spend cap | the owner (or wait for the 1st) |
//! | 429 otherwise | a rate limit; `retry-after` says how long | nobody: wait |
//! | 5xx, 529, timeouts | trouble on the way | nobody: try again |
//!
//! Nothing the server wrote is repeated in an error (only the sanitised request
//! id, so the owner can quote it to Anthropic).

use crate::http::{AnthropicHttp, TransportError};
use crate::key::ApiKey;
use serde::Deserialize;
use std::collections::HashSet;
use std::fmt;

/// The most of one page of models that is read.
pub const MAX_PAGE_BYTES: usize = 4 * 1024 * 1024;
/// The most pages followed (at 1 000 models a page this is far more than exists).
pub const MAX_PAGES: usize = 5;
const PAGE_SIZE: &str = "1000";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifecycle {
    Active,
    Deprecated,
    Retired,
    /// A value this program does not know (treated as not usable).
    Unknown,
}

/// One model a key may name, with the capabilities ClearCut cares about. A
/// capability that is not stated counts as **not supported**.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelInfo {
    pub id: String,
    pub display_name: String,
    pub lifecycle: Lifecycle,
    pub max_input_tokens: Option<u64>,
    pub max_tokens: Option<u64>,
    pub structured_outputs: bool,
    pub image_input: bool,
    pub pdf_input: bool,
    pub batch: bool,
}

/// What a good key can do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyCheck {
    pub models: Vec<ModelInfo>,
    pub request_id: Option<String>,
}

impl KeyCheck {
    pub fn model(&self, id: &str) -> Option<&ModelInfo> {
        self.models.iter().find(|model| model.id == id)
    }

    /// Whether `id` can be called: it exists for this key and is not retired
    /// (or of an unknown stage).
    pub fn can_use(&self, id: &str) -> bool {
        self.model(id)
            .is_some_and(|model| matches!(model.lifecycle, Lifecycle::Active | Lifecycle::Deprecated))
    }
}

/// What went wrong, in the terms of who has to do something about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyCheckKind {
    Rejected,
    NoPermission,
    BillingProblem,
    RateLimited { retry_after_secs: Option<u64> },
    SpendCapReached,
    Unavailable { status: u16 },
    Unexpected { status: u16 },
    Unreachable,
    Timeout,
    Interrupted,
    BadResponse,
    TooManyPages,
}

impl KeyCheckKind {
    /// The owner of the key has to fix something (the key, the billing, the cap);
    /// retrying will not help.
    pub fn owner_must_act(self) -> bool {
        matches!(
            self,
            KeyCheckKind::Rejected
                | KeyCheckKind::NoPermission
                | KeyCheckKind::BillingProblem
                | KeyCheckKind::SpendCapReached
        )
    }
}

impl fmt::Display for KeyCheckKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeyCheckKind::Rejected => f.write_str("Anthropic rejected the API key (it may be wrong, revoked or expired)"),
            KeyCheckKind::NoPermission => f.write_str("the API key does not have permission for this"),
            KeyCheckKind::BillingProblem => f.write_str("Anthropic reports a billing problem on the account behind this key"),
            KeyCheckKind::RateLimited { retry_after_secs: Some(seconds) } => {
                write!(f, "Anthropic is rate limiting this key; try again in {seconds} seconds")
            }
            KeyCheckKind::RateLimited { retry_after_secs: None } => {
                f.write_str("Anthropic is rate limiting this key; try again shortly")
            }
            KeyCheckKind::SpendCapReached => f.write_str(
                "the monthly spend cap of this Anthropic account has been reached; it resets at 00:00 UTC on the 1st",
            ),
            KeyCheckKind::Unavailable { status } => write!(f, "Anthropic is having trouble (status {status}); try again"),
            KeyCheckKind::Unexpected { status } => write!(f, "Anthropic answered with an unexpected status ({status})"),
            KeyCheckKind::Unreachable => f.write_str("Anthropic could not be reached"),
            KeyCheckKind::Timeout => f.write_str("Anthropic took too long to answer"),
            KeyCheckKind::Interrupted => f.write_str("the connection to Anthropic was cut"),
            KeyCheckKind::BadResponse => f.write_str("Anthropic's answer could not be understood"),
            KeyCheckKind::TooManyPages => f.write_str("Anthropic's model list had more pages than expected"),
        }
    }
}

/// A failed check: the kind, and Anthropic's request id when there is one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyCheckError {
    pub kind: KeyCheckKind,
    pub request_id: Option<String>,
}

impl KeyCheckError {
    pub fn kind(&self) -> KeyCheckKind {
        self.kind
    }

    pub fn request_id(&self) -> Option<&str> {
        self.request_id.as_deref()
    }
}

impl fmt::Display for KeyCheckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.kind)?;
        if let Some(id) = &self.request_id {
            write!(f, " (request {id})")?;
        }
        Ok(())
    }
}

impl std::error::Error for KeyCheckError {}

fn fail(kind: KeyCheckKind, request_id: Option<String>) -> KeyCheckError {
    KeyCheckError { kind, request_id }
}

/// Asks Anthropic which models this key can use.
pub async fn check_key(http: &AnthropicHttp) -> Result<KeyCheck, KeyCheckError> {
    let mut models = Vec::new();
    let mut first_request_id = None;
    let mut cursor: Option<String> = None;
    let mut seen_cursors = HashSet::new();

    for _ in 0..MAX_PAGES {
        let mut query = vec![("limit", PAGE_SIZE)];
        if let Some(after) = &cursor {
            query.push(("after_id", after.as_str()));
        }
        let reply = http
            .get("/v1/models", &query, MAX_PAGE_BYTES)
            .await
            .map_err(|error| {
                let kind = match error {
                    TransportError::Unreachable => KeyCheckKind::Unreachable,
                    TransportError::Timeout => KeyCheckKind::Timeout,
                    TransportError::Interrupted => KeyCheckKind::Interrupted,
                    TransportError::TooLarge { .. } | TransportError::Client => KeyCheckKind::BadResponse,
                };
                fail(kind, None)
            })?;
        let id = reply.request_id.clone();
        first_request_id = first_request_id.or_else(|| id.clone());

        match reply.status {
            200 => {}
            401 => return Err(fail(KeyCheckKind::Rejected, id)),
            402 => return Err(fail(KeyCheckKind::BillingProblem, id)),
            403 => return Err(fail(KeyCheckKind::NoPermission, id)),
            429 => {
                let kind = if is_spend_cap(&reply.body) {
                    KeyCheckKind::SpendCapReached
                } else {
                    KeyCheckKind::RateLimited {
                        retry_after_secs: reply.retry_after,
                    }
                };
                return Err(fail(kind, id));
            }
            500..=599 => return Err(fail(KeyCheckKind::Unavailable { status: reply.status }, id)),
            status => return Err(fail(KeyCheckKind::Unexpected { status }, id)),
        }

        let page: Page =
            serde_json::from_slice(&reply.body).map_err(|_| fail(KeyCheckKind::BadResponse, id.clone()))?;
        models.extend(page.data.into_iter().map(ModelInfo::from));
        if !page.has_more.unwrap_or(false) {
            return Ok(KeyCheck {
                models,
                request_id: first_request_id,
            });
        }
        // More to fetch: the cursor must be a plain id that moves forward.
        let next = page.last_id.filter(|id| is_plain_id(id));
        match next {
            Some(next) if seen_cursors.insert(next.clone()) => cursor = Some(next),
            _ => return Err(fail(KeyCheckKind::BadResponse, id)),
        }
    }
    Err(fail(KeyCheckKind::TooManyPages, first_request_id))
}

/// A cursor or model id: 1 to 128 of letters, digits, dot, dash, underscore.
fn is_plain_id(id: &str) -> bool {
    (1..=128).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

/// The monthly-cap 429 has a body of the form
/// `{"error":{"details":{"error_code":"enforced_spend_limit_reached"}}}`.
fn is_spend_cap(body: &[u8]) -> bool {
    #[derive(Deserialize)]
    struct Body {
        error: Option<Error>,
    }
    #[derive(Deserialize)]
    struct Error {
        details: Option<Details>,
    }
    #[derive(Deserialize)]
    struct Details {
        error_code: Option<String>,
    }
    serde_json::from_slice::<Body>(body)
        .ok()
        .and_then(|body| body.error?.details?.error_code)
        .is_some_and(|code| code == "enforced_spend_limit_reached")
}

// ---- the wire format (only what we use; everything else is ignored) ----

#[derive(Deserialize)]
struct Page {
    data: Vec<WireModel>,
    has_more: Option<bool>,
    last_id: Option<String>,
}

#[derive(Deserialize)]
struct WireModel {
    id: String,
    display_name: Option<String>,
    lifecycle: Option<String>,
    max_input_tokens: Option<u64>,
    max_tokens: Option<u64>,
    capabilities: Option<WireCapabilities>,
}

#[derive(Deserialize, Default)]
struct WireCapabilities {
    batch: Option<Supported>,
    image_input: Option<Supported>,
    pdf_input: Option<Supported>,
    structured_outputs: Option<Supported>,
}

#[derive(Deserialize, Default)]
struct Supported {
    supported: Option<bool>,
}

fn yes(capability: Option<Supported>) -> bool {
    capability.and_then(|c| c.supported).unwrap_or(false)
}

impl From<WireModel> for ModelInfo {
    fn from(wire: WireModel) -> ModelInfo {
        let capabilities = wire.capabilities.unwrap_or_default();
        ModelInfo {
            display_name: wire.display_name.unwrap_or_else(|| wire.id.clone()),
            id: wire.id,
            lifecycle: match wire.lifecycle.as_deref() {
                Some("active") => Lifecycle::Active,
                Some("deprecated") => Lifecycle::Deprecated,
                Some("retired") => Lifecycle::Retired,
                _ => Lifecycle::Unknown,
            },
            max_input_tokens: wire.max_input_tokens,
            max_tokens: wire.max_tokens,
            structured_outputs: yes(capabilities.structured_outputs),
            image_input: yes(capabilities.image_input),
            pdf_input: yes(capabilities.pdf_input),
            batch: yes(capabilities.batch),
        }
    }
}

/// Convenience for callers that hold a key and a configuration but not yet a client.
pub async fn check_key_with(
    config: &crate::config::AnthropicConfig,
    key: &ApiKey,
) -> Result<KeyCheck, KeyCheckError> {
    let http = AnthropicHttp::new(config, key).map_err(|_| fail(KeyCheckKind::BadResponse, None))?;
    check_key(&http).await
}

#[cfg(test)]
mod tests;
