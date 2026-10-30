// Written by Kone Tshivhinda & Claude
//! Pure tests: no database, no clock, no model.

use super::*;
use crate::event::{Citation, Confidence, DocumentId, Event, EventId, EventParts, EventPayload, Extractor};
use crate::Money;
use crate::TaxCategory;
use std::num::NonZeroU32;
use time::macros::date as d;

fn event(
    doc: &str,
    id: &str,
    date: Option<time::Date>,
    payload: EventPayload,
    supersedes: Option<&Event>,
) -> Event {
    Event::new(EventParts {
        id: EventId::new(format!("{doc}:{id}")),
        date,
        payload,
        citation: Citation {
            document: DocumentId::new(doc),
            page: NonZeroU32::MIN,
            locator: None,
        },
        extractor: Extractor::System,
        confidence: Confidence::CERTAIN,
        supersedes: supersedes.map(|earlier| earlier.id().clone()),
    })
    .unwrap()
}

fn zar(minor: i64) -> Money {
    Money::zar(minor)
}

fn facts(id: &str, status: DocumentStatus, events: Vec<Event>) -> RegisterFacts {
    RegisterFacts {
        id: id.into(),
        filename: format!("{id}.pdf"),
        uploaded_at: time::OffsetDateTime::UNIX_EPOCH,
        status,
        page_count: Some(1),
        pages: Vec::new(),
        events,
    }
}

#[test]
fn a_date_is_read_strictly() {
    assert_eq!(parse_date("2026-09-01"), Some(d!(2026 - 09 - 01)));
    assert_eq!(parse_date("2024-02-29"), Some(d!(2024 - 02 - 29)));
    for bad in [
        "2026-9-1",
        "2026-02-30",
        "2026/09/01",
        "",
        "2026-09-01T00:00",
        "٢٠٢٦-٠٩-٠١",
        "2026-09-0x",
    ] {
        assert_eq!(parse_date(bad), None, "{bad}");
    }
}

#[test]
fn a_selection_has_one_clear_json_shape() {
    let period = Selection::Period {
        from: d!(2026 - 09 - 01),
        to: d!(2026 - 09 - 30),
    };
    assert_eq!(
        serde_json::to_value(&period).unwrap(),
        serde_json::json!({ "type": "period", "from": "2026-09-01", "to": "2026-09-30" })
    );
    let chosen = Selection::Documents {
        ids: vec!["doc-a".into()],
    };
    assert_eq!(
        serde_json::to_value(&chosen).unwrap(),
        serde_json::json!({ "type": "documents", "ids": ["doc-a"] })
    );
}

#[test]
fn the_latest_current_fact_wins() {
    let old = event(
        "doc",
        "supplier-1",
        Some(d!(2026 - 09 - 05)),
        EventPayload::SupplierIdentified {
            name: "Old Name".into(),
            vat_number: None,
        },
        None,
    );
    let corrected = event(
        "doc",
        "supplier-2",
        Some(d!(2026 - 09 - 05)),
        EventPayload::SupplierIdentified {
            name: "Right Name".into(),
            vat_number: Some("4123456789".into()),
        },
        Some(&old),
    );
    let totals = event(
        "doc",
        "totals",
        Some(d!(2026 - 09 - 05)),
        EventPayload::TotalsStated {
            net: zar(100_000),
            vat: zar(15_000),
            gross: zar(115_000),
        },
        None,
    );
    // The superseded event is listed after its correction, so only supersession can
    // explain the reading.
    let old_id = old.id().clone();
    let reading = read(&facts("doc", DocumentStatus::Done, vec![corrected, old, totals]));
    assert!(reading.current.iter().all(|event| event.id() != &old_id));
    assert_eq!(reading.document.supplier.as_deref(), Some("Right Name"));
    assert_eq!(
        reading.document.supplier_vat_number.as_deref(),
        Some("4123456789")
    );
    assert_eq!(reading.document.invoice_date, Some(d!(2026 - 09 - 05)));
    assert_eq!(reading.document.kind, Kind::Invoice);
    assert_eq!(reading.document.currency.as_deref(), Some("ZAR"));
}

