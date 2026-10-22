// Written by Kone Tshivhinda & Claude
use super::*;
use clearcut_core::event::{Confidence, Extractor};
use clearcut_core::extraction::validate;
use clearcut_core::{Currency, Money, TaxCategory};
use serde_json::{json, Value};

/// A correct label: one line, a supplier and recipient with every field a
/// full tax invoice needs, and totals that add up. (10 x R1500.00 = R15000.00,
/// 15% VAT = R2250.00.)
pub(crate) fn base() -> Value {
    json!({
        "format": 1,
        "case": "g-0001",
        "document": { "file": "document.pdf", "sha256": "a".repeat(64), "pages": 2 },
        "tags": ["text-layer", "multi-page", "tax-invoice"],
        "split": "dev",
        "provenance": {
            "tier": "consented",
            "consent_ref": "C-017",
            "anonymised": true,
            "retain_until": "2031-03-31",
            "labelled_by": "KT",
            "reviewed_by": null
        },
        "truth": {
            "currency": "ZAR",
            "invoice_number": { "value": "INV-000123", "page": 1 },
            "invoice_date": { "value": "2026-03-31", "page": 1 },
            "supplier": { "value": {
                "name": "Acme Advisory (Pty) Ltd", "vat_number": "4999900001", "address": "1 Main Rd, Rosebank"
            }, "page": 1 },
            "recipient": { "value": {
                "name": "Client Co", "vat_number": "4999900002", "address": "2 Side St, Sandton"
            }, "page": 1 },
            "lines": [{
                "page": 1, "description": "Consulting", "quantity": "10", "unit_price": "1500.00",
                "line_total": "15000.00", "tax": "standard"
            }],
            "stated_totals": { "net": "15000.00", "vat": "2250.00", "gross": "17250.00", "page": 2 },
            "due": { "date": "2026-04-30", "terms": "30 days", "page": 2 }
        },
        "expect": { "findings": [] }
    })
}

fn label_of(value: &Value) -> Label {
    Label::parse(value.to_string().as_bytes()).expect("a valid label")
}

fn parse_error(value: &Value) -> ParseError {
    Label::parse(value.to_string().as_bytes()).expect_err("an invalid label")
}

/// `base()` with the JSON pointer `path` set to `new`.
fn with(path: &str, new: Value) -> Value {
    let mut value = base();
    *value.pointer_mut(path).expect("path exists") = new;
    value
}

fn without(path: &str) -> Value {
    let mut value = base();
    let (parent, key) = path.rsplit_once('/').expect("a path");
    value
        .pointer_mut(parent)
        .expect("parent exists")
        .as_object_mut()
        .expect("an object")
        .remove(key);
    value
}

#[test]
fn the_example_label_parses() {
    let label = label_of(&base());
    assert_eq!(label.format, 1);
    assert_eq!(label.case, "g-0001");
    assert_eq!(label.document.file, "document.pdf");
    assert_eq!(label.document.pages, 2);
    assert_eq!(label.split, Split::Dev);
    assert_eq!(label.provenance.tier, Tier::Consented);
    assert_eq!(label.tags, ["text-layer", "multi-page", "tax-invoice"]);
    assert!(label.expect.findings.is_empty());
    assert!(label.problems().is_empty(), "{:?}", label.problems());
}

#[test]
fn expect_is_optional_and_defaults_to_no_findings() {
    let label = label_of(&without("/expect"));
    assert!(label.expect.findings.is_empty());
    let label = label_of(&without("/tags"));
    assert!(label.tags.is_empty());
}

#[test]
fn a_label_survives_a_round_trip_through_json() {
    let label = label_of(&base());
    let again = Label::parse(&serde_json::to_vec(&label).unwrap()).unwrap();
    assert_eq!(label, again);
}

