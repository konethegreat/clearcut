// Written by Kone Tshivhinda & Claude
use super::*;
use clearcut_provider::{
    Block, CacheTtl, CallContext, Effort, ImageType, ModelRequest, ProviderError, Purpose, RequestBuilder,
    Thinking,
};
use serde_json::{json, Value};
use uuid::Uuid;

const TENANT: u128 = 0x1234_5678_9abc_def0_1234_5678_9abc_def0;

fn context() -> CallContext {
    CallContext {
        tenant: Uuid::from_u128(TENANT),
        job: Some(Uuid::from_u128(77)),
        document: Some(Uuid::from_u128(88)),
        page: Some(3),
        purpose: Purpose::Extraction,
    }
}

fn schema() -> Value {
    json!({
        "type": "object",
        "properties": { "number": { "type": "string" } },
        "required": ["number"]
    })
}

fn base(model: &str) -> RequestBuilder {
    ModelRequest::builder(model, context())
        .content(Block::text("Invoice INV-1"))
        .output_schema(schema())
        .max_output_tokens(2000)
}

fn body(request: RequestBuilder) -> Value {
    build(&request.build().unwrap()).expect("should build").body
}

fn reason(request: RequestBuilder) -> String {
    match build(&request.build().unwrap()) {
        Err(ProviderError::Rejected(reason)) => reason,
        other => panic!("expected Rejected, got {other:?}"),
    }
}

// ---------- the whole shape ----------

#[test]
fn the_smallest_request_is_exactly_this() {
    assert_eq!(
        body(base("claude-haiku-5-5")),
        json!({
            "model": "claude-haiku-5-5",
            "max_tokens": 2000,
            "stream": false,
            "messages": [
                { "role": "user", "content": [ { "type": "text", "text": "Invoice INV-1" } ] }
            ],
            "output_config": {
                "effort": "medium",
                "format": {
                    "type": "json_schema",
                    "schema": {
                        "type": "object",
                        "properties": { "number": { "type": "string" } },
                        "required": ["number"],
                        "additionalProperties": false
                    }
                }
            }
        })
    );
}

#[test]
fn a_full_request_is_exactly_this() {
    let request = base("claude-sonnet-5-5")
        .cached_system("Rules and examples.")
        .system("Today is a Tuesday.")
        .effort(Effort::High)
        .content(Block::image(ImageType::Png, b"abc".to_vec()))
        .cache_ttl(CacheTtl::OneHour);
    assert_eq!(
        body(request),
        json!({
            "model": "claude-sonnet-5-5",
            "max_tokens": 2000,
            "stream": false,
            "system": [
                { "type": "text", "text": "Rules and examples.",
                  "cache_control": { "type": "ephemeral", "ttl": "1h" } },
                { "type": "text", "text": "Today is a Tuesday." }
            ],
            "messages": [ { "role": "user", "content": [
                { "type": "text", "text": "Invoice INV-1" },
                { "type": "image", "source": {
                    "type": "base64", "media_type": "image/png", "data": "YWJj" } }
            ] } ],
            "output_config": {
                "effort": "high",
                "format": { "type": "json_schema", "schema": {
                    "type": "object",
                    "properties": { "number": { "type": "string" } },
                    "required": ["number"],
                    "additionalProperties": false
                } }
            }
        })
    );
}

#[test]
fn exactly_these_top_level_keys_are_ever_sent() {
    let full = body(
        base("claude-haiku-5-5")
            .cached_system("x")
            .thinking(Thinking::Off),
    );
    let mut keys: Vec<&str> = full.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "max_tokens",
            "messages",
            "model",
            "output_config",
            "stream",
            "system",
            "thinking"
        ]
    );
    // And never any of these (ADR-0012 §1).
    let text = full.to_string();
    for never in [
        "tools",
        "tool_choice",
        "temperature",
        "top_p",
        "top_k",
        "budget_tokens",
        "metadata",
        "service_tier",
        "inference_geo",
        "speed",
    ] {
        assert!(
            !text.contains(&format!("\"{never}\"")),
            "{never} must never be sent"
        );
    }
}

#[test]
fn nothing_about_who_or_what_the_call_is_for_is_sent() {
    let text = body(base("claude-haiku-5-5")).to_string();
    for id in [TENANT, 77, 88] {
        assert!(!text.contains(&Uuid::from_u128(id).to_string()), "{id}");
    }
    assert!(!text.to_lowercase().contains("tenant"));
}

