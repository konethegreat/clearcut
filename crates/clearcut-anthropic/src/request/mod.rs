// Written by Kone Tshivhinda & Claude
//! Turning a [`ModelRequest`] into the JSON the Messages API takes (ADR-0012 §1).
//!
//! Nothing here sends anything: [`build`] is a pure function, so every row of
//! the ADR's mapping table can be tested with an exact JSON comparison and no
//! network. Things that can never be sent (tools, sampling settings, the
//! tenant or job ids, metadata) are never added: the body is built from a fixed
//! list of keys.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use clearcut_provider::{Block, CacheTtl, ModelRequest, ProviderError, Thinking};
use serde_json::{json, Map, Value};

use crate::schema::{self, Removal};

/// Without streaming, an answer may be at most this many tokens (ADR-0012 §5).
/// Anything bigger could run past the ten minutes a plain request may take.
pub const MAX_NON_STREAMING_TOKENS: u32 = 16_000;

/// The body to send, plus the rules the schema profile had to remove (the
/// caller can check that `clearcut-core` enforces them afterwards).
#[derive(Debug, Clone, PartialEq)]
pub struct Built {
    pub body: Value,
    pub removed: Vec<Removal>,
}

/// Builds the request body, or says why it cannot be sent. Every refusal is a
/// [`ProviderError::Rejected`] with a fixed reason: it names the model, the
/// limit or the schema path, and never any text from the document.
pub fn build(request: &ModelRequest) -> Result<Built, ProviderError> {
    if request.max_output_tokens() > MAX_NON_STREAMING_TOKENS {
        return Err(ProviderError::Rejected(format!(
            "max_output_tokens {} is above {MAX_NON_STREAMING_TOKENS}; answers that long need streaming, which is not supported yet",
            request.max_output_tokens()
        )));
    }

    let thinking = match request.thinking() {
        Thinking::Adaptive => None,
        Thinking::Off => Some(thinking_off(request.model()).ok_or_else(|| {
            ProviderError::Rejected(format!(
                "thinking cannot be turned off for the model `{}`",
                request.model()
            ))
        })?),
    };

    let profile = schema::profile(request.output_schema())
        .map_err(|error| ProviderError::Rejected(format!("the output schema cannot be sent: {error}")))?;

    let cache = cache_control(request.cache_ttl());

    let mut body = Map::new();
    body.insert("model".into(), json!(request.model()));
    body.insert("max_tokens".into(), json!(request.max_output_tokens()));
    body.insert("stream".into(), json!(false));

    if !request.system().is_empty() {
        let system: Vec<Value> = request
            .system()
            .iter()
            .map(|block| text_block(block.text(), block.is_cache_breakpoint(), &cache))
            .collect();
        body.insert("system".into(), Value::Array(system));
    }

    let content: Vec<Value> = request
        .content()
        .iter()
        .map(|block| match block {
            Block::Text(text) => text_block(text.text(), text.is_cache_breakpoint(), &cache),
            Block::Image { media, bytes } => json!({
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": media.media_type(),
                    "data": STANDARD.encode(bytes),
                }
            }),
        })
        .collect();
    body.insert("messages".into(), json!([{ "role": "user", "content": content }]));

    if let Some(thinking) = thinking {
        body.insert("thinking".into(), thinking);
    }
    body.insert(
        "output_config".into(),
        json!({
            "effort": request.effort().as_str(),
            "format": { "type": "json_schema", "schema": profile.schema },
        }),
    );

    Ok(Built {
        body: Value::Object(body),
        removed: profile.removed,
    })
}

fn cache_control(ttl: CacheTtl) -> Value {
    match ttl {
        CacheTtl::FiveMinutes => json!({ "type": "ephemeral" }),
        CacheTtl::OneHour => json!({ "type": "ephemeral", "ttl": ttl.as_str() }),
    }
}

fn text_block(text: &str, breakpoint: bool, cache: &Value) -> Value {
    let mut block = json!({ "type": "text", "text": text });
    if breakpoint {
        block["cache_control"] = cache.clone();
    }
    block
}

/// What `Thinking::Off` means for each model family (research 06): Haiku can
/// switch thinking off, Sonnet can only keep it to the gaps between tool calls,
/// and Opus and Fable always think. A model we do not know is treated as
/// "adaptive only", so `Off` is refused rather than guessed.
fn thinking_off(model: &str) -> Option<Value> {
    if in_family(model, "claude-haiku-5") {
        Some(json!({ "type": "disabled" }))
    } else if in_family(model, "claude-sonnet-5") {
        Some(json!({ "type": "between_tools" }))
    } else {
        None
    }
}

/// `claude-haiku-5` matches `claude-haiku-5`, `claude-haiku-5-5` and
/// `claude-haiku-5-5-20260301`, but not `claude-haiku-50`.
fn in_family(model: &str, family: &str) -> bool {
    model
        .strip_prefix(family)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('-'))
}

#[cfg(test)]
mod tests;
