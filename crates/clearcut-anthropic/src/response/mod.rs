// Written by Kone Tshivhinda & Claude
//! Reading a successful Messages API answer (ADR-0012 §2).
//!
//! The answer is **untrusted**: it came over the network from a model that has
//! read an uploaded document. So [`parse`] trusts nothing: every count must be a
//! plausible whole number, there must be exactly one text block holding a JSON
//! object, and every failure carries a **fixed** reason (never a piece of the
//! answer, never a parser message that might quote it). Thinking is dropped
//! without being looked at.
//!
//! A call that reached the model is billed even when its answer is unusable, so
//! every failure after the usage is read carries that usage.

use clearcut_provider::{ModelResponse, ProviderError, Usage};
use serde_json::{Map, Value};

/// A single count above this is not believable (the largest context is a few million tokens).
const MAX_PLAUSIBLE_TOKENS: u64 = 100_000_000;
const MAX_MODEL_NAME_LEN: usize = 100;

/// Reads the body of a `200` answer. `requested_model` is used when the answer
/// does not name a usable model; `request_id` is the (already sanitised) header.
pub fn parse(
    body: &[u8],
    requested_model: &str,
    request_id: Option<String>,
) -> Result<ModelResponse, ProviderError> {
    let Ok(Value::Object(message)) = serde_json::from_slice::<Value>(body) else {
        return Err(invalid(Usage::default(), "the answer was not a JSON message"));
    };

    let usage = read_usage(&message).ok_or_else(|| {
        invalid(
            Usage::default(),
            "the usage figures in the answer were not plausible",
        )
    })?;

    match message.get("stop_reason").and_then(Value::as_str) {
        Some("end_turn") => {}
        Some("max_tokens") => return Err(ProviderError::Truncated { usage }),
        Some("refusal") => return Err(ProviderError::Refused { usage }),
        Some("model_context_window_exceeded") => {
            return Err(invalid(
                usage,
                "the request and the answer did not fit in the model's context window",
            ))
        }
        Some("pause_turn") => return Err(invalid(usage, "the model paused its turn")),
        Some("tool_use") => return Err(invalid(usage, "the model asked to use a tool")),
        Some("stop_sequence") => return Err(invalid(usage, "the model stopped at a stop sequence")),
        _ => return Err(invalid(usage, "the model stopped for an unknown reason")),
    }

    let output = read_output(&message).map_err(|reason| invalid(usage, reason))?;

    let model = message
        .get("model")
        .and_then(Value::as_str)
        .filter(|name| usable_model_name(name))
        .unwrap_or(requested_model)
        .to_string();

    Ok(ModelResponse {
        output,
        usage,
        model,
        request_id,
    })
}

fn invalid(usage: Usage, reason: &str) -> ProviderError {
    ProviderError::InvalidOutput {
        usage,
        reason: reason.to_string(),
    }
}

/// The one text block, parsed as a JSON object. Thinking blocks are skipped.
fn read_output(message: &Map<String, Value>) -> Result<Value, &'static str> {
    let Some(Value::Array(blocks)) = message.get("content") else {
        return Err("the answer had no list of content blocks");
    };
    let mut text_blocks = blocks.iter().filter(|block| {
        !matches!(
            block.get("type").and_then(Value::as_str),
            Some("thinking" | "redacted_thinking")
        )
    });
    let (Some(block), None) = (text_blocks.next(), text_blocks.next()) else {
        return Err("the answer did not hold exactly one text block");
    };
    if block.get("type").and_then(Value::as_str) != Some("text") {
        return Err("the answer held a block that is not text");
    }
    let Some(text) = block.get("text").and_then(Value::as_str) else {
        return Err("the text block had no text");
    };
    match serde_json::from_str::<Value>(text) {
        Ok(output @ Value::Object(_)) => Ok(output),
        _ => Err("the answer text was not a JSON object"),
    }
}

/// `None` when any figure is present but not a plausible whole number. A figure
/// that is missing counts as zero.
fn read_usage(message: &Map<String, Value>) -> Option<Usage> {
    let Some(usage) = message.get("usage") else {
        return Some(Usage::default());
    };
    let usage = usage.as_object()?;

    let input_tokens = count(usage.get("input_tokens"))?;
    let output_tokens = count(usage.get("output_tokens"))?;
    let cache_read_tokens = count(usage.get("cache_read_input_tokens"))?;
    let stated_writes = count(usage.get("cache_creation_input_tokens"))?;

    let (five_minute, one_hour) = match usage.get("cache_creation") {
        None | Some(Value::Null) => (0, 0),
        Some(Value::Object(parts)) => (
            count(parts.get("ephemeral_5m_input_tokens"))?,
            count(parts.get("ephemeral_1h_input_tokens"))?,
        ),
        Some(_) => return None,
    };

    // The total is the figure to trust; the breakdown stands in when it is absent.
    // The one-hour part is a part of the writes, so it can never be larger.
    let cache_write_tokens = if stated_writes > 0 {
        stated_writes
    } else {
        five_minute.checked_add(one_hour)?
    };
    Some(Usage {
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_write_tokens,
        cache_write_1h_tokens: one_hour.min(cache_write_tokens),
    })
}

/// A missing figure is zero; a present one must be a whole number that is not absurd.
fn count(value: Option<&Value>) -> Option<u64> {
    match value {
        None | Some(Value::Null) => Some(0),
        Some(number) => number.as_u64().filter(|n| *n <= MAX_PLAUSIBLE_TOKENS),
    }
}

fn usable_model_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_MODEL_NAME_LEN
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
}

#[cfg(test)]
mod tests;
