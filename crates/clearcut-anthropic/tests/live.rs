// Written by Kone Tshivhinda & Claude
//! A live check of a real key against the real Anthropic API.
//!
//! **Ignored by default, and never run in CI** (CI has no key). It spends **no
//! tokens**: `GET /v1/models` is free. To run it, with your own key:
//!
//! ```text
//! ANTHROPIC_API_KEY=sk-ant-api03-... cargo test -p clearcut-anthropic --test live -- --ignored --nocapture
//! ```
//!
//! It prints which of ClearCut's four models the key can use (never the key).

use clearcut_anthropic::{check_key_with, load, AnthropicConfig};

#[tokio::test]
#[ignore = "needs your own ANTHROPIC_API_KEY; costs nothing"]
async fn the_key_in_the_environment_is_accepted_by_anthropic() {
    let key = load().expect("set ANTHROPIC_API_KEY (or ANTHROPIC_API_KEY_FILE)");
    let config = AnthropicConfig::from_env(|name| std::env::var_os(name)).expect("a valid configuration");
    let check = check_key_with(&config, &key)
        .await
        .unwrap_or_else(|error| panic!("the check failed: {error}"));

    println!(
        "key {} accepted; {} models visible",
        key.fingerprint(),
        check.models.len()
    );
    for id in [
        "claude-haiku-5-5",
        "claude-sonnet-5-5",
        "claude-opus-5-5",
        "claude-fable-5-1",
    ] {
        match check.model(id) {
            Some(model) => println!(
                "  {id}: {:?}, structured outputs {}, images {}, pdf {}, batch {}",
                model.lifecycle, model.structured_outputs, model.image_input, model.pdf_input, model.batch
            ),
            None => println!("  {id}: not available to this key"),
        }
    }
    assert!(
        check.can_use("claude-haiku-5-5"),
        "ClearCut's default model is not available to this key"
    );
}

/// One tiny real call through the whole adapter. **This one costs a little**:
/// about 60 input and 15 output tokens on Haiku 5.5, a small fraction of a cent.
/// It exists so that the first time the adapter meets the real API is on
/// purpose, with your own key, and not in the middle of an invoice.
///
/// ```text
/// ANTHROPIC_API_KEY=sk-ant-api03-... cargo test -p clearcut-anthropic --test live -- --ignored --nocapture one_tiny
/// ```
#[tokio::test]
#[ignore = "needs your own ANTHROPIC_API_KEY; costs a fraction of a cent"]
async fn one_tiny_real_call_goes_through_the_adapter() {
    use clearcut_anthropic::provider::AnthropicProvider;
    use clearcut_provider::{Block, CallContext, Effort, ModelProvider, ModelRequest, Purpose};
    use serde_json::json;

    let key = load().expect("set ANTHROPIC_API_KEY (or ANTHROPIC_API_KEY_FILE)");
    let config = AnthropicConfig::from_env(|name| std::env::var_os(name)).expect("a valid configuration");
    let provider = AnthropicProvider::new(&config, &key).expect("a client");

    let request = ModelRequest::builder(
        "claude-haiku-5-5",
        CallContext {
            tenant: uuid::Uuid::nil(),
            job: None,
            document: None,
            page: None,
            purpose: Purpose::Extraction,
        },
    )
    .system("Answer with the JSON the schema asks for. Nothing else.")
    .content(Block::text("Say that you are working."))
    .effort(Effort::Low)
    .output_schema(json!({
        "type": "object",
        "properties": { "status": { "type": "string", "description": "the word ok" } },
        "required": ["status"]
    }))
    .max_output_tokens(200)
    .build()
    .expect("a valid request");

    let response = provider
        .complete(request)
        .await
        .unwrap_or_else(|error| panic!("the call failed: {error}"));
    assert!(response.output.is_object(), "{:?}", response);

    // A rough bill, in whole numbers (no floats for money): Haiku 5.5 at R18/USD
    // and prompts under 100k tokens costs R1.80 per million input tokens and
    // R9.00 per million output tokens, so 180 and 900 hundredths of a millionth of
    // a rand per token. Caching is ignored.
    let u = response.usage;
    let millionths = (u.input_tokens * 180 + u.output_tokens * 900) / 100;
    println!(
        "model {}, request {:?}, usage {:?}, about R{}.{:06}",
        response.model,
        response.request_id,
        u,
        millionths / 1_000_000,
        millionths % 1_000_000
    );
}
