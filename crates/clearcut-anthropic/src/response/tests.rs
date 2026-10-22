// Written by Kone Tshivhinda & Claude
use super::*;
use clearcut_core::jobs::FailureKind;
use clearcut_provider::{ProviderError, Usage};
use serde_json::{json, Value};

const MODEL: &str = "claude-haiku-5-5";

fn message(content: Value, stop_reason: Value, usage: Value) -> Vec<u8> {
    json!({
        "id": "msg_01",
        "type": "message",
        "role": "assistant",
        "model": MODEL,
        "content": content,
        "stop_reason": stop_reason,
        "usage": usage
    })
    .to_string()
    .into_bytes()
}

fn answer() -> Value {
    json!([
        { "type": "thinking", "thinking": "", "signature": "sig" },
        { "type": "text", "text": "{\"invoice_number\":\"INV-1\"}" }
    ])
}

fn plain_usage() -> Value {
    json!({ "input_tokens": 2095, "output_tokens": 503 })
}

fn ok(body: Vec<u8>) -> ModelResponse {
    parse(&body, MODEL, Some("req_1".into())).expect("should parse")
}

fn err(body: Vec<u8>) -> ProviderError {
    parse(&body, MODEL, Some("req_1".into())).expect_err("should fail")
}

// ---------- the answer ----------

#[test]
fn a_normal_answer_is_read() {
    let response = ok(message(
        answer(),
        json!("end_turn"),
        json!({
            "input_tokens": 2095, "output_tokens": 503,
            "cache_creation_input_tokens": 2051, "cache_read_input_tokens": 7,
            "cache_creation": { "ephemeral_5m_input_tokens": 2051, "ephemeral_1h_input_tokens": 0 }
        }),
    ));
    assert_eq!(response.output, json!({ "invoice_number": "INV-1" }));
    assert_eq!(response.model, MODEL);
    assert_eq!(response.request_id.as_deref(), Some("req_1"));
    assert_eq!(
        response.usage,
        Usage {
            input_tokens: 2095,
            output_tokens: 503,
            cache_read_tokens: 7,
            cache_write_tokens: 2051,
            cache_write_1h_tokens: 0,
        }
    );
}

#[test]
fn thinking_is_ignored_and_never_kept() {
    for hidden in [
        json!({ "type": "thinking", "thinking": "SECRET REASONING", "signature": "s" }),
        json!({ "type": "redacted_thinking", "data": "SECRET REASONING" }),
    ] {
        let response = ok(message(
            json!([hidden, { "type": "text", "text": "{\"a\":1}" }]),
            json!("end_turn"),
            plain_usage(),
        ));
        assert_eq!(response.output, json!({ "a": 1 }));
        assert!(!format!("{response:?}").contains("SECRET REASONING"));
    }
}

#[test]
fn there_must_be_exactly_one_text_block() {
    let text = |s: &str| json!({ "type": "text", "text": s });
    for content in [
        json!([]),
        json!([{ "type": "thinking", "thinking": "", "signature": "s" }]),
        json!([text("{}"), text("{}")]),
        json!([text("{}"), { "type": "tool_use", "id": "t", "name": "n", "input": {} }]),
        json!([{ "type": "tool_use", "id": "t", "name": "n", "input": {} }]),
        json!("not a list"),
    ] {
        match err(message(content.clone(), json!("end_turn"), plain_usage())) {
            ProviderError::InvalidOutput { usage, reason } => {
                assert_eq!(usage.input_tokens, 2095, "{content}: the call was billed");
                assert!(!reason.is_empty());
            }
            other => panic!("{content}: wrong error {other:?}"),
        }
    }
}

#[test]
fn the_text_must_be_a_json_object() {
    let secret = "ACCOUNT 1234567890";
    for text in [
        format!("not json {secret}"),
        "[1,2,3]".to_string(),
        "\"just a string\"".to_string(),
        "42".to_string(),
        "null".to_string(),
        format!("{{\"cut\": \"{secret}"),
        String::new(),
    ] {
        let error = err(message(
            json!([{ "type": "text", "text": text }]),
            json!("end_turn"),
            plain_usage(),
        ));
        match &error {
            ProviderError::InvalidOutput { reason, usage } => {
                assert!(
                    !reason.contains("1234567890"),
                    "the document's text must not leak: {reason}"
                );
                assert_eq!(usage.output_tokens, 503);
            }
            other => panic!("wrong error {other:?}"),
        }
        assert_eq!(error.failure_kind(), FailureKind::Transient);
        assert!(!error.to_string().contains("1234567890"));
    }
}