#[test]
fn no_instructions_means_no_system_field() {
    assert!(body(base("claude-haiku-5-5")).get("system").is_none());
}

#[test]
fn the_same_request_always_gives_the_same_bytes() {
    let make = || {
        body(
            base("claude-haiku-5-5")
                .cached_system("rules")
                .content(Block::cached_text("page")),
        )
    };
    let first = make().to_string();
    for _ in 0..5 {
        assert_eq!(make().to_string(), first);
    }
}

// ---------- order, images and the cache ----------

#[test]
fn content_keeps_its_order_and_images_are_base64() {
    let request = ModelRequest::builder("claude-haiku-5-5", context())
        .content(Block::image(ImageType::Jpeg, vec![0xff, 0xd8, 0xff]))
        .content(Block::text("the page text"))
        .content(Block::image(ImageType::Png, vec![0, 1, 2, 3]))
        .output_schema(schema())
        .max_output_tokens(1000);
    let sent = body(request);
    let parts = sent["messages"][0]["content"].as_array().unwrap();
    let kinds: Vec<&str> = parts.iter().map(|p| p["type"].as_str().unwrap()).collect();
    assert_eq!(kinds, vec!["image", "text", "image"]);
    assert_eq!(parts[0]["source"]["media_type"], json!("image/jpeg"));
    assert_eq!(parts[0]["source"]["data"], json!("/9j/"));
    assert_eq!(parts[2]["source"]["media_type"], json!("image/png"));
    assert_eq!(parts[2]["source"]["data"], json!("AAECAw=="));
    assert_eq!(parts[1]["text"], json!("the page text"));
    assert_eq!(sent["messages"].as_array().unwrap().len(), 1);
    assert_eq!(sent["messages"][0]["role"], json!("user"));
}

#[test]
fn a_cache_breakpoint_in_the_content_gets_cache_control_too() {
    let request = base("claude-haiku-5-5")
        .content(Block::cached_text("end of the stable part"))
        .content(Block::text("after"));
    let sent = body(request);
    let parts = sent["messages"][0]["content"].as_array().unwrap();
    assert!(parts[0].get("cache_control").is_none());
    assert_eq!(parts[1]["cache_control"], json!({ "type": "ephemeral" }));
    assert!(parts[2].get("cache_control").is_none());
}

#[test]
fn the_cache_lifetime_applies_to_every_breakpoint() {
    for (ttl, expected) in [
        (CacheTtl::FiveMinutes, json!({ "type": "ephemeral" })),
        (CacheTtl::OneHour, json!({ "type": "ephemeral", "ttl": "1h" })),
    ] {
        let sent = body(
            base("claude-haiku-5-5")
                .cached_system("a")
                .content(Block::cached_text("b"))
                .cache_ttl(ttl),
        );
        assert_eq!(sent["system"][0]["cache_control"], expected);
        assert_eq!(sent["messages"][0]["content"][1]["cache_control"], expected);
    }
}

// ---------- effort and thinking ----------

#[test]
fn effort_is_sent_as_its_api_name() {
    for (effort, name) in [
        (Effort::Low, "low"),
        (Effort::Medium, "medium"),
        (Effort::High, "high"),
    ] {
        assert_eq!(
            body(base("claude-haiku-5-5").effort(effort))["output_config"]["effort"],
            json!(name)
        );
    }
}

#[test]
fn adaptive_thinking_is_the_default_so_the_field_is_left_out() {
    for model in [
        "claude-haiku-5-5",
        "claude-sonnet-5-5",
        "claude-opus-5-5",
        "claude-fable-5-1",
    ] {
        let sent = body(base(model).thinking(Thinking::Adaptive));
        assert!(sent.get("thinking").is_none(), "{model}");
    }
}

#[test]
fn thinking_off_depends_on_the_model() {
    assert_eq!(
        body(base("claude-haiku-5-5").thinking(Thinking::Off))["thinking"],
        json!({ "type": "disabled" })
    );
    assert_eq!(
        body(base("claude-sonnet-5-5").thinking(Thinking::Off))["thinking"],
        json!({ "type": "between_tools" })
    );
}

