// Written by Kone Tshivhinda & Claude
use super::*;
use clearcut_core::compliance_za::Issue;
use clearcut_core::event::TotalField;
use clearcut_core::Money;

#[test]
fn every_kind_of_finding_has_one_stable_key() {
    let stated = Money::zar(100);
    let recomputed = Money::zar(200);
    let cases = [
        (Finding::NoLines, "no_lines"),
        (Finding::CurrencyMismatch { line: 3 }, "currency_mismatch:3"),
        (
            Finding::NonPositiveQuantity { line: 1 },
            "non_positive_quantity:1",
        ),
        (Finding::AmountOutOfRange { line: 12 }, "amount_out_of_range:12"),
        (
            Finding::LineTotalMismatch {
                line: 2,
                stated,
                recomputed,
            },
            "line_total_mismatch:2",
        ),
        (
            Finding::TotalsMismatch {
                field: TotalField::Net,
                stated,
                recomputed,
            },
            "totals_mismatch:net",
        ),
        (
            Finding::TotalsMismatch {
                field: TotalField::Vat,
                stated,
                recomputed,
            },
            "totals_mismatch:vat",
        ),
        (
            Finding::TotalsMismatch {
                field: TotalField::Gross,
                stated,
                recomputed,
            },
            "totals_mismatch:gross",
        ),
        (Finding::TotalsOutOfRange, "totals_out_of_range"),
        (Finding::DueBeforeIssue, "due_before_issue"),
        (
            Finding::Compliance(Issue {
                field: "supplier_address",
                message: "m",
            }),
            "compliance:supplier_address",
        ),
    ];
    for (finding, key) in cases {
        assert_eq!(finding_key(&finding), key);
    }
}

#[test]
fn a_key_does_not_contain_the_amounts() {
    let finding = Finding::TotalsMismatch {
        field: TotalField::Gross,
        stated: Money::zar(123_456),
        recomputed: Money::zar(654_321),
    };
    let key = finding_key(&finding);
    assert!(!key.contains("123456") && !key.contains("654321"));
}

#[test]
fn keys_of_a_validation_are_sorted_so_order_never_matters() {
    let findings = vec![
        Finding::DueBeforeIssue,
        Finding::NoLines,
        Finding::CurrencyMismatch { line: 2 },
    ];
    assert_eq!(
        finding_keys(&findings),
        ["currency_mismatch:2", "due_before_issue", "no_lines"]
    );
}

#[test]
fn repeated_findings_keep_their_repeats() {
    let findings = vec![
        Finding::Compliance(Issue {
            field: "x",
            message: "a",
        }),
        Finding::Compliance(Issue {
            field: "x",
            message: "b",
        }),
    ];
    assert_eq!(finding_keys(&findings), ["compliance:x", "compliance:x"]);
}

#[test]
fn comparing_expected_and_actual_reports_what_is_missing_and_what_is_unexpected() {
    let expected = vec![
        "no_lines".to_string(),
        "totals_mismatch:vat".to_string(),
        "totals_mismatch:vat".to_string(),
    ];
    let actual = vec!["totals_mismatch:vat".to_string(), "due_before_issue".to_string()];
    let difference = compare(&expected, &actual);
    assert_eq!(difference.missing, ["no_lines", "totals_mismatch:vat"]);
    assert_eq!(difference.unexpected, ["due_before_issue"]);
    assert!(!difference.is_empty());
}

#[test]
fn equal_lists_in_any_order_have_no_difference() {
    let a = vec!["b".to_string(), "a".to_string(), "a".to_string()];
    let b = vec!["a".to_string(), "a".to_string(), "b".to_string()];
    assert!(compare(&a, &b).is_empty());
    assert!(compare(&[], &[]).is_empty());
}

#[test]
fn every_key_the_system_produces_is_accepted_in_a_label() {
    use crate::label::is_finding_key;
    let amount = Money::zar(1);
    let findings = [
        Finding::NoLines,
        Finding::CurrencyMismatch { line: 3 },
        Finding::NonPositiveQuantity { line: 1 },
        Finding::AmountOutOfRange { line: 12 },
        Finding::LineTotalMismatch {
            line: 2,
            stated: amount,
            recomputed: amount,
        },
        Finding::TotalsMismatch {
            field: TotalField::Net,
            stated: amount,
            recomputed: amount,
        },
        Finding::TotalsMismatch {
            field: TotalField::Vat,
            stated: amount,
            recomputed: amount,
        },
        Finding::TotalsMismatch {
            field: TotalField::Gross,
            stated: amount,
            recomputed: amount,
        },
        Finding::TotalsOutOfRange,
        Finding::CarriedBalanceMismatch {
            after_page: 2,
            carried_forward: amount,
            brought_forward: amount,
        },
        Finding::DueBeforeIssue,
        Finding::Compliance(Issue {
            field: "recipient_vat_number",
            message: "m",
        }),
    ];
    for finding in findings {
        let key = finding_key(&finding);
        assert!(is_finding_key(&key), "{key}");
    }
}