#[test]
fn unknown_and_missing_fields_are_rejected() {
    // A typo in a field name must not silently drop the value.
    for path in [
        "/truth/currency_code",
        "/split_by",
        "/provenance/consent",
        "/document/path",
    ] {
        let (parent, key) = path.rsplit_once('/').unwrap();
        let mut value = base();
        let object = if parent.is_empty() {
            &mut value
        } else {
            value.pointer_mut(parent).unwrap()
        };
        object.as_object_mut().unwrap().insert(key.to_string(), json!(1));
        assert!(
            matches!(parse_error(&value), ParseError::WrongShape { .. }),
            "{path}"
        );
    }
    for path in [
        "/case",
        "/document",
        "/split",
        "/provenance/tier",
        "/truth",
        "/truth/currency",
        "/format",
    ] {
        assert!(
            matches!(parse_error(&without(path)), ParseError::WrongShape { .. }),
            "{path}"
        );
    }
    // Inside a line and inside a citation.
    let mut value = base();
    value
        .pointer_mut("/truth/lines/0")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .remove("page");
    assert!(matches!(parse_error(&value), ParseError::WrongShape { .. }));
    let mut value = base();
    value
        .pointer_mut("/truth/invoice_number")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .insert("extra".into(), json!(1));
    assert!(matches!(parse_error(&value), ParseError::WrongShape { .. }));
}

#[test]
fn page_zero_a_bad_date_and_a_bad_tax_category_are_rejected_when_parsing() {
    assert!(matches!(
        parse_error(&with("/truth/invoice_number/page", json!(0))),
        ParseError::WrongShape { .. }
    ));
    assert!(matches!(
        parse_error(&with("/truth/invoice_date/value", json!("31/03/2026"))),
        ParseError::WrongShape { .. }
    ));
    assert!(matches!(
        parse_error(&with("/truth/invoice_date/value", json!("2026-02-30"))),
        ParseError::WrongShape { .. }
    ));
    assert!(matches!(
        parse_error(&with("/provenance/retain_until", json!("soon"))),
        ParseError::WrongShape { .. }
    ));
    assert!(matches!(
        parse_error(&with("/truth/lines/0/tax", json!("fifteen"))),
        ParseError::WrongShape { .. }
    ));
    assert!(matches!(
        parse_error(&with("/split", json!("train"))),
        ParseError::WrongShape { .. }
    ));
    assert!(matches!(
        parse_error(&with("/truth/currency", json!("zar"))),
        ParseError::WrongShape { .. }
    ));
}

#[test]
fn amounts_must_be_text_not_numbers() {
    // A JSON number would pass through a float parser on the way in.
    assert!(matches!(
        parse_error(&with("/truth/lines/0/unit_price", json!(1500.0))),
        ParseError::WrongShape { .. }
    ));
    assert!(matches!(
        parse_error(&with("/truth/stated_totals/net", json!(15000))),
        ParseError::WrongShape { .. }
    ));
}

#[test]
fn not_json_is_reported_as_not_json() {
    assert!(matches!(Label::parse(b"{ nope"), Err(ParseError::NotJson { .. })));
    assert!(matches!(Label::parse(b""), Err(ParseError::NotJson { .. })));
    assert!(matches!(
        Label::parse(&[0xff, 0xfe]),
        Err(ParseError::NotJson { .. })
    ));
}

#[test]
fn parse_errors_say_where_but_never_echo_the_file() {
    let mut value = base();
    *value.pointer_mut("/truth/supplier/value/name").unwrap() = json!(12345);
    let pretty = serde_json::to_string_pretty(&value).unwrap();
    let error = Label::parse(pretty.as_bytes()).unwrap_err();
    let text = error.to_string();
    assert!(text.contains("line"), "{text}");
    for secret in ["12345", "Acme", "Rosebank", "supplier", "name"] {
        assert!(!text.contains(secret), "{text}");
    }
    let broken = pretty.replace("\"Acme Advisory (Pty) Ltd\"", "Acme Advisory secret");
    let text = Label::parse(broken.as_bytes()).unwrap_err().to_string();
    assert!(!text.contains("secret") && !text.contains("Acme"), "{text}");
}

