// Written by Kone Tshivhinda & Claude
use super::*;
use crate::label::tests::base;
use crate::label::Label;
use clearcut_core::event::{Confidence, Extractor};
use clearcut_core::{Currency, Money, TaxCategory};
use time::macros::date;

/// The truth for the example label: a document with every kind of field.
fn truth() -> Extraction {
    let label = Label::parse(base().to_string().as_bytes()).unwrap();
    label.truth.to_extraction(&label.case, "KT").unwrap()
}

/// A perfect copy of the truth, as a system would report it.
fn perfect() -> Extraction {
    let mut result = truth();
    result.extractor = Extractor::Model {
        name: "m".into(),
        prompt_version: "p".into(),
    };
    result
}

fn verdict_of(compared: &[Compared], field: &str) -> Verdict {
    compared
        .iter()
        .find(|c| c.field.to_string() == field)
        .unwrap_or_else(|| panic!("no field {field}"))
        .verdict
}

const ALL_FIELDS: [&str; 19] = [
    "currency",
    "invoice_number",
    "invoice_date",
    "supplier.name",
    "supplier.vat_number",
    "supplier.address",
    "recipient.name",
    "recipient.vat_number",
    "recipient.address",
    "lines[0].quantity",
    "lines[0].unit_price",
    "lines[0].line_total",
    "lines[0].description",
    "lines[0].tax",
    "totals.net",
    "totals.vat",
    "totals.gross",
    "due.date",
    "due.terms",
];

#[test]
fn a_perfect_copy_is_correct_in_every_field_and_cites_the_right_pages() {
    let result = perfect();
    let compared = compare(&truth(), Some(&result));
    let names: Vec<String> = compared.iter().map(|c| c.field.to_string()).collect();
    assert_eq!(names, ALL_FIELDS);
    for c in &compared {
        assert_eq!(c.verdict, Verdict::Correct, "{}", c.field);
        // Every field but the currency comes from a page.
        assert_eq!(
            c.page_correct,
            if c.field == Field::Currency {
                None
            } else {
                Some(true)
            },
            "{}",
            c.field
        );
    }
}

#[test]
fn nothing_extracted_means_every_field_is_missing() {
    let compared = compare(&truth(), None);
    assert_eq!(compared.len(), ALL_FIELDS.len());
    for (c, name) in compared.iter().zip(ALL_FIELDS) {
        assert_eq!(c.field.to_string(), name);
        assert_eq!(c.verdict, Verdict::Missing, "{name}");
        assert_eq!(c.confidence, None);
        assert_eq!(c.page_correct, None);
    }
}

#[test]
fn a_changed_value_is_wrong_and_only_that_field() {
    let mut result = perfect();
    result.invoice_number.as_mut().unwrap().value = "INV-000124".into();
    let compared = compare(&truth(), Some(&result));
    for c in &compared {
        let expected = if c.field == Field::InvoiceNumber {
            Verdict::Wrong
        } else {
            Verdict::Correct
        };
        assert_eq!(c.verdict, expected, "{}", c.field);
    }
    // A wrong value does not get a page verdict.
    assert_eq!(
        compared
            .iter()
            .find(|c| c.field == Field::InvoiceNumber)
            .unwrap()
            .page_correct,
        None
    );
}

#[test]
fn names_addresses_descriptions_and_terms_ignore_case_and_spacing() {
    let mut result = perfect();
    result.supplier.as_mut().unwrap().value.name = "  ACME   advisory (PTY)\tLtd ".into();
    result.supplier.as_mut().unwrap().value.address = Some("1  main rd,  ROSEBANK".into());
    result.recipient.as_mut().unwrap().value.name = "client CO".into();
    result.lines[0].value.description = "CONSULTING ".into();
    result.due.as_mut().unwrap().value.terms = Some("30   Days".into());
    for c in compare(&truth(), Some(&result)) {
        assert_eq!(c.verdict, Verdict::Correct, "{}", c.field);
    }
}

