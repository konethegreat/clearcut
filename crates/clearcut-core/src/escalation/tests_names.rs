// Written by Kone Tshivhinda & Claude
//! The fixed names that are stored in the database round-trip exactly.

use super::*;

#[test]
fn every_review_reason_has_a_name_that_parses_back() {
    for reason in ReviewReason::ALL {
        assert_eq!(ReviewReason::parse(reason.as_str()), Some(reason));
    }
}

#[test]
fn a_name_that_is_not_exactly_a_review_reason_is_none() {
    for name in [
        "",
        "Budget",
        " budget",
        "budget ",
        "budgets",
        "top-rung-failed",
        "unknown",
    ] {
        assert_eq!(ReviewReason::parse(name), None, "{name:?}");
    }
}

#[test]
fn the_confidence_names_are_the_ones_the_database_checks() {
    assert_eq!(ConfidenceBasis::TextConfirmed.as_str(), "text_confirmed");
    assert_eq!(ConfidenceBasis::ArithmeticOnly.as_str(), "arithmetic_only");
    assert_eq!(ConfidenceBasis::Unresolved.as_str(), "unresolved");
    for basis in [
        ConfidenceBasis::TextConfirmed,
        ConfidenceBasis::ArithmeticOnly,
        ConfidenceBasis::Unresolved,
    ] {
        assert_eq!(ConfidenceBasis::parse(basis.as_str()), Some(basis));
        // The same names the serialised form uses.
        assert_eq!(serde_json::to_value(basis).unwrap(), basis.as_str());
    }
    assert_eq!(ConfidenceBasis::parse("Unresolved"), None);
    assert_eq!(ConfidenceBasis::parse(""), None);
}

#[test]
fn the_effort_names_are_the_ones_the_database_checks() {
    assert_eq!(ThinkingEffort::Low.as_str(), "low");
    assert_eq!(ThinkingEffort::Medium.as_str(), "medium");
    assert_eq!(ThinkingEffort::High.as_str(), "high");
    for effort in [ThinkingEffort::Low, ThinkingEffort::Medium, ThinkingEffort::High] {
        assert_eq!(serde_json::to_value(effort).unwrap(), effort.as_str());
    }
}
