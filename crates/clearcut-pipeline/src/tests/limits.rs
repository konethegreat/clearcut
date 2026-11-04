// Written by Kone Tshivhinda & Claude
//! What the server learns from the Models API narrows the ladder before a page is read.

use crate::{ModelLimits, ReadSettings};
use std::collections::BTreeMap;

#[test]
fn the_settings_skip_a_model_the_key_cannot_use_and_clamp_its_answer() {
    let settings = ReadSettings::standard().with_limits(&ModelLimits {
        unavailable: vec!["claude-sonnet-5-5".to_string()],
        max_output_tokens: BTreeMap::from([("claude-haiku-5-5".to_string(), 10_000)]),
        max_input_tokens: BTreeMap::from([("claude-haiku-5-5".to_string(), 200_000)]),
    });

    let names: Vec<&str> = settings
        .router()
        .policy
        .rungs()
        .iter()
        .map(|rung| rung.model.as_str())
        .collect();
    assert_eq!(names, ["claude-haiku-5-5", "claude-opus-5-5", "claude-fable-5-1"]);
    assert_eq!(settings.router().policy.rungs()[0].max_output_tokens, 10_000);
    assert_eq!(
        settings.router().model_input_limits.get("claude-haiku-5-5"),
        Some(&200_000)
    );
}

#[test]
fn limits_that_name_no_ladder_model_leave_the_ladder_as_it_was() {
    let before = ReadSettings::standard();
    for limits in [
        ModelLimits::default(),
        ModelLimits {
            unavailable: vec!["claude-legacy-1".to_string()],
            max_output_tokens: BTreeMap::from([("claude-legacy-1".to_string(), 1_000)]),
            max_input_tokens: BTreeMap::from([("claude-legacy-1".to_string(), 1_000)]),
        },
    ] {
        let after = before.clone().with_limits(&limits);
        assert_eq!(after.router().policy, before.router().policy);
    }
}
