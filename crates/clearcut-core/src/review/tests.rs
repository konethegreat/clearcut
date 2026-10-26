// Written by Kone Tshivhinda & Claude
use super::*;
use crate::reading::ProblemKind;
use crate::TaxCategory;
use serde_json::json;

/// What the model read of an invoice that prints no tax: every line `not_shown`.
fn draft(tax: &str, gross: &str) -> PageReading {
    serde_json::from_value(json!({
        "currency": "USD",
        "amounts_include_vat": "no",
        "invoice_number": "INV-1",
        "invoice_date": "2026-10-01",
        "supplier": { "name": "Acme", "address": "", "vat_number": "" },
        "recipient": { "name": "", "address": "", "vat_number": "" },
        "lines": [
            { "description": "A", "quantity": "1", "unit_price": "400.00", "line_total": "400.00", "tax": tax },
            { "description": "B", "quantity": "1", "unit_price": "600.00", "line_total": "600.00", "tax": tax }
        ],
        "totals": { "net": "", "vat": "", "gross": gross },
        "due": { "date": "", "terms": "" },
        "credit_note": { "original_invoice_number": "", "amount": "" }
    }))
    .expect("a reading")
}

fn check(reading: &PageReading) -> Result<Extraction, Refusal> {
    check_confirmed(
        reading,
        DocumentId::new("doc_1"),
        NonZeroU32::MIN,
        "api_key:test",
        true,
    )
}

#[test]
fn a_person_can_supply_the_tax_the_page_did_not_show_and_it_is_recorded_as_theirs() {
    let extraction = check(&draft("exempt", "")).expect("confirmed");
    assert!(extraction
        .lines
        .iter()
        .all(|l| l.value.tax == TaxCategory::Exempt));
    let line = &extraction.lines[0];
    assert_eq!(
        line.confidence.get(),
        ConfidenceBasis::PersonConfirmed.per_mille()
    );
    assert_eq!(
        extraction.extractor,
        Extractor::Human {
            user: "api_key:test".into()
        }
    );
}

#[test]
fn a_page_is_refused_while_anything_is_still_wrong_and_the_refusal_says_where() {
    let refusal = check(&draft("not_shown", "")).expect_err("the tax is still missing");
    assert_eq!(refusal.problems.len(), 2);
    assert!(refusal
        .problems
        .iter()
        .all(|p| p.kind == ProblemKind::TaxNotShown));

    // Typing a different total does not get a page past the arithmetic.
    let mut wrong_total = draft("exempt", "");
    wrong_total.totals = crate::reading::ReadTotals {
        net: "1000.00".into(),
        vat: "0.00".into(),
        gross: "999.00".into(),
    };
    let refusal = check(&wrong_total).expect_err("the totals do not agree");
    assert!(!refusal.findings.is_empty());

    let mut no_currency = draft("exempt", "");
    no_currency.currency = crate::reading::ReadCurrency::NotShown;
    assert!(check(&no_currency).expect_err("no currency").currency_missing);
}
