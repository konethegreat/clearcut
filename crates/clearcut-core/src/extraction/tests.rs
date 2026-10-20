// Written by Kone Tshivhinda & Claude
use super::fixtures::{at, clean, correct_totals, line, zar};
use super::*;
use crate::compliance_za::Issue;
use crate::event::{DocumentId, Locator, TotalField};
use crate::{Currency, InvoiceTotals, Money, TaxCategory};
use serde_json::json;
use std::num::NonZeroU32;
use time::macros::date as d;

fn issue(field: &'static str, message: &'static str) -> Finding {
    Finding::Compliance(Issue { field, message })
}

// ---------- a clean extraction ----------

#[test]
fn a_correct_complete_invoice_has_no_findings() {
    let result = validate(&clean());
    assert_eq!(result.findings, []);
    assert!(result.is_clean());
    assert_eq!(result.recomputed, Some(correct_totals()));
}

// ---------- lines ----------

#[test]
fn an_invoice_without_lines_is_reported_and_nothing_else_is_guessed() {
    let mut x = clean();
    x.lines.clear();
    x.stated_totals = None;
    let result = validate(&x);
    assert_eq!(result.findings, [Finding::NoLines]);
    assert_eq!(result.recomputed, None);
}

#[test]
fn a_wrong_line_total_is_reported_with_the_line_number() {
    let mut x = clean();
    x.lines[1].value.line_total = zar(110_000); // should be R1 000.00
    let result = validate(&x);
    assert_eq!(
        result.findings,
        [Finding::LineTotalMismatch {
            line: 2,
            stated: zar(110_000),
            recomputed: zar(100_000),
        }]
    );
    // Totals are rebuilt from quantity x price, not from the stated line total.
    assert_eq!(result.recomputed, Some(correct_totals()));
}

#[test]
fn fractional_quantities_are_rounded_half_away_from_zero() {
    let mut x = clean();
    // 2.5 x R0.01 = R0.025 -> R0.03 (half away from zero)
    x.lines = vec![line("Tiny", 2_500, 1, 3, TaxCategory::ZeroRated)];
    x.stated_totals = None;
    x.recipient = None;
    let result = validate(&x);
    assert_eq!(result.findings, []);
}

#[test]
fn a_zero_or_negative_quantity_is_reported() {
    for qty in [0, -1_000] {
        let mut x = clean();
        x.lines[0].value.quantity_milli = qty;
        let result = validate(&x);
        assert_eq!(
            result.findings,
            [Finding::NonPositiveQuantity { line: 1 }],
            "{qty}"
        );
        // With a line we cannot trust, no totals are recomputed or compared.
        assert_eq!(result.recomputed, None);
    }
}

#[test]
fn a_line_in_another_currency_is_reported() {
    let mut x = clean();
    x.lines[1].value.unit_price = Money::new(100_000, Currency::Usd);
    let result = validate(&x);
    assert_eq!(result.findings, [Finding::CurrencyMismatch { line: 2 }]);
    assert_eq!(result.recomputed, None);
}

#[test]
fn amounts_too_large_to_calculate_are_reported() {
    let mut x = clean();
    x.lines[0].value.unit_price = zar(i64::MAX);
    x.lines[0].value.line_total = zar(i64::MAX);
    let result = validate(&x);
    assert_eq!(result.findings, [Finding::AmountOutOfRange { line: 1 }]);
    assert_eq!(result.recomputed, None);
}

#[test]
fn a_sum_too_large_to_calculate_is_reported() {
    let mut x = clean();
    // Each line is fine on its own (R50 quadrillion), but together they overflow.
    let huge = 5_000_000_000_000_000_000;
    x.lines = vec![
        line("A", 1_000, huge, huge, TaxCategory::ZeroRated),
        line("B", 1_000, huge, huge, TaxCategory::ZeroRated),
    ];
    x.stated_totals = None;
    let result = validate(&x);
    assert_eq!(result.findings, [Finding::TotalsOutOfRange]);
    assert_eq!(result.recomputed, None);
}

#[test]
fn every_bad_line_is_reported_in_line_order() {
    let mut x = clean();
    x.lines[0].value.quantity_milli = 0;
    x.lines[1].value.line_total = zar(1);
    let result = validate(&x);
    assert_eq!(
        result.findings,
        [
            Finding::NonPositiveQuantity { line: 1 },
            Finding::LineTotalMismatch {
                line: 2,
                stated: zar(1),
                recomputed: zar(100_000),
            },
        ]
    );
}

// ---------- stated totals ----------