#[test]
fn a_body_that_is_not_json_at_all_is_an_invalid_output_with_no_usage() {
    for body in [
        &b""[..],
        b"<html>bad gateway</html>",
        b"{\"unfinished\":",
        b"null",
        b"[]",
    ] {
        match parse(body, MODEL, None) {
            Err(ProviderError::InvalidOutput { usage, reason }) => {
                assert_eq!(usage, Usage::default());
                assert!(!reason.contains("html"), "{reason}");
            }
            other => panic!("wrong result {other:?}"),
        }
    }
}

// ---------- stop reasons ----------

#[test]
fn a_cut_off_answer_is_truncated_and_billed() {
    let error = err(message(answer(), json!("max_tokens"), plain_usage()));
    match error {
        ProviderError::Truncated { usage } => assert_eq!(usage.output_tokens, 503),
        other => panic!("wrong error {other:?}"),
    }
}

#[test]
fn a_refusal_is_refused_and_billed() {
    let error = err(message(
        json!([{ "type": "text", "text": "I can't help with that. SECRET" }]),
        json!("refusal"),
        plain_usage(),
    ));
    assert!(matches!(error, ProviderError::Refused { usage } if usage.input_tokens == 2095));
    assert!(!format!("{error:?}{error}").contains("SECRET"));
    assert_eq!(error.failure_kind(), FailureKind::NeedsReview);
}

#[test]
fn every_other_stop_reason_is_an_invalid_output_with_its_own_fixed_reason() {
    let mut reasons = Vec::new();
    for stop in [
        json!("model_context_window_exceeded"),
        json!("pause_turn"),
        json!("tool_use"),
        json!("stop_sequence"),
        json!("something_new_in_2027"),
        json!(null),
        json!(5),
    ] {
        match err(message(answer(), stop.clone(), plain_usage())) {
            ProviderError::InvalidOutput { usage, reason } => {
                assert_eq!(usage.input_tokens, 2095, "{stop}");
                assert!(
                    !reason.contains("2027"),
                    "an unknown value is not repeated: {reason}"
                );
                reasons.push(reason);
            }
            other => panic!("{stop}: wrong error {other:?}"),
        }
    }
    let first_five = &reasons[..4];
    for (i, a) in first_five.iter().enumerate() {
        for b in &first_five[i + 1..] {
            assert_ne!(a, b, "known stop reasons say different things");
        }
    }
}

#[test]
fn the_stop_reason_is_checked_before_the_content() {
    // A cut-off answer has broken JSON in it; it is a truncation, not an invalid output.
    let cut = json!([{ "type": "text", "text": "{\"a\": \"half" }]);
    assert!(matches!(
        err(message(cut, json!("max_tokens"), plain_usage())),
        ProviderError::Truncated { .. }
    ));
}

// ---------- usage ----------

#[test]
fn missing_usage_fields_count_as_zero() {
    let response = ok(message(
        answer(),
        json!("end_turn"),
        json!({ "output_tokens": 9 }),
    ));
    assert_eq!(
        response.usage,
        Usage {
            output_tokens: 9,
            ..Usage::default()
        }
    );
    let none = ok(
        json!({ "model": MODEL, "content": answer(), "stop_reason": "end_turn" })
            .to_string()
            .into_bytes(),
    );
    assert_eq!(none.usage, Usage::default());
}

#[test]
fn the_one_hour_part_of_cache_writes_is_split_out() {
    let response = ok(message(
        answer(),
        json!("end_turn"),
        json!({
            "input_tokens": 10, "output_tokens": 20,
            "cache_creation_input_tokens": 300, "cache_read_input_tokens": 40,
            "cache_creation": { "ephemeral_5m_input_tokens": 100, "ephemeral_1h_input_tokens": 200 }
        }),
    ));
    assert_eq!(response.usage.cache_write_tokens, 300);
    assert_eq!(response.usage.cache_write_1h_tokens, 200);
    assert_eq!(response.usage.cache_write_5m_tokens(), 100);
    assert_eq!(response.usage.total(), 10 + 20 + 300 + 40);
}