#[test]
fn the_truth_converts_to_exactly_the_extraction_the_system_would_produce() {
    let label = label_of(&base());
    let extraction = label
        .truth
        .to_extraction(&label.case, &label.provenance.labelled_by)
        .unwrap();

    assert_eq!(extraction.document.as_str(), "g-0001");
    assert_eq!(extraction.extractor, Extractor::Human { user: "KT".into() });
    assert_eq!(extraction.currency, Currency::Zar);

    let number = extraction.invoice_number.as_ref().unwrap();
    assert_eq!(number.value, "INV-000123");
    assert_eq!(number.page.get(), 1);
    assert_eq!(number.confidence, Confidence::CERTAIN);
    assert!(number.locator.is_none());

    assert_eq!(
        extraction.invoice_date.as_ref().unwrap().value.to_string(),
        "2026-03-31"
    );
    let supplier = &extraction.supplier.as_ref().unwrap().value;
    assert_eq!(supplier.name, "Acme Advisory (Pty) Ltd");
    assert_eq!(supplier.vat_number.as_deref(), Some("4999900001"));
    assert_eq!(supplier.address.as_deref(), Some("1 Main Rd, Rosebank"));

    let line = &extraction.lines[0];
    assert_eq!(line.page.get(), 1);
    assert_eq!(line.value.description, "Consulting");
    assert_eq!(line.value.quantity_milli, 10_000);
    assert_eq!(line.value.unit_price, Money::zar(150_000));
    assert_eq!(line.value.line_total, Money::zar(1_500_000));
    assert_eq!(line.value.tax, TaxCategory::Standard);
    assert_eq!(line.confidence, Confidence::CERTAIN);

    let totals = extraction.stated_totals.as_ref().unwrap();
    assert_eq!(totals.page.get(), 2);
    assert_eq!(totals.value.net, Money::zar(1_500_000));
    assert_eq!(totals.value.vat, Money::zar(225_000));
    assert_eq!(totals.value.gross, Money::zar(1_725_000));

    let due = extraction.due.as_ref().unwrap();
    assert_eq!(due.value.due.to_string(), "2026-04-30");
    assert_eq!(due.value.terms.as_deref(), Some("30 days"));
    assert_eq!(due.page.get(), 2);
    assert!(extraction.credit_note.is_none());
}

#[test]
fn a_correct_example_passes_the_systems_own_validation_cleanly() {
    let label = label_of(&base());
    let extraction = label.truth.to_extraction(&label.case, "KT").unwrap();
    let validation = validate(&extraction);
    assert!(validation.is_clean(), "{:?}", validation.findings);
}

#[test]
fn optional_parts_may_be_absent() {
    let mut value = base();
    let truth = value.pointer_mut("/truth").unwrap().as_object_mut().unwrap();
    for key in [
        "invoice_number",
        "invoice_date",
        "supplier",
        "recipient",
        "stated_totals",
        "due",
    ] {
        truth.remove(key);
    }
    let label = label_of(&value);
    let extraction = label.truth.to_extraction(&label.case, "KT").unwrap();
    assert!(extraction.invoice_number.is_none() && extraction.supplier.is_none() && extraction.due.is_none());
    assert_eq!(extraction.lines.len(), 1);
}

#[test]
fn a_credit_note_reference_converts() {
    let mut value = base();
    value["truth"]["credit_note"] =
        json!({ "original_invoice_number": "INV-000100", "amount": "115.00", "page": 1 });
    let label = label_of(&value);
    let extraction = label.truth.to_extraction(&label.case, "KT").unwrap();
    let note = extraction.credit_note.unwrap();
    assert_eq!(note.value.original_invoice_number, "INV-000100");
    assert_eq!(note.value.amount, Money::zar(11_500));
}