#[test]
fn ids_ignore_surrounding_spaces_but_nothing_else() {
    let mut result = perfect();
    result.invoice_number.as_mut().unwrap().value = "  INV-000123\n".into();
    result.supplier.as_mut().unwrap().value.vat_number = Some(" 4999900001 ".into());
    let compared = compare(&truth(), Some(&result));
    assert_eq!(verdict_of(&compared, "invoice_number"), Verdict::Correct);
    assert_eq!(verdict_of(&compared, "supplier.vat_number"), Verdict::Correct);

    for wrong in ["inv-000123", "INV 000123", "INV-00123", "INV-0001230"] {
        let mut result = perfect();
        result.invoice_number.as_mut().unwrap().value = wrong.into();
        assert_eq!(
            verdict_of(&compare(&truth(), Some(&result)), "invoice_number"),
            Verdict::Wrong,
            "{wrong}"
        );
    }
    let mut result = perfect();
    result.supplier.as_mut().unwrap().value.vat_number = Some("4999 900001".into());
    assert_eq!(
        verdict_of(&compare(&truth(), Some(&result)), "supplier.vat_number"),
        Verdict::Wrong
    );
}

#[test]
fn a_different_letter_in_a_name_is_wrong() {
    let mut result = perfect();
    result.supplier.as_mut().unwrap().value.name = "Acme Advisory (Pty) Ltt".into();
    assert_eq!(
        verdict_of(&compare(&truth(), Some(&result)), "supplier.name"),
        Verdict::Wrong
    );
}

/// A change to a result, applied to a perfect copy of the truth.
type Change = Box<dyn Fn(&mut Extraction)>;

#[test]
fn amounts_quantities_dates_currency_and_tax_are_exact() {
    let cases: Vec<(&str, Change)> = vec![
        (
            "lines[0].unit_price",
            Box::new(|r| r.lines[0].value.unit_price = Money::zar(150_001)),
        ),
        (
            "lines[0].unit_price",
            Box::new(|r| r.lines[0].value.unit_price = Money::new(150_000, Currency::Usd)),
        ),
        (
            "lines[0].line_total",
            Box::new(|r| r.lines[0].value.line_total = Money::zar(1_499_999)),
        ),
        (
            "lines[0].quantity",
            Box::new(|r| r.lines[0].value.quantity_milli = 10_001),
        ),
        (
            "lines[0].tax",
            Box::new(|r| r.lines[0].value.tax = TaxCategory::ZeroRated),
        ),
        (
            "totals.net",
            Box::new(|r| r.stated_totals.as_mut().unwrap().value.net = Money::zar(1)),
        ),
        (
            "totals.vat",
            Box::new(|r| r.stated_totals.as_mut().unwrap().value.vat = Money::zar(225_001)),
        ),
        (
            "totals.gross",
            Box::new(|r| r.stated_totals.as_mut().unwrap().value.gross = Money::zar(0)),
        ),
        (
            "invoice_date",
            Box::new(|r| r.invoice_date.as_mut().unwrap().value = date!(2026 - 04 - 01)),
        ),
        (
            "due.date",
            Box::new(|r| r.due.as_mut().unwrap().value.due = date!(2026 - 05 - 01)),
        ),
        ("currency", Box::new(|r| r.currency = Currency::Usd)),
    ];
    for (field, change) in cases {
        let mut result = perfect();
        change(&mut result);
        let compared = compare(&truth(), Some(&result));
        assert_eq!(verdict_of(&compared, field), Verdict::Wrong, "{field}");
        let wrong: Vec<_> = compared
            .iter()
            .filter(|c| c.verdict != Verdict::Correct)
            .collect();
        assert_eq!(wrong.len(), 1, "{field}: {wrong:?}");
    }
}