#[test]
fn a_credit_note_is_not_an_invoice() {
    let events = vec![
        event(
            "doc",
            "number",
            Some(d!(2026 - 09 - 06)),
            EventPayload::InvoiceNumberStated {
                number: "CN-1".into(),
            },
            None,
        ),
        event(
            "doc",
            "credit",
            Some(d!(2026 - 09 - 06)),
            EventPayload::CreditNoteReferenced {
                original_invoice_number: "INV-1".into(),
                amount: zar(50_000),
            },
            None,
        ),
    ];
    assert_eq!(
        read(&facts("doc", DocumentStatus::Done, events)).document.kind,
        Kind::CreditNote
    );
}

#[test]
fn mixed_currencies_leave_the_currency_blank() {
    // An event may not hold two currencies, so the disagreement is between two lines.
    let events = vec![
        event(
            "doc",
            "line-1",
            Some(d!(2026 - 09 - 07)),
            EventPayload::LineItemBilled {
                description: "Consulting".into(),
                quantity_milli: 1_000,
                unit_price: zar(100_000),
                line_total: zar(100_000),
                tax_category: crate::TaxCategory::Standard,
            },
            None,
        ),
        event(
            "doc",
            "line-2",
            Some(d!(2026 - 09 - 07)),
            EventPayload::LineItemBilled {
                description: "Travel".into(),
                quantity_milli: 1_000,
                unit_price: Money::new(15_000, crate::Currency::Usd),
                line_total: Money::new(15_000, crate::Currency::Usd),
                tax_category: crate::TaxCategory::Standard,
            },
            None,
        ),
    ];
    let reading = read(&facts("doc", DocumentStatus::Done, events));
    assert_eq!(reading.document.currency, None);
    assert!(reading.document.flags.contains(&"mixed_currency".to_string()));
}

#[test]
fn no_totals_leaves_the_amounts_blank_and_reads_currency_from_the_lines() {
    let events = vec![event(
        "doc",
        "line-1",
        Some(d!(2026 - 09 - 08)),
        EventPayload::LineItemBilled {
            description: "Consulting".into(),
            quantity_milli: 1_000,
            unit_price: zar(100_000),
            line_total: zar(100_000),
            tax_category: crate::TaxCategory::Standard,
        },
        None,
    )];
    let reading = read(&facts("doc", DocumentStatus::Done, events));
    assert_eq!(reading.document.currency.as_deref(), Some("ZAR"));
    assert_eq!(reading.document.net, None);
    assert_eq!(reading.document.gross, None);
    assert_eq!(reading.document.line_count, 1);
}

#[test]
fn totals_in_one_currency_do_not_hide_a_line_in_another() {
    // ZAR totals with a USD line: the document's money events disagree.
    let events = vec![
        event(
            "doc",
            "totals",
            Some(d!(2026 - 09 - 10)),
            EventPayload::TotalsStated {
                net: zar(100_000),
                vat: zar(15_000),
                gross: zar(115_000),
            },
            None,
        ),
        event(
            "doc",
            "line-1",
            Some(d!(2026 - 09 - 10)),
            EventPayload::LineItemBilled {
                description: "Travel".into(),
                quantity_milli: 1_000,
                unit_price: Money::new(15_000, crate::Currency::Usd),
                line_total: Money::new(15_000, crate::Currency::Usd),
                tax_category: crate::TaxCategory::Standard,
            },
            None,
        ),
    ];
    let reading = read(&facts("doc", DocumentStatus::Done, events));
    assert_eq!(reading.document.currency, None);
    assert!(reading.document.flags.contains(&"mixed_currency".to_string()));
}