#[test]
fn money_in_another_currency_converts_with_that_currency() {
    let mut value = base();
    value["truth"]["currency"] = json!("USD");
    let label = label_of(&value);
    let extraction = label.truth.to_extraction(&label.case, "KT").unwrap();
    assert_eq!(
        extraction.lines[0].value.unit_price,
        Money::new(150_000, Currency::Usd)
    );
    assert_eq!(
        extraction.stated_totals.unwrap().value.gross.currency,
        Currency::Usd
    );
}

#[test]
fn a_bad_amount_is_reported_with_its_field_but_not_its_text() {
    let cases = [
        ("/truth/lines/0/unit_price", "lines[0].unit_price", "1,500.00"),
        ("/truth/lines/0/quantity", "lines[0].quantity", "1.0005"),
        ("/truth/lines/0/line_total", "lines[0].line_total", "R15000"),
        ("/truth/stated_totals/net", "stated_totals.net", "15000.001"),
        ("/truth/stated_totals/vat", "stated_totals.vat", ""),
        ("/truth/stated_totals/gross", "stated_totals.gross", "-0"),
    ];
    for (pointer, field, text) in cases {
        let label = label_of(&with(pointer, json!(text)));
        let error = label.truth.to_extraction(&label.case, "KT").unwrap_err();
        assert_eq!(error.field, field, "{pointer}");
        assert!(matches!(error.kind, TruthErrorKind::Decimal(_)));
        assert!(!error.to_string().contains(text) || text.is_empty(), "{error}");
        assert!(error.to_string().contains(field), "{error}");
    }
}

#[test]
fn the_line_number_in_the_field_is_the_right_one() {
    let mut value = base();
    let line = value["truth"]["lines"][0].clone();
    value["truth"]["lines"] = json!([line.clone(), line.clone(), line]);
    value["truth"]["lines"][2]["unit_price"] = json!("oops");
    let label = label_of(&value);
    let error = label.truth.to_extraction(&label.case, "KT").unwrap_err();
    assert_eq!(error.field, "lines[2].unit_price");
}

#[test]
fn a_credit_note_amount_is_checked_too() {
    let mut value = base();
    value["truth"]["credit_note"] =
        json!({ "original_invoice_number": "INV-1", "amount": "1.234", "page": 1 });
    let label = label_of(&value);
    assert_eq!(
        label.truth.to_extraction(&label.case, "KT").unwrap_err().field,
        "credit_note.amount"
    );
}

#[test]
fn empty_names_numbers_and_users_are_rejected_when_converting() {
    for (pointer, field) in [
        ("/truth/invoice_number/value", "invoice_number"),
        ("/truth/supplier/value/name", "supplier.name"),
        ("/truth/recipient/value/name", "recipient.name"),
    ] {
        for blank in ["", "   "] {
            let label = label_of(&with(pointer, json!(blank)));
            let error = label.truth.to_extraction(&label.case, "KT").unwrap_err();
            assert_eq!(error.field, field);
            assert_eq!(error.kind, TruthErrorKind::Empty);
        }
    }
}

#[test]
fn cited_pages_lists_every_page_the_truth_points_at() {
    let mut value = base();
    value["truth"]["credit_note"] =
        json!({ "original_invoice_number": "INV-1", "amount": "1.00", "page": 2 });
    let label = label_of(&value);
    let pages = label.truth.cited_pages();
    let names: Vec<&str> = pages.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(
        names,
        [
            "invoice_number",
            "invoice_date",
            "supplier",
            "recipient",
            "lines[0]",
            "stated_totals",
            "due",
            "credit_note"
        ]
    );
    assert_eq!(
        pages.iter().map(|(_, page)| *page).collect::<Vec<_>>(),
        [1, 1, 1, 1, 1, 2, 2, 2]
    );
}

// ---- problems(): what is wrong with a label that parsed ----