#[test]
fn the_one_hour_part_can_never_exceed_the_writes() {
    let response = ok(message(
        answer(),
        json!("end_turn"),
        json!({
            "cache_creation_input_tokens": 50,
            "cache_creation": { "ephemeral_1h_input_tokens": 900 }
        }),
    ));
    assert_eq!(response.usage.cache_write_tokens, 50);
    assert_eq!(response.usage.cache_write_1h_tokens, 50);
}

#[test]
fn a_breakdown_without_a_total_is_still_counted() {
    // Only the detailed form is present: the writes are the two parts added.
    let response = ok(message(
        answer(),
        json!("end_turn"),
        json!({ "cache_creation": { "ephemeral_5m_input_tokens": 100, "ephemeral_1h_input_tokens": 200 } }),
    ));
    assert_eq!(response.usage.cache_write_tokens, 300);
    assert_eq!(response.usage.cache_write_1h_tokens, 200);
}

#[test]
fn an_implausible_usage_is_an_invalid_output() {
    for usage in [
        json!({ "input_tokens": -1 }),
        json!({ "output_tokens": 1.5 }),
        json!({ "input_tokens": "12" }),
        json!({ "cache_read_input_tokens": 1_000_000_000_000u64 }),
        json!({ "cache_creation": { "ephemeral_1h_input_tokens": -4 } }),
        json!("lots"),
    ] {
        match err(message(answer(), json!("end_turn"), usage.clone())) {
            ProviderError::InvalidOutput { reason, .. } => {
                assert!(reason.contains("usage"), "{usage}: {reason}");
            }
            other => panic!("{usage}: wrong error {other:?}"),
        }
    }
}

// ---------- model and request id ----------

#[test]
fn the_model_that_answered_is_reported() {
    let body = json!({
        "model": "claude-haiku-5-5-20260301", "content": answer(),
        "stop_reason": "end_turn", "usage": plain_usage()
    })
    .to_string()
    .into_bytes();
    assert_eq!(
        parse(&body, MODEL, None).unwrap().model,
        "claude-haiku-5-5-20260301"
    );
}

#[test]
fn a_missing_or_odd_model_name_falls_back_to_the_one_we_asked_for() {
    for model in [
        json!(null),
        json!(7),
        json!(""),
        json!("bad model\nname"),
        json!("x".repeat(500)),
    ] {
        let body = json!({
            "model": model, "content": answer(),
            "stop_reason": "end_turn", "usage": plain_usage()
        })
        .to_string()
        .into_bytes();
        assert_eq!(parse(&body, MODEL, None).unwrap().model, MODEL, "{model}");
    }
}

#[test]
fn the_request_id_is_passed_through() {
    let body = message(answer(), json!("end_turn"), plain_usage());
    assert_eq!(parse(&body, MODEL, None).unwrap().request_id, None);
    assert_eq!(
        parse(&body, MODEL, Some("req_abc".into()))
            .unwrap()
            .request_id
            .as_deref(),
        Some("req_abc")
    );
}

#[test]
fn a_usage_figure_that_is_null_counts_as_zero() {
    let response = ok(message(
        answer(),
        json!("end_turn"),
        json!({ "input_tokens": null, "output_tokens": 4, "cache_read_input_tokens": null,
                "cache_creation_input_tokens": null, "cache_creation": null }),
    ));
    assert_eq!(
        response.usage,
        Usage {
            output_tokens: 4,
            ..Usage::default()
        }
    );
}

#[test]
fn a_cache_breakdown_that_is_not_an_object_is_implausible() {
    match err(message(
        answer(),
        json!("end_turn"),
        json!({ "cache_creation": "lots" }),
    )) {
        ProviderError::InvalidOutput { reason, .. } => assert!(reason.contains("usage"), "{reason}"),
        other => panic!("wrong error {other:?}"),
    }
}