#[test]
fn readers_and_review_reasons_are_listed_once_in_first_met_order() {
    let events = vec![
        event(
            "doc",
            "supplier-1",
            Some(d!(2026 - 09 - 11)),
            EventPayload::SupplierIdentified {
                name: "Acme".into(),
                vat_number: None,
            },
            None,
        ),
        event(
            "doc",
            "supplier-2",
            Some(d!(2026 - 09 - 11)),
            EventPayload::SupplierIdentified {
                name: "Acme".into(),
                vat_number: None,
            },
            None,
        ),
        Event::new(EventParts {
            id: EventId::new("doc:supplier-3".to_string()),
            date: Some(d!(2026 - 09 - 11)),
            payload: EventPayload::SupplierIdentified {
                name: "Acme".into(),
                vat_number: None,
            },
            citation: Citation {
                document: DocumentId::new("doc"),
                page: NonZeroU32::MIN,
                locator: None,
            },
            extractor: Extractor::Human { user: "kone".into() },
            confidence: Confidence::CERTAIN,
            supersedes: None,
        })
        .unwrap(),
    ];
    let mut source = facts("doc", DocumentStatus::NeedsReview, events);
    source.pages = vec![
        PageReview {
            page: 1,
            reason: ReviewReason::AmountsIncludeVat,
        },
        PageReview {
            page: 2,
            reason: ReviewReason::AmountsIncludeVat,
        },
    ];
    let reading = read(&source);
    assert_eq!(reading.document.read_by, vec!["ClearCut", "a person"]);
    assert_eq!(reading.document.review_reasons, vec!["amounts_include_vat"]);
}

#[test]
fn review_pages_duplicates_and_readers_are_carried() {
    let events = vec![event(
        "doc",
        "duplicate",
        Some(d!(2026 - 09 - 09)),
        EventPayload::PossibleDuplicate {
            matching_document: DocumentId::new("earlier"),
            reason: crate::event::DuplicateReason::SupplierNumberDateTotal,
        },
        None,
    )];
    let mut source = facts("doc", DocumentStatus::NeedsReview, events);
    source.pages = vec![PageReview {
        page: 2,
        reason: ReviewReason::AmountsIncludeVat,
    }];
    let reading = read(&source);
    assert_eq!(reading.document.duplicate_of.as_deref(), Some("earlier"));
    assert_eq!(reading.document.review_reasons, vec!["amounts_include_vat"]);
    assert_eq!(reading.document.read_by, vec!["ClearCut"]);
}

fn line_on(doc: &str, id: &str, page: u32, description: &str, quantity_milli: i64, unit_minor: i64) -> Event {
    Event::new(EventParts {
        id: EventId::new(format!("{doc}:{id}")),
        date: Some(d!(2026 - 09 - 10)),
        payload: EventPayload::LineItemBilled {
            description: description.into(),
            quantity_milli,
            unit_price: zar(unit_minor),
            line_total: zar(unit_minor * quantity_milli / 1_000),
            tax_category: TaxCategory::Standard,
        },
        citation: Citation {
            document: DocumentId::new(doc),
            page: NonZeroU32::new(page).unwrap(),
            locator: None,
        },
        extractor: Extractor::System,
        confidence: Confidence::CERTAIN,
        supersedes: None,
    })
    .unwrap()
}

fn vat_on(
    doc: &str,
    id: &str,
    date: Option<time::Date>,
    category: TaxCategory,
    taxable: i64,
    vat: i64,
) -> Event {
    event(
        doc,
        id,
        date,
        EventPayload::VatCalculated {
            category,
            taxable: zar(taxable),
            rate_bps: category.rate_bps_in(crate::tax::ZA_VAT),
            vat: zar(vat),
        },
        None,
    )
}

fn totals_on(doc: &str, date: Option<time::Date>, net: i64, vat: i64, gross: i64) -> Event {
    event(
        doc,
        "totals",
        date,
        EventPayload::TotalsStated {
            net: zar(net),
            vat: zar(vat),
            gross: zar(gross),
        },
        None,
    )
}