#[test]
fn thinking_off_is_refused_where_the_model_cannot_do_it() {
    for model in [
        "claude-opus-5-5",
        "claude-fable-5-1",
        "claude-future-9",
        "mystery",
    ] {
        let why = reason(base(model).thinking(Thinking::Off));
        assert!(why.contains(model), "{why}");
        assert!(why.to_lowercase().contains("thinking"), "{why}");
    }
}

#[test]
fn a_refusal_for_thinking_is_a_needs_review_not_a_retry() {
    use clearcut_core::jobs::FailureKind;
    let error = build(&base("claude-opus-5-5").thinking(Thinking::Off).build().unwrap()).unwrap_err();
    assert_eq!(error.failure_kind(), FailureKind::NeedsReview);
    assert_eq!(error.usage(), None);
}

#[test]
fn dated_and_suffixed_model_ids_of_a_known_family_still_match() {
    assert!(body(base("claude-haiku-5-5-20260301").thinking(Thinking::Off))
        .get("thinking")
        .is_some());
    assert!(build(
        &base("claude-opus-5-5-20260301")
            .thinking(Thinking::Off)
            .build()
            .unwrap()
    )
    .is_err());
}

// ---------- output size ----------

#[test]
fn up_to_sixteen_thousand_output_tokens_are_allowed_and_more_needs_streaming() {
    assert_eq!(
        body(base("claude-haiku-5-5").max_output_tokens(16_000))["max_tokens"],
        json!(16_000)
    );
    assert_eq!(
        body(base("claude-haiku-5-5").max_output_tokens(1))["max_tokens"],
        json!(1)
    );
    let why = reason(base("claude-haiku-5-5").max_output_tokens(16_001));
    assert!(why.contains("streaming"), "{why}");
    assert!(why.contains("16000"), "{why}");
}

// ---------- the schema profile is applied ----------

#[test]
fn the_output_schema_is_profiled_before_it_is_sent() {
    let sent = body(
        ModelRequest::builder("claude-haiku-5-5", context())
            .content(Block::text("x"))
            .output_schema(json!({
                "type": "object",
                "properties": { "qty": { "type": "integer", "minimum": 1 } },
                "required": ["qty"]
            }))
            .max_output_tokens(500),
    );
    let schema = &sent["output_config"]["format"]["schema"];
    assert_eq!(schema["additionalProperties"], json!(false));
    assert!(schema["properties"]["qty"].get("minimum").is_none());
    assert!(schema["properties"]["qty"]["description"]
        .as_str()
        .unwrap()
        .contains("not enforced here"));
}

#[test]
fn the_rules_that_were_removed_are_reported_to_the_caller() {
    let built = build(
        &ModelRequest::builder("claude-haiku-5-5", context())
            .content(Block::text("x"))
            .output_schema(json!({
                "type": "object",
                "properties": { "qty": { "type": "integer", "minimum": 1 } },
                "required": ["qty"]
            }))
            .max_output_tokens(500)
            .build()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(built.removed.len(), 1);
    assert_eq!(built.removed[0].keyword, "minimum");
}

#[test]
fn a_schema_the_api_cannot_take_is_refused_here_with_the_reason() {
    let why = reason(
        ModelRequest::builder("claude-haiku-5-5", context())
            .content(Block::text("x"))
            .output_schema(json!({
                "type": "object",
                "properties": { "x": { "enum": [{ "a": 1 }] } },
                "required": ["x"]
            }))
            .max_output_tokens(500),
    );
    assert!(why.contains("/properties/x"), "{why}");
    assert!(why.contains("enum"), "{why}");
}

#[test]
fn a_refusal_never_contains_the_documents_text() {
    let secret = "ACCOUNT 1234567890 SECRET";
    let request = ModelRequest::builder("claude-opus-5-5", context())
        .cached_system(secret)
        .content(Block::text(secret))
        .output_schema(schema())
        .max_output_tokens(500)
        .thinking(Thinking::Off);
    assert!(!reason(request).contains(secret));
}

#[test]
fn a_model_name_that_only_starts_like_a_family_is_not_in_it() {
    // `claude-haiku-50` is not `claude-haiku-5`: thinking cannot be switched off blindly.
    let why = reason(base("claude-haiku-50").thinking(Thinking::Off));
    assert!(why.contains("claude-haiku-50"), "{why}");
    assert!(body(base("claude-haiku-5").thinking(Thinking::Off))
        .get("thinking")
        .is_some());
}