#[test]
fn a_value_left_out_is_missing_and_an_invented_one_is_spurious() {
    // Left out: an optional part of a party, the due terms, the totals.
    let mut result = perfect();
    result.supplier.as_mut().unwrap().value.address = None;
    result.supplier.as_mut().unwrap().value.vat_number = None;
    result.due.as_mut().unwrap().value.terms = None;
    result.stated_totals = None;
    let compared = compare(&truth(), Some(&result));
    for field in [
        "supplier.address",
        "supplier.vat_number",
        "due.terms",
        "totals.net",
        "totals.vat",
        "totals.gross",
    ] {
        assert_eq!(verdict_of(&compared, field), Verdict::Missing, "{field}");
    }
    assert_eq!(
        compared.iter().filter(|c| c.verdict == Verdict::Missing).count(),
        6
    );
    assert_eq!(verdict_of(&compared, "supplier.name"), Verdict::Correct);

    // Invented: things the document does not have.
    let mut bare = truth();
    bare.recipient = None;
    bare.due = None;
    bare.supplier.as_mut().unwrap().value.address = None;
    let compared = compare(&bare, Some(&perfect()));
    for field in [
        "recipient.name",
        "recipient.vat_number",
        "recipient.address",
        "due.date",
        "due.terms",
        "supplier.address",
    ] {
        assert_eq!(verdict_of(&compared, field), Verdict::Spurious, "{field}");
    }
    assert_eq!(
        compared.iter().filter(|c| c.verdict == Verdict::Spurious).count(),
        6
    );
}

#[test]
fn a_part_neither_side_has_is_not_a_field_at_all() {
    let mut truth = truth();
    truth.recipient.as_mut().unwrap().value.address = None;
    let mut result = perfect();
    result.recipient.as_mut().unwrap().value.address = None;
    let compared = compare(&truth, Some(&result));
    assert!(!compared
        .iter()
        .any(|c| c.field.to_string() == "recipient.address"));
    assert_eq!(compared.len(), ALL_FIELDS.len() - 1);
}

#[test]
fn blank_text_counts_as_absent() {
    let mut result = perfect();
    result.supplier.as_mut().unwrap().value.address = Some("   ".into());
    result.due.as_mut().unwrap().value.terms = Some(String::new());
    let compared = compare(&truth(), Some(&result));
    assert_eq!(verdict_of(&compared, "supplier.address"), Verdict::Missing);
    assert_eq!(verdict_of(&compared, "due.terms"), Verdict::Missing);

    // Blank where the truth is absent is not an invention.
    let mut bare = truth();
    bare.supplier.as_mut().unwrap().value.address = None;
    let mut result = perfect();
    result.supplier.as_mut().unwrap().value.address = Some("  ".into());
    let compared = compare(&bare, Some(&result));
    assert!(!compared.iter().any(|c| c.field.to_string() == "supplier.address"));
}

#[test]
fn a_whole_missing_party_misses_each_of_its_parts() {
    let mut result = perfect();
    result.supplier = None;
    let compared = compare(&truth(), Some(&result));
    for field in ["supplier.name", "supplier.vat_number", "supplier.address"] {
        assert_eq!(verdict_of(&compared, field), Verdict::Missing, "{field}");
    }
}

#[test]
fn lines_are_matched_by_position() {
    let mut two = truth();
    let second = two.lines[0].clone();
    two.lines.push(second);

    // The system found only the first line: the second's five fields are missing.
    let mut one = perfect();
    one.lines.truncate(1);
    let compared = compare(&two, Some(&one));
    for part in ["quantity", "unit_price", "line_total", "description", "tax"] {
        assert_eq!(
            verdict_of(&compared, &format!("lines[0].{part}")),
            Verdict::Correct
        );
        assert_eq!(
            verdict_of(&compared, &format!("lines[1].{part}")),
            Verdict::Missing
        );
    }

    // The system invented a second line: its five fields are spurious.
    let mut invented = perfect();
    let extra = invented.lines[0].clone();
    invented.lines.push(extra);
    let compared = compare(&truth(), Some(&invented));
    for part in ["quantity", "unit_price", "line_total", "description", "tax"] {
        assert_eq!(
            verdict_of(&compared, &format!("lines[1].{part}")),
            Verdict::Spurious
        );
    }

    // No lines at all, on either side.
    let mut none = truth();
    none.lines.clear();
    let mut result = perfect();
    result.lines.clear();
    let compared = compare(&none, Some(&result));
    assert!(!compared.iter().any(|c| matches!(c.field, Field::Line(..))));
}