fn numbered(doc: &str, date: Option<time::Date>, number: &str) -> Event {
    event(
        doc,
        "number",
        date,
        EventPayload::InvoiceNumberStated {
            number: number.into(),
        },
        None,
    )
}

#[test]
fn a_line_row_carries_its_document_number_and_page() {
    let reading = read(&facts(
        "doc",
        DocumentStatus::Done,
        vec![
            numbered("doc", Some(d!(2026 - 09 - 10)), "INV-1"),
            line_on("doc", "l1", 1, "Consulting", 1_000, 100_000),
            line_on("doc", "l2", 2, "Travel", 2_500, 10_000),
        ],
    ));
    let rows = lines(&[reading]);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].number, 1);
    assert_eq!(rows[0].invoice_number.as_deref(), Some("INV-1"));
    assert_eq!(rows[0].quantity, "1");
    assert_eq!(rows[0].line_total.minor, 100_000);
    assert_eq!(rows[1].number, 2);
    assert_eq!(rows[1].quantity, "2.5");
    assert_eq!(rows[1].page, 2);
    assert_eq!(rows[1].line_total.minor, 25_000);
}

#[test]
fn vat_is_added_per_category_and_side() {
    let on = Some(d!(2026 - 09 - 15));
    let done = read(&facts(
        "done",
        DocumentStatus::Done,
        vec![
            numbered("done", on, "INV-1"),
            vat_on("done", "std", on, TaxCategory::Standard, 100_000, 15_000),
            vat_on("done", "zero", on, TaxCategory::ZeroRated, 50_000, 0),
        ],
    ));
    let awaiting = read(&facts(
        "awaiting",
        DocumentStatus::NeedsReview,
        vec![
            numbered("awaiting", on, "INV-2"),
            vat_on("awaiting", "std", on, TaxCategory::Standard, 20_000, 3_000),
        ],
    ));
    let groups = vat_by_category(&[done, awaiting]).unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].category, TaxCategory::Standard);
    assert_eq!(groups[0].currency, "ZAR");
    assert_eq!(groups[0].rate_bps, 1_500);
    let ready = groups[0].ready.as_ref().unwrap();
    assert_eq!(ready.documents, 1);
    assert_eq!(ready.taxable.minor, 100_000);
    assert_eq!(ready.vat.minor, 15_000);
    assert_eq!(groups[0].awaiting.as_ref().unwrap().vat.minor, 3_000);
    assert_eq!(groups[1].category, TaxCategory::ZeroRated);
    assert!(groups[1].awaiting.is_none());
}

#[test]
fn a_credit_note_is_kept_apart_from_an_invoice() {
    let on = Some(d!(2026 - 09 - 16));
    let invoice = read(&facts(
        "inv",
        DocumentStatus::Done,
        vec![
            numbered("inv", on, "INV-9"),
            vat_on("inv", "vat", on, TaxCategory::Standard, 100_000, 15_000),
        ],
    ));
    let credit = read(&facts(
        "credit",
        DocumentStatus::Done,
        vec![
            event(
                "credit",
                "ref",
                on,
                EventPayload::CreditNoteReferenced {
                    original_invoice_number: "INV-1".into(),
                    amount: zar(-115_000),
                },
                None,
            ),
            vat_on("credit", "vat", on, TaxCategory::Standard, -100_000, -15_000),
        ],
    ));
    let groups = vat_by_category(&[invoice, credit]).unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].kind, Kind::Invoice);
    assert_eq!(groups[1].kind, Kind::CreditNote);
    assert_eq!(groups[1].ready.as_ref().unwrap().vat.minor, -15_000);
}