#[test]
fn each_wrong_total_is_reported_on_its_own() {
    let cases = [
        (TotalField::Net, 1_600_001, 225_000, 1_825_000),
        (TotalField::Vat, 1_600_000, 225_001, 1_825_000),
        (TotalField::Gross, 1_600_000, 225_000, 1_825_001),
    ];
    for (field, net, vat, gross) in cases {
        let mut x = clean();
        x.stated_totals = Some(at(InvoiceTotals {
            net: zar(net),
            vat: zar(vat),
            gross: zar(gross),
        }));
        let stated = match field {
            TotalField::Net => zar(net),
            TotalField::Vat => zar(vat),
            TotalField::Gross => zar(gross),
        };
        let recomputed = match field {
            TotalField::Net => zar(1_600_000),
            TotalField::Vat => zar(225_000),
            TotalField::Gross => zar(1_825_000),
        };
        assert_eq!(
            validate(&x).findings,
            [Finding::TotalsMismatch {
                field,
                stated,
                recomputed
            }],
            "{field:?}"
        );
    }
}

#[test]
fn all_three_wrong_totals_are_reported_net_then_vat_then_gross() {
    let mut x = clean();
    x.stated_totals = Some(at(InvoiceTotals {
        net: zar(1),
        vat: zar(2),
        gross: zar(3),
    }));
    let fields: Vec<TotalField> = validate(&x)
        .findings
        .iter()
        .filter_map(|f| match f {
            Finding::TotalsMismatch { field, .. } => Some(*field),
            _ => None,
        })
        .collect();
    assert_eq!(fields, [TotalField::Net, TotalField::Vat, TotalField::Gross]);
}

#[test]
fn a_difference_of_one_cent_is_still_reported() {
    // Deliberately exact: a flagged cent is a review task, a silent one is not.
    let mut x = clean();
    x.stated_totals.as_mut().unwrap().value.vat = zar(225_001);
    x.stated_totals.as_mut().unwrap().value.gross = zar(1_825_001);
    assert!(!validate(&x).is_clean());
}

#[test]
fn missing_stated_totals_are_not_a_finding() {
    let mut x = clean();
    x.stated_totals = None;
    let result = validate(&x);
    assert_eq!(result.findings, []);
    assert_eq!(result.recomputed, Some(correct_totals()));
}

// ---------- dates ----------

#[test]
fn a_due_date_before_the_issue_date_is_reported() {
    let mut x = clean();
    x.due.as_mut().unwrap().value.due = d!(2026 - 03 - 30);
    assert_eq!(validate(&x).findings, [Finding::DueBeforeIssue]);
}

#[test]
fn a_due_date_on_the_issue_date_is_fine() {
    let mut x = clean();
    x.due.as_mut().unwrap().value.due = d!(2026 - 03 - 31);
    assert_eq!(validate(&x).findings, []);
}

// ---------- South African field rules ----------

#[test]
fn missing_required_fields_are_reported_using_the_za_rules() {
    let mut x = clean();
    x.invoice_number = None;
    x.invoice_date = None;
    x.due = None;
    x.supplier.as_mut().unwrap().value.address = None;
    let fields: Vec<&str> = validate(&x)
        .findings
        .iter()
        .filter_map(|f| match f {
            Finding::Compliance(i) => Some(i.field),
            _ => None,
        })
        .collect();
    assert_eq!(fields, ["supplier_address", "serial_number", "date_of_issue"]);
}

#[test]
fn a_full_invoice_needs_the_recipient_details() {
    let mut x = clean();
    x.recipient = None;
    let result = validate(&x);
    assert!(result.findings.contains(&issue(
        "recipient_name",
        "recipient name is required above R5 000"
    )));
    assert_eq!(result.findings.len(), 3);
}

#[test]
fn an_invalid_supplier_vat_number_is_reported() {
    let mut x = clean();
    x.supplier.as_mut().unwrap().value.vat_number = Some("3123456789".into());
    assert_eq!(
        validate(&x).findings,
        [issue(
            "supplier_vat_number",
            "supplier VAT number must be 10 digits starting with 4"
        )]
    );
}

#[test]
fn the_threshold_uses_the_stated_gross_or_else_the_recomputed_one() {
    let mut x = clean();
    x.recipient = None;
    // Remove the stated totals: the recomputed gross (R18 250.00) still makes
    // this a full tax invoice, so the recipient is still required.
    x.stated_totals = None;
    assert_eq!(validate(&x).findings.len(), 3);
    // A small invoice (R10.00) needs no recipient and no formal invoice at all.
    x.lines = vec![line("Sweets", 1_000, 1_000, 1_000, TaxCategory::ZeroRated)];
    x.supplier = None;
    assert_eq!(validate(&x).findings, []);
}