fn problems_of(value: &Value) -> Vec<LabelProblem> {
    label_of(value).problems()
}

#[test]
fn only_format_one_is_supported() {
    assert_eq!(
        problems_of(&with("/format", json!(2))),
        [LabelProblem::UnsupportedFormat]
    );
    assert_eq!(
        problems_of(&with("/format", json!(0))),
        [LabelProblem::UnsupportedFormat]
    );
}

#[test]
fn case_ids_have_one_shape() {
    for ok in ["g-0001", "g-12345", "g-0000"] {
        assert!(problems_of(&with("/case", json!(ok))).is_empty(), "{ok}");
    }
    for bad in [
        "",
        "G-0001",
        "g-001",
        "g0001",
        "g-00a1",
        "g-0001 ",
        "acme-0001",
        "g-0001/..",
        "../g-0001",
    ] {
        assert_eq!(
            problems_of(&with("/case", json!(bad))),
            [LabelProblem::BadCaseId],
            "{bad:?}"
        );
    }
}

#[test]
fn the_document_file_name_must_be_a_plain_file_name() {
    for ok in ["document.pdf", "invoice-1.PDF", "scan_02.jpg"] {
        assert!(problems_of(&with("/document/file", json!(ok))).is_empty(), "{ok}");
    }
    for bad in [
        "",
        ".",
        "..",
        "../x.pdf",
        "a/b.pdf",
        "a\\b.pdf",
        "C:x.pdf",
        "x.pdf:stream",
        "label.json",
        ".hidden.pdf",
        "x\u{0}.pdf",
        "with space .pdf ",
    ] {
        assert_eq!(
            problems_of(&with("/document/file", json!(bad))),
            [LabelProblem::BadFileName],
            "{bad:?}"
        );
    }
    assert!(problems_of(&with("/document/file", json!("x".repeat(255)))).is_empty());
    for long in [256, 300] {
        assert_eq!(
            problems_of(&with("/document/file", json!("x".repeat(long)))),
            [LabelProblem::BadFileName]
        );
    }
}

#[test]
fn the_hash_must_be_64_lowercase_hex_digits() {
    assert!(problems_of(&with("/document/sha256", json!("0123456789abcdef".repeat(4)))).is_empty());
    for bad in [
        "",
        "abc",
        "A".repeat(64).as_str(),
        "g".repeat(64).as_str(),
        "a".repeat(63).as_str(),
        "a".repeat(65).as_str(),
    ] {
        assert_eq!(
            problems_of(&with("/document/sha256", json!(bad))),
            [LabelProblem::BadSha256]
        );
    }
}

#[test]
fn the_page_count_is_between_one_and_ten_thousand() {
    assert!(!problems_of(&with("/document/pages", json!(1))).contains(&LabelProblem::PagesOutOfRange));
    assert!(problems_of(&with("/document/pages", json!(10_000))).is_empty());
    assert_eq!(
        problems_of(&with("/document/pages", json!(0))),
        [LabelProblem::PagesOutOfRange]
    );
    assert_eq!(
        problems_of(&with("/document/pages", json!(10_001))),
        [LabelProblem::PagesOutOfRange]
    );
}

#[test]
fn tags_are_short_lowercase_words_and_not_repeated() {
    assert!(problems_of(&with(
        "/tags",
        json!(["a", "scan", "credit-note", "x1", "a".repeat(32)])
    ))
    .is_empty());
    for bad in [
        "",
        "Scan",
        "two words",
        "-lead",
        "trail-",
        "under_score",
        "é",
        &"a".repeat(33),
    ] {
        assert_eq!(
            problems_of(&with("/tags", json!(["ok", bad]))),
            [LabelProblem::BadTag { index: 1 }],
            "{bad:?}"
        );
    }
    assert_eq!(
        problems_of(&with("/tags", json!(["scan", "photo", "scan"]))),
        [LabelProblem::DuplicateTag { index: 2 }]
    );
}

