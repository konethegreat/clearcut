// Written by Kone Tshivhinda & Claude
use super::*;
use crate::Error;

// ---------- the names that are stored and shown ----------

#[test]
fn escalation_reasons_have_exactly_the_agreed_names() {
    let names: Vec<&str> = EscalationReason::ALL.iter().map(|r| r.as_str()).collect();
    assert_eq!(
        names,
        [
            "reading_problems",
            "line_arithmetic",
            "totals_mismatch",
            "text_check_failed",
            "schema_invalid",
            "rung_unavailable",
        ]
    );
}

#[test]
fn review_reasons_have_exactly_the_agreed_names() {
    let names: Vec<&str> = ReviewReason::ALL.iter().map(|r| r.as_str()).collect();
    assert_eq!(
        names,
        [
            "top_rung_failed",
            "too_long",
            "refused",
            "budget",
            "call_cap",
            "cannot_be_sent_safely",
            "request_rejected",
            "no_rung_available",
            "amounts_include_vat",
            "tax_not_shown",
        ]
    );
}

#[test]
fn confidence_comes_from_evidence_not_from_the_rung() {
    assert_eq!(ConfidenceBasis::TextConfirmed.per_mille(), 950);
    assert_eq!(ConfidenceBasis::ArithmeticOnly.per_mille(), 850);
    assert_eq!(ConfidenceBasis::Unresolved.per_mille(), 500);
    // The gold set flags anything under 800 for review (ADR-0010): only the last is flagged.
    assert!(ConfidenceBasis::ArithmeticOnly.per_mille() >= 800);
    assert!(ConfidenceBasis::Unresolved.per_mille() < 800);
}

// ---------- the default ladder (ADR-0018 §2) ----------

#[test]
fn the_default_ladder_is_the_one_in_the_adr() {
    let policy = RouterPolicy::default_ladder();
    let rungs = policy.rungs();
    assert_eq!(rungs.len(), 4);

    let row = |i: usize| {
        let r = &rungs[i];
        (
            r.model.as_str(),
            r.effort,
            r.max_output_tokens,
            (r.picture.dpi, r.picture.max_edge),
            r.enabled,
        )
    };
    assert_eq!(
        row(0),
        (
            "claude-haiku-5-5",
            ThinkingEffort::Medium,
            16_000,
            (150, 2_000),
            true
        )
    );
    assert_eq!(
        row(1),
        (
            "claude-sonnet-5-5",
            ThinkingEffort::Medium,
            12_000,
            (150, 2_000),
            true
        )
    );
    assert_eq!(
        row(2),
        ("claude-opus-5-5", ThinkingEffort::High, 8_000, (200, 2_576), true)
    );
    // Fable keeps data for 30 days and an account may refuse it: off unless someone turns it on.
    assert_eq!(
        row(3),
        (
            "claude-fable-5-1",
            ThinkingEffort::High,
            4_000,
            (200, 2_576),
            false
        )
    );

    assert_eq!(policy.answer_ceiling(), 16_000);
    assert_eq!(policy.max_calls_per_page(), 5);
    assert_eq!(policy.document_call_cap(1), 13);
    assert_eq!(policy.document_call_cap(10), 40);
}

#[test]
fn only_enabled_rungs_are_in_the_effective_ladder() {
    let policy = RouterPolicy::default_ladder();
    let names: Vec<&str> = policy.effective().iter().map(|r| r.model.as_str()).collect();
    assert_eq!(
        names,
        ["claude-haiku-5-5", "claude-sonnet-5-5", "claude-opus-5-5"]
    );
    let with_fable = policy.with_enabled("claude-fable-5-1", true).unwrap();
    assert_eq!(with_fable.effective().len(), 4);
}

#[test]
fn a_model_can_be_switched_on_or_off_by_name_and_an_unknown_name_is_refused() {
    let policy = RouterPolicy::default_ladder();
    let off = policy.with_enabled("claude-haiku-5-5", false).unwrap();
    assert_eq!(off.effective()[0].model, "claude-sonnet-5-5");
    assert_eq!(
        policy.with_enabled("claude-nothing", true),
        Err(Error::InvalidRouterPolicy("no such model in the ladder"))
    );
    // Switching off the last enabled rungs leaves nothing to run.
    let none = policy
        .with_enabled("claude-haiku-5-5", false)
        .unwrap()
        .with_enabled("claude-sonnet-5-5", false)
        .unwrap();
    assert_eq!(
        none.with_enabled("claude-opus-5-5", false),
        Err(Error::InvalidRouterPolicy("at least one rung must be on"))
    );
}