#[test]
fn rand_thresholds_are_not_applied_to_other_currencies() {
    // The same R18 250.00-sized invoice in dollars: the thresholds are in rand,
    // so a missing recipient and address are not judged against them.
    let usd = |minor| Money::new(minor, Currency::Usd);
    let mut x = clean();
    x.currency = Currency::Usd;
    for l in &mut x.lines {
        l.value.unit_price = usd(l.value.unit_price.minor);
        l.value.line_total = usd(l.value.line_total.minor);
    }
    x.stated_totals = Some(at(InvoiceTotals {
        net: usd(1_600_000),
        vat: usd(225_000),
        gross: usd(1_825_000),
    }));
    x.recipient = None;
    x.supplier.as_mut().unwrap().value.address = None;
    assert_eq!(validate(&x).findings, []);
}

#[test]
fn compliance_is_not_judged_when_the_amount_is_unknown() {
    let mut x = clean();
    x.lines.clear();
    x.stated_totals = None;
    x.supplier = None;
    x.recipient = None;
    assert_eq!(validate(&x).findings, [Finding::NoLines]);
}

// ---------- ordering and shape ----------

#[test]
fn findings_come_in_a_fixed_order() {
    let mut x = clean();
    x.lines[0].value.line_total = zar(1); // line mismatch
    x.stated_totals.as_mut().unwrap().value.net = zar(5); // totals mismatch
    x.due.as_mut().unwrap().value.due = d!(2026 - 03 - 01); // due before issue
    x.supplier.as_mut().unwrap().value.address = None; // compliance
    let kinds: Vec<&str> = validate(&x)
        .findings
        .iter()
        .map(|f| match f {
            Finding::LineTotalMismatch { .. } => "line",
            Finding::TotalsMismatch { .. } => "totals",
            Finding::DueBeforeIssue => "due",
            Finding::Compliance(_) => "compliance",
            _ => "other",
        })
        .collect();
    // net is wrong, and gross is not, but vat/gross follow the recomputed values:
    assert_eq!(kinds, ["line", "totals", "due", "compliance"]);
}

// ---------- JSON ----------

#[test]
fn an_extraction_round_trips_through_json() {
    let x = clean();
    let text = serde_json::to_string(&x).unwrap();
    assert_eq!(serde_json::from_str::<Extraction>(&text).unwrap(), x);
}

#[test]
fn only_the_document_extractor_and_currency_are_required_in_json() {
    let minimal = json!({
        "document": "doc_7",
        "extractor": { "type": "model", "name": "claude-haiku-5-5", "prompt_version": "v1" },
        "currency": "ZAR"
    });
    let x: Extraction = serde_json::from_value(minimal).unwrap();
    assert!(x.lines.is_empty());
    assert!(x.supplier.is_none() && x.stated_totals.is_none());
}

#[test]
fn a_sourced_value_carries_its_page_and_confidence_in_json() {
    let value = serde_json::to_value(at("INV-1".to_string())).unwrap();
    assert_eq!(value, json!({ "value": "INV-1", "page": 1, "confidence": 950 }));
    let with_locator = Sourced {
        locator: Some(Locator::TextSpan { start: 3, end: 9 }),
        ..at(1_i64)
    };
    let value = serde_json::to_value(with_locator).unwrap();
    assert_eq!(
        value["locator"],
        json!({ "type": "text_span", "start": 3, "end": 9 })
    );
}

#[test]
fn json_from_an_extractor_is_held_to_the_page_and_confidence_rules() {
    let bad_page = json!({ "value": "x", "page": 0, "confidence": 900 });
    assert!(serde_json::from_value::<Sourced<String>>(bad_page).is_err());
    let bad_confidence = json!({ "value": "x", "page": 1, "confidence": 1001 });
    assert!(serde_json::from_value::<Sourced<String>>(bad_confidence).is_err());
}

#[test]
fn a_sourced_value_makes_a_citation_for_a_document() {
    let sourced = Sourced {
        locator: Some(Locator::TextSpan { start: 1, end: 2 }),
        page: NonZeroU32::new(4).unwrap(),
        ..at(0_i64)
    };
    let citation = sourced.citation(&DocumentId::new("doc_1"));
    assert_eq!(citation.document.as_str(), "doc_1");
    assert_eq!(citation.page.get(), 4);
    assert_eq!(citation.locator, Some(Locator::TextSpan { start: 1, end: 2 }));
}