#[test]
fn a_document_without_a_date_is_left_out_of_the_sums() {
    let source = facts(
        "undated",
        DocumentStatus::Done,
        vec![
            numbered("undated", None, "INV-3"),
            totals_on("undated", None, 100_000, 15_000, 115_000),
            vat_on("undated", "vat", None, TaxCategory::Standard, 100_000, 15_000),
        ],
    );
    assert!(vat_by_category(&[read(&source)]).unwrap().is_empty());
    assert!(stated_totals(&[read(&source)]).unwrap().is_empty());
    assert!(reconciliation(&[read(&source)]).unwrap().is_empty());
}

#[test]
fn a_mixed_currency_document_is_left_out_of_the_sums() {
    // An event may not hold two currencies, so the disagreement is between two lines.
    let on = Some(d!(2026 - 09 - 18));
    let source = facts(
        "mixed",
        DocumentStatus::Done,
        vec![
            numbered("mixed", on, "INV-4"),
            event(
                "mixed",
                "l1",
                on,
                EventPayload::LineItemBilled {
                    description: "Consulting".into(),
                    quantity_milli: 1_000,
                    unit_price: zar(100_000),
                    line_total: zar(100_000),
                    tax_category: TaxCategory::Standard,
                },
                None,
            ),
            event(
                "mixed",
                "l2",
                on,
                EventPayload::LineItemBilled {
                    description: "Travel".into(),
                    quantity_milli: 1_000,
                    unit_price: Money::new(15_000, crate::Currency::Usd),
                    line_total: Money::new(15_000, crate::Currency::Usd),
                    tax_category: TaxCategory::Standard,
                },
                None,
            ),
            vat_on("mixed", "vat", on, TaxCategory::Standard, 100_000, 15_000),
            totals_on("mixed", on, 100_000, 15_000, 115_000),
        ],
    );
    let readings = [read(&source)];
    assert_eq!(readings[0].document.currency, None);
    assert!(vat_by_category(&readings).unwrap().is_empty());
    assert!(stated_totals(&readings).unwrap().is_empty());
    assert!(reconciliation(&readings).unwrap().is_empty());
}

#[test]
fn a_document_still_being_read_has_no_figures() {
    // Spec section 5: a received or processing document is a register row with its status and
    // no figures, whatever its partial extraction already holds.
    let on = Some(d!(2026 - 09 - 22));
    let reading = read(&facts(
        "reading",
        DocumentStatus::Processing,
        vec![
            line_on("reading", "l1", 1, "Consulting", 1_000, 100_000),
            numbered("reading", on, "INV-5"),
            totals_on("reading", on, 100_000, 15_000, 115_000),
            vat_on("reading", "vat", on, TaxCategory::Standard, 100_000, 15_000),
        ],
    ));
    let readings = [reading];
    assert_eq!(readings[0].document.status, DocumentStatus::Processing);
    assert!(lines(&readings).is_empty());
    assert!(vat_by_category(&readings).unwrap().is_empty());
    assert!(stated_totals(&readings).unwrap().is_empty());
    assert!(reconciliation(&readings).unwrap().is_empty());
}

#[test]
fn the_stated_totals_of_a_month_are_added_per_side() {
    let on = Some(d!(2026 - 09 - 19));
    let done = read(&facts(
        "done",
        DocumentStatus::Done,
        vec![
            numbered("done", on, "INV-1"),
            totals_on("done", on, 100_000, 15_000, 115_000),
        ],
    ));
    let awaiting = read(&facts(
        "awaiting",
        DocumentStatus::NeedsReview,
        vec![
            numbered("awaiting", on, "INV-2"),
            totals_on("awaiting", on, 200_000, 30_000, 230_000),
        ],
    ));
    let groups = stated_totals(&[done, awaiting]).unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].month, "2026-09");
    assert_eq!(groups[0].currency, "ZAR");
    let ready = groups[0].ready.as_ref().unwrap();
    assert_eq!(ready.documents, 1);
    assert_eq!(ready.gross.minor, 115_000);
    let awaiting = groups[0].awaiting.as_ref().unwrap();
    assert_eq!(awaiting.documents, 1);
    assert_eq!(awaiting.gross.minor, 230_000);
}