#[test]
fn models_the_key_cannot_use_are_taken_out_without_losing_the_rest() {
    let policy = RouterPolicy::default_ladder();
    let narrowed = policy.without_models(&["claude-sonnet-5-5"]);
    let names: Vec<&str> = narrowed.effective().iter().map(|r| r.model.as_str()).collect();
    assert_eq!(names, ["claude-haiku-5-5", "claude-opus-5-5"]);
    // Taking every model out is allowed here (the key may be useless); the router then reports it.
    let all: Vec<&str> = policy.rungs().iter().map(|r| r.model.as_str()).collect();
    assert!(policy.without_models(&all).effective().is_empty());
}

// ---------- the document's call cap ----------

#[test]
fn a_document_may_make_three_calls_a_page_plus_ten() {
    let policy = RouterPolicy::default_ladder();
    assert_eq!(policy.document_call_cap(0), 10);
    assert_eq!(policy.document_call_cap(100), 310);
    assert_eq!(policy.document_call_cap(u32::MAX), u32::MAX);
}

// ---------- building a policy ----------

fn rung(model: &str) -> Rung {
    Rung {
        model: model.to_string(),
        effort: ThinkingEffort::Medium,
        max_output_tokens: 8_000,
        picture: Picture {
            dpi: 150,
            max_edge: 2_000,
        },
        enabled: true,
    }
}

fn build(rungs: Vec<Rung>) -> Result<RouterPolicy, Error> {
    RouterPolicy::new(rungs, 16_000, 5, 3, 10)
}

#[test]
fn a_valid_policy_is_built() {
    let policy = build(vec![rung("a"), rung("b")]).unwrap();
    assert_eq!(policy.rungs().len(), 2);
}

#[test]
fn a_policy_without_rungs_or_with_none_on_is_refused() {
    assert_eq!(
        build(vec![]),
        Err(Error::InvalidRouterPolicy("a ladder needs at least one rung"))
    );
    let mut off = rung("a");
    off.enabled = false;
    assert_eq!(
        build(vec![off]),
        Err(Error::InvalidRouterPolicy("at least one rung must be on"))
    );
}

#[test]
fn model_ids_must_be_plain_and_unique() {
    for bad in ["", "has space", "slash/model", "quote\"", "ünï", &"m".repeat(129)] {
        assert_eq!(
            build(vec![rung(bad)]),
            Err(Error::InvalidRouterPolicy("a model id is not a plain identifier")),
            "{bad:?}"
        );
    }
    assert!(build(vec![rung(&"m".repeat(128))]).is_ok());
    assert!(build(vec![rung("claude-opus-5-5"), rung("model.v2_x")]).is_ok());
    assert_eq!(
        build(vec![rung("a"), rung("a")]),
        Err(Error::InvalidRouterPolicy("a model appears twice in the ladder"))
    );
}

#[test]
fn the_answer_cap_of_a_rung_is_between_one_token_and_the_ceiling() {
    let mut zero = rung("a");
    zero.max_output_tokens = 0;
    assert_eq!(
        build(vec![zero]),
        Err(Error::InvalidRouterPolicy(
            "a rung's answer cap is outside 1 to the ceiling"
        ))
    );
    let mut high = rung("a");
    high.max_output_tokens = 16_001;
    assert_eq!(
        build(vec![high]),
        Err(Error::InvalidRouterPolicy(
            "a rung's answer cap is outside 1 to the ceiling"
        ))
    );
    let mut edge = rung("a");
    edge.max_output_tokens = 16_000;
    assert!(build(vec![edge]).is_ok());
    let mut one = rung("a");
    one.max_output_tokens = 1;
    assert!(build(vec![one]).is_ok());
}

