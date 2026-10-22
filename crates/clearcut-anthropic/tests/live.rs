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