#[test]
fn confidence_is_the_results_and_only_when_it_gave_a_value() {
    let mut result = perfect();
    result.invoice_number.as_mut().unwrap().confidence = Confidence::new(420).unwrap();
    result.supplier.as_mut().unwrap().confidence = Confidence::new(610).unwrap();
    result.lines[0].confidence = Confidence::new(777).unwrap();
    result.supplier.as_mut().unwrap().value.address = None;
    let compared = compare(&truth(), Some(&result));
    let conf = |field: &str| {
        compared
            .iter()
            .find(|c| c.field.to_string() == field)
            .unwrap()
            .confidence
    };
    assert_eq!(conf("invoice_number"), Some(Confidence::new(420).unwrap()));
    assert_eq!(conf("supplier.name"), Some(Confidence::new(610).unwrap()));
    assert_eq!(conf("supplier.vat_number"), Some(Confidence::new(610).unwrap()));
    assert_eq!(conf("lines[0].tax"), Some(Confidence::new(777).unwrap()));
    assert_eq!(conf("totals.net"), Some(Confidence::CERTAIN));
    // Nothing was read for the address, so there is no confidence to speak of.
    assert_eq!(conf("supplier.address"), None);
    assert_eq!(conf("currency"), None);
}

#[test]
fn citing_the_wrong_page_is_noted_only_for_correct_values() {
    let mut result = perfect();
    result.invoice_number.as_mut().unwrap().page = std::num::NonZeroU32::new(2).unwrap();
    result.stated_totals.as_mut().unwrap().page = std::num::NonZeroU32::new(1).unwrap();
    let compared = compare(&truth(), Some(&result));
    let page = |field: &str| {
        compared
            .iter()
            .find(|c| c.field.to_string() == field)
            .unwrap()
            .page_correct
    };
    assert_eq!(page("invoice_number"), Some(false));
    assert_eq!(page("totals.gross"), Some(false));
    assert_eq!(page("invoice_date"), Some(true));
}

#[test]
fn a_credit_note_reference_is_compared() {
    let mut value = base();
    value["truth"]["credit_note"] =
        serde_json::json!({ "original_invoice_number": "INV-000100", "amount": "115.00", "page": 1 });
    let label = Label::parse(value.to_string().as_bytes()).unwrap();
    let truth = label.truth.to_extraction(&label.case, "KT").unwrap();

    let compared = compare(&truth, Some(&truth.clone()));
    assert_eq!(
        verdict_of(&compared, "credit_note.original_invoice_number"),
        Verdict::Correct
    );
    assert_eq!(verdict_of(&compared, "credit_note.amount"), Verdict::Correct);

    let mut result = truth.clone();
    result.credit_note.as_mut().unwrap().value.amount = Money::zar(11_501);
    result.credit_note.as_mut().unwrap().value.original_invoice_number = "inv-000100".into();
    let compared = compare(&truth, Some(&result));
    assert_eq!(verdict_of(&compared, "credit_note.amount"), Verdict::Wrong);
    assert_eq!(
        verdict_of(&compared, "credit_note.original_invoice_number"),
        Verdict::Wrong
    );

    result.credit_note = None;
    let compared = compare(&truth, Some(&result));
    assert_eq!(verdict_of(&compared, "credit_note.amount"), Verdict::Missing);
}

#[test]
fn comparing_never_mentions_values_in_what_it_returns() {
    // Compared holds a field, a verdict, a confidence and a page flag, nothing else.
    let mut result = perfect();
    result.supplier.as_mut().unwrap().value.name = "Secretname".into();
    let shown = format!("{:?}", compare(&truth(), Some(&result)));
    assert!(
        !shown.contains("Secretname") && !shown.contains("Acme"),
        "{shown}"
    );
}