#[test]
fn the_ceiling_cannot_pass_what_the_provider_allows() {
    let ok = RouterPolicy::new(vec![rung("a")], 64_000, 5, 3, 10);
    assert!(ok.is_ok());
    assert_eq!(
        RouterPolicy::new(vec![rung("a")], 64_001, 5, 3, 10),
        Err(Error::InvalidRouterPolicy(
            "the answer ceiling is outside 1 to 64000"
        ))
    );
    assert_eq!(
        RouterPolicy::new(vec![rung("a")], 0, 5, 3, 10),
        Err(Error::InvalidRouterPolicy(
            "the answer ceiling is outside 1 to 64000"
        ))
    );
}

#[test]
fn the_picture_of_a_rung_stays_inside_what_the_worker_draws() {
    for (dpi, edge, ok) in [
        (99, 2_000, false),
        (100, 2_000, true),
        (300, 2_000, true),
        (301, 2_000, false),
        (150, 499, false),
        (150, 500, true),
        (150, 2_576, true),
        (150, 2_577, false),
    ] {
        let mut r = rung("a");
        r.picture = Picture { dpi, max_edge: edge };
        assert_eq!(build(vec![r]).is_ok(), ok, "{dpi} dpi, {edge} px");
    }
    let mut bad = rung("a");
    bad.picture.dpi = 50;
    assert_eq!(
        build(vec![bad]),
        Err(Error::InvalidRouterPolicy(
            "a rung's picture is outside what the worker draws"
        ))
    );
}

#[test]
fn the_call_caps_must_be_above_zero() {
    for (per_page, per_doc_page, extra, ok) in [
        (0, 3, 10, false),
        (1, 3, 10, true),
        (5, 0, 10, false),
        (5, 1, 0, true),
        (5, 3, 0, true),
    ] {
        let result = RouterPolicy::new(vec![rung("a")], 16_000, per_page, per_doc_page, extra);
        assert_eq!(result.is_ok(), ok, "{per_page} {per_doc_page} {extra}");
    }
    assert_eq!(
        RouterPolicy::new(vec![rung("a")], 16_000, 0, 3, 10),
        Err(Error::InvalidRouterPolicy("a call cap must be above zero"))
    );
}

// ---------- a policy read back is checked like one built ----------

#[test]
fn a_policy_survives_being_written_and_read_back() {
    let policy = RouterPolicy::default_ladder();
    let json = serde_json::to_string(&policy).unwrap();
    let back: RouterPolicy = serde_json::from_str(&json).unwrap();
    assert_eq!(back, policy);
}

fn tampered(change: impl FnOnce(&mut serde_json::Value)) -> Result<RouterPolicy, serde_json::Error> {
    let mut value = serde_json::to_value(RouterPolicy::default_ladder()).unwrap();
    change(&mut value);
    serde_json::from_value(value)
}

#[test]
fn a_policy_read_from_text_cannot_skip_the_checks() {
    use serde_json::json;
    assert!(tampered(|_| {}).is_ok());
    let refused: Vec<(&str, Tamper)> = vec![
        ("no rungs", Box::new(|v| v["rungs"] = json!([]))),
        (
            "a model id with a space",
            Box::new(|v| v["rungs"][0]["model"] = json!("has space")),
        ),
        (
            "a duplicate model",
            Box::new(|v| v["rungs"][1]["model"] = json!("claude-haiku-5-5")),
        ),
        (
            "an answer over the ceiling",
            Box::new(|v| v["rungs"][0]["max_output_tokens"] = json!(16_001)),
        ),
        (
            "a zero answer",
            Box::new(|v| v["rungs"][0]["max_output_tokens"] = json!(0)),
        ),
        (
            "a picture too small",
            Box::new(|v| v["rungs"][0]["picture"]["dpi"] = json!(50)),
        ),
        (
            "a ceiling too large",
            Box::new(|v| v["answer_ceiling"] = json!(64_001)),
        ),
        (
            "a zero page cap",
            Box::new(|v| v["max_calls_per_page"] = json!(0)),
        ),
        (
            "every rung off",
            Box::new(|v| {
                for rung in v["rungs"].as_array_mut().unwrap() {
                    rung["enabled"] = json!(false);
                }
            }),
        ),
    ];
    for (case, change) in refused {
        assert!(tampered(change).is_err(), "{case}");
    }
}

/// A change to make to a stored value, to see whether it is still accepted.
type Tamper = Box<dyn FnOnce(&mut serde_json::Value)>;