#[test]
fn the_vat_calculated_is_reconciled_with_the_vat_stated() {
    let on = Some(d!(2026 - 09 - 20));
    let agrees = read(&facts(
        "agrees",
        DocumentStatus::Done,
        vec![
            numbered("agrees", on, "INV-1"),
            totals_on("agrees", on, 100_000, 15_000, 115_000),
            vat_on("agrees", "vat", on, TaxCategory::Standard, 100_000, 15_000),
        ],
    ));
    let disagrees = read(&facts(
        "disagrees",
        DocumentStatus::NeedsReview,
        vec![
            numbered("disagrees", on, "INV-2"),
            totals_on("disagrees", on, 100_000, 14_000, 114_000),
            vat_on("disagrees", "vat", on, TaxCategory::Standard, 100_000, 15_000),
        ],
    ));
    let rows = reconciliation(&[agrees, disagrees]).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].calculated_vat.minor, 30_000);
    assert_eq!(rows[0].stated_vat.minor, 29_000);
    assert_eq!(rows[0].difference.minor, 1_000);
    assert_eq!(rows[0].documents_disagree, 1);
}

#[test]
fn a_document_with_no_calculated_vat_is_not_reconciled() {
    let on = Some(d!(2026 - 09 - 21));
    let only_stated = read(&facts(
        "stated",
        DocumentStatus::Done,
        vec![
            numbered("stated", on, "INV-7"),
            totals_on("stated", on, 100_000, 15_000, 115_000),
        ],
    ));
    assert!(reconciliation(&[only_stated]).unwrap().is_empty());
}

fn request(facts: Vec<RegisterFacts>) -> RegisterInput {
    RegisterInput {
        selection: Selection::Documents {
            ids: facts.iter().map(|facts| facts.id.clone()).collect(),
        },
        only_ready: false,
        undated_in_tenant: 0,
        facts,
    }
}

#[test]
fn an_empty_selection_makes_an_empty_register() {
    let register = build(RegisterInput {
        selection: Selection::Documents { ids: Vec::new() },
        only_ready: false,
        undated_in_tenant: 2,
        facts: Vec::new(),
    })
    .unwrap();
    assert!(register.rows.is_empty());
    assert!(register.lines.is_empty());
    assert!(register.to_check.is_empty());
    assert_eq!(register.manifest.documents, 0);
    assert_eq!(register.manifest.undated_in_tenant, 2);
    assert_eq!(register.manifest.automation_rate_bps, None);
    assert_eq!(register.notice, REGISTER_NOTICE);
    assert!(!register.version.is_empty());
}

#[test]
fn rows_are_in_date_order_with_undated_last() {
    let late = facts(
        "late",
        DocumentStatus::Done,
        vec![numbered("late", Some(d!(2026 - 09 - 20)), "INV-2")],
    );
    let early = facts(
        "early",
        DocumentStatus::Done,
        vec![numbered("early", Some(d!(2026 - 09 - 01)), "INV-1")],
    );
    let undated = facts(
        "undated",
        DocumentStatus::Done,
        vec![numbered("undated", None, "INV-3")],
    );
    let register = build(request(vec![late, undated, early])).unwrap();
    let ids: Vec<&str> = register.rows.iter().map(|row| row.id.as_str()).collect();
    assert_eq!(ids, vec!["early", "late", "undated"]);
    assert_eq!(register.manifest.undated_in_selection, 1);
}

