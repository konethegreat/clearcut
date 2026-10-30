// Written by Kone Tshivhinda & Claude
//! Pure tests: no database, no clock, no model.

use super::*;
use crate::event::{Citation, Confidence, DocumentId, Event, EventId, EventParts, EventPayload, Extractor};
use crate::Money;
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
    let reading = read(&facts("doc", DocumentStatus::Done, vec![old, corrected, totals]));
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