#[test]
fn pages_cited_must_exist_in_the_document() {
    let problems = problems_of(&with("/document/pages", json!(1)));
    assert_eq!(
        problems,
        [
            LabelProblem::PageOutsideDocument {
                field: "stated_totals".into(),
                page: 2
            },
            LabelProblem::PageOutsideDocument {
                field: "due".into(),
                page: 2
            },
        ]
    );
    assert!(problems_of(&with("/document/pages", json!(2))).is_empty());
}

#[test]
fn real_documents_need_consent_a_retention_date_and_anonymisation() {
    for tier in ["own", "consented"] {
        let mut value = with("/provenance/tier", json!(tier));
        assert!(problems_of(&value).is_empty());

        value["provenance"]["consent_ref"] = Value::Null;
        assert_eq!(problems_of(&value), [LabelProblem::ConsentMissing]);
        value["provenance"]["consent_ref"] = json!("  ");
        assert_eq!(problems_of(&value), [LabelProblem::ConsentMissing]);
        value["provenance"]["consent_ref"] = json!("C-1");

        value["provenance"]["retain_until"] = Value::Null;
        assert_eq!(problems_of(&value), [LabelProblem::RetentionMissing]);
        value["provenance"]["retain_until"] = json!("2031-03-31");

        value["provenance"]["anonymised"] = json!(false);
        assert_eq!(problems_of(&value), [LabelProblem::NotAnonymised]);
    }
}

#[test]
fn synthetic_documents_need_none_of_that() {
    let mut value = with("/provenance/tier", json!("synthetic"));
    value["provenance"]["consent_ref"] = Value::Null;
    value["provenance"]["retain_until"] = Value::Null;
    value["provenance"]["anonymised"] = json!(false);
    assert!(problems_of(&value).is_empty());
}

#[test]
fn the_labeller_and_a_named_reviewer_must_not_be_blank() {
    assert_eq!(
        problems_of(&with("/provenance/labelled_by", json!(" "))),
        [LabelProblem::LabellerMissing]
    );
    for blank in ["", " ", "	"] {
        assert_eq!(
            problems_of(&with("/provenance/reviewed_by", json!(blank))),
            [LabelProblem::ReviewerBlank]
        );
    }
    assert!(problems_of(&with("/provenance/reviewed_by", json!("AB"))).is_empty());
}

#[test]
fn every_problem_in_a_label_is_reported_together() {
    let mut value = base();
    value["format"] = json!(3);
    value["case"] = json!("bad");
    value["document"]["sha256"] = json!("zz");
    let problems = problems_of(&value);
    assert_eq!(problems.len(), 3, "{problems:?}");
}

#[test]
fn expected_findings_must_be_well_formed_keys() {
    let good = [
        "no_lines",
        "totals_out_of_range",
        "due_before_issue",
        "currency_mismatch:1",
        "non_positive_quantity:12",
        "amount_out_of_range:3",
        "line_total_mismatch:2",
        "totals_mismatch:net",
        "totals_mismatch:vat",
        "totals_mismatch:gross",
        "compliance:supplier_address",
    ];
    assert!(problems_of(&with("/expect/findings", json!(good))).is_empty());
    for bad in [
        "",
        "Acme Corp owes 5000",
        "no_lines ",
        "line_total_mismatch",
        "line_total_mismatch:",
        "line_total_mismatch:x",
        "line_total_mismatch:12345678901",
        "totals_mismatch:tax",
        "compliance:",
        "compliance:Supplier",
        "compliance:a b",
        "unknown_kind",
        "no_lines:1",
        "0821234567",
    ] {
        assert_eq!(
            problems_of(&with("/expect/findings", json!(["no_lines", bad]))),
            [LabelProblem::BadExpectedFinding { index: 1 }],
            "{bad:?}"
        );
    }
}