#[test]
fn a_candidate_outside_the_period_is_dropped() {
    let inside = facts(
        "inside",
        DocumentStatus::Done,
        vec![numbered("inside", Some(d!(2026 - 09 - 10)), "INV-1")],
    );
    let outside = facts(
        "outside",
        DocumentStatus::Done,
        vec![numbered("outside", Some(d!(2026 - 10 - 01)), "INV-2")],
    );
    let register = build(RegisterInput {
        selection: Selection::Period {
            from: d!(2026 - 09 - 01),
            to: d!(2026 - 09 - 30),
        },
        only_ready: false,
        undated_in_tenant: 0,
        facts: vec![inside, outside],
    })
    .unwrap();
    assert_eq!(register.rows.len(), 1);
    assert_eq!(register.rows[0].id, "inside");
    assert_eq!(register.manifest.documents, 1);
}

#[test]
fn an_undated_document_is_listed_to_check() {
    let undated = facts(
        "undated",
        DocumentStatus::Done,
        vec![numbered("undated", None, "INV-3")],
    );
    let register = build(request(vec![undated])).unwrap();
    assert_eq!(register.rows.len(), 1);
    let row = register
        .to_check
        .iter()
        .find(|row| row.reason == "no_invoice_date")
        .expect("the undated document is listed");
    assert_eq!(row.document_id, "undated");
    assert_eq!(row.page, None);
}

#[test]
fn a_page_that_stopped_for_a_person_is_listed_with_its_page() {
    let mut source = facts(
        "review",
        DocumentStatus::NeedsReview,
        vec![numbered("review", Some(d!(2026 - 09 - 05)), "INV-4")],
    );
    source.pages = vec![PageReview {
        page: 3,
        reason: ReviewReason::AmountsIncludeVat,
    }];
    let register = build(request(vec![source])).unwrap();
    let row = register
        .to_check
        .iter()
        .find(|row| row.reason == "amounts_include_vat")
        .expect("the page is listed");
    assert_eq!(row.page, Some(3));
    assert!(row.sentence.contains("include VAT"));
}

#[test]
fn a_document_still_reading_is_listed_to_check() {
    let reading = facts("reading", DocumentStatus::Processing, Vec::new());
    let register = build(request(vec![reading])).unwrap();
    assert_eq!(register.rows.len(), 1);
    assert!(register.to_check.iter().any(|row| row.reason == "still_reading"));
    assert_eq!(register.manifest.automation_rate_bps, None);
}

#[test]
fn only_ready_leaves_awaiting_documents_out() {
    let done = facts(
        "done",
        DocumentStatus::Done,
        vec![numbered("done", Some(d!(2026 - 09 - 06)), "INV-1")],
    );
    let awaiting = facts(
        "awaiting",
        DocumentStatus::NeedsReview,
        vec![numbered("awaiting", Some(d!(2026 - 09 - 07)), "INV-2")],
    );
    let mut input = request(vec![done, awaiting]);
    input.only_ready = true;
    let register = build(input).unwrap();
    assert_eq!(register.rows.len(), 1);
    assert_eq!(register.manifest.documents, 2);
    assert_eq!(register.manifest.done, 1);
    assert_eq!(register.manifest.needs_review, 1);
    assert_eq!(register.manifest.awaiting_left_out, 1);
    assert_eq!(register.manifest.automation_rate_bps, Some(5_000));
}

#[test]
fn more_documents_than_the_limit_is_refused() {
    let many: Vec<RegisterFacts> = (0..=MAX_DOCUMENTS)
        .map(|n| facts(&format!("d{n}"), DocumentStatus::Done, Vec::new()))
        .collect();
    assert_eq!(build(request(many)).unwrap_err(), RegisterError::TooManyDocuments);
}

#[test]
fn more_lines_than_the_limit_is_refused() {
    let events: Vec<Event> = (0..=MAX_LINES)
        .map(|n| line_on("big", &format!("l{n}"), 1, "Item", 1_000, 100))
        .collect();
    let source = facts("big", DocumentStatus::Done, events);
    assert_eq!(
        build(request(vec![source])).unwrap_err(),
        RegisterError::TooManyLines
    );
}
