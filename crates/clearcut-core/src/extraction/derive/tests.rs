// Written by Kone Tshivhinda & Claude
use super::*;
use crate::event::{
    Confidence, DocumentId, EventGroup, EventId, EventPayload, EventType, Extractor, Locator, TotalField,
};
use crate::extraction::fixtures::{at, clean, zar};
use crate::extraction::{validate, CreditNoteRef, Sourced};
use crate::narration::guard::check;
use crate::narration::narrate_timeline;
use crate::{Currency, Error, InvoiceTotals, Money, TaxCategory};
use std::num::NonZeroU32;
use time::macros::date as d;

/// `value` read from `page` at the given confidence (per-mille).
fn read_at<T>(value: T, page: u32, confidence: u16) -> Sourced<T> {
    Sourced {
        value,
        page: NonZeroU32::new(page).unwrap(),
        locator: None,
        confidence: Confidence::new(confidence).unwrap(),
    }
}

fn derive(x: &Extraction) -> Vec<Event> {
    derive_events(x).unwrap().events
}

fn ids(events: &[Event]) -> Vec<&str> {
    events.iter().map(|e| e.id().as_str()).collect()
}

fn find<'a>(events: &'a [Event], id: &str) -> &'a Event {
    events
        .iter()
        .find(|e| e.id().as_str() == id)
        .unwrap_or_else(|| panic!("no event {id}; have {:?}", ids(events)))
}

fn has(events: &[Event], id: &str) -> bool {
    events.iter().any(|e| e.id().as_str() == id)
}

fn issue_payload(field: &str, message: &str) -> EventPayload {
    EventPayload::ComplianceIssueFound {
        field: field.into(),
        message: message.into(),
    }
}

/// An extraction with something wrong in most places.
fn messy() -> Extraction {
    let mut x = clean();
    x.lines[1].value.line_total = zar(110_000);
    x.stated_totals = Some(at(InvoiceTotals {
        net: zar(1_600_000),
        vat: zar(225_001),
        gross: zar(1_825_001),
    }));
    x.supplier.as_mut().unwrap().value.address = None;
    x.due.as_mut().unwrap().value.due = d!(2026 - 03 - 01);
    x.credit_note = Some(at(CreditNoteRef {
        original_invoice_number: "INV-0042".into(),
        amount: zar(50_000),
    }));
    x
}

// ---------- a clean extraction ----------

#[test]
fn a_clean_invoice_gives_one_event_per_fact_with_stable_ids() {
    let events = derive(&clean());
    assert_eq!(
        ids(&events),
        [
            "doc_1:supplier",
            "doc_1:line-1",
            "doc_1:line-2",
            "doc_1:vat-standard",
            "doc_1:vat-zero-rated",
            "doc_1:totals-stated",
            "doc_1:due",
        ]
    );
}

#[test]
fn the_validation_is_returned_with_the_events() {
    let x = messy();
    let derived = derive_events(&x).unwrap();
    assert_eq!(derived.validation, validate(&x));
}

#[test]
fn facts_keep_their_printed_values() {
    let events = derive(&clean());
    assert_eq!(
        find(&events, "doc_1:supplier").payload(),
        &EventPayload::SupplierIdentified {
            name: "Acme (Pty) Ltd".into(),
            vat_number: Some("4123456789".into()),
        }
    );
    assert_eq!(
        find(&events, "doc_1:line-1").payload(),
        &EventPayload::LineItemBilled {
            description: "Consulting".into(),
            quantity_milli: 10_000,
            unit_price: zar(150_000),
            line_total: zar(1_500_000),
            tax_category: TaxCategory::Standard,
        }
    );
    assert_eq!(
        find(&events, "doc_1:totals-stated").payload(),
        &EventPayload::TotalsStated {
            net: zar(1_600_000),
            vat: zar(225_000),
            gross: zar(1_825_000),
        }
    );
    assert_eq!(
        find(&events, "doc_1:due").payload(),
        &EventPayload::DueDateStated {
            due: d!(2026 - 04 - 30),
            terms: Some("30 days".into()),
        }
    );
}

#[test]
fn a_line_event_shows_the_printed_total_even_when_it_is_wrong() {
    let mut x = clean();
    x.lines[1].value.line_total = zar(110_000);
    let events = derive(&x);
    match find(&events, "doc_1:line-2").payload() {
        EventPayload::LineItemBilled { line_total, .. } => assert_eq!(*line_total, zar(110_000)),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn vat_is_calculated_per_tax_category_that_is_present() {
    let events = derive(&clean());
    assert_eq!(
        find(&events, "doc_1:vat-standard").payload(),
        &EventPayload::VatCalculated {
            category: TaxCategory::Standard,
            taxable: zar(1_500_000),
            rate_bps: 1_500,
            vat: zar(225_000),
        }
    );
    assert_eq!(
        find(&events, "doc_1:vat-zero-rated").payload(),
        &EventPayload::VatCalculated {
            category: TaxCategory::ZeroRated,
            taxable: zar(100_000),
            rate_bps: 0,
            vat: zar(0),
        }
    );
    assert!(!has(&events, "doc_1:vat-exempt"));
    assert!(!has(&events, "doc_1:vat-out-of-scope"));
}

#[test]
fn a_credit_note_reference_becomes_an_adjustment_event() {
    let events = derive(&messy());
    let event = find(&events, "doc_1:credit-note");
    assert_eq!(event.group(), EventGroup::Adjustments);
    assert_eq!(
        event.payload(),
        &EventPayload::CreditNoteReferenced {
            original_invoice_number: "INV-0042".into(),
            amount: zar(50_000),
        }
    );
}

// ---------- who said it, and where ----------

#[test]
fn printed_facts_are_attributed_to_the_extractor_with_their_source() {
    let mut x = clean();
    x.extractor = Extractor::Model {
        name: "claude-haiku-5-5".into(),
        prompt_version: "v3".into(),
    };
    x.supplier = Some(Sourced {
        locator: Some(Locator::TextSpan { start: 5, end: 20 }),
        ..read_at(x.supplier.clone().unwrap().value, 3, 800)
    });
    let events = derive(&x);
    let supplier = find(&events, "doc_1:supplier");
    assert_eq!(supplier.extractor(), &x.extractor);
    assert_eq!(supplier.confidence().get(), 800);
    assert_eq!(supplier.citation().page.get(), 3);
    assert_eq!(
        supplier.citation().locator,
        Some(Locator::TextSpan { start: 5, end: 20 })
    );
    assert_eq!(supplier.citation().document, DocumentId::new("doc_1"));
}

#[test]
fn calculated_facts_are_attributed_to_the_system_with_the_weakest_source_confidence() {
    let mut x = clean();
    x.extractor = Extractor::Model {
        name: "claude-haiku-5-5".into(),
        prompt_version: "v3".into(),
    };
    x.lines = vec![
        read_at(x.lines[0].value.clone(), 2, 900),
        read_at(x.lines[0].value.clone(), 3, 600), // a second standard-rated line
        read_at(x.lines[1].value.clone(), 4, 700),
    ];
    x.stated_totals = None;
    let events = derive(&x);
    let standard = find(&events, "doc_1:vat-standard");
    assert_eq!(standard.extractor(), &Extractor::System);
    assert_eq!(standard.confidence().get(), 600);
    assert_eq!(standard.citation().page.get(), 2); // first line of the category
    let zero = find(&events, "doc_1:vat-zero-rated");
    assert_eq!(zero.confidence().get(), 700);
    assert_eq!(zero.citation().page.get(), 4);
}

#[test]
fn every_event_is_dated_with_the_invoice_date() {
    let events = derive(&messy());
    assert!(events.iter().all(|e| e.date() == Some(d!(2026 - 03 - 31))));
    let mut x = clean();
    x.invoice_date = None;
    assert!(derive(&x).iter().all(|e| e.date().is_none()));
}

// ---------- findings become events ----------

#[test]
fn a_wrong_line_total_becomes_a_critical_event_citing_that_line() {
    let mut x = clean();
    x.lines[1] = read_at(x.lines[1].value.clone(), 2, 910);
    x.lines[1].value.line_total = zar(110_000);
    let events = derive(&x);
    let event = find(&events, "doc_1:line-mismatch-2");
    assert_eq!(
        event.payload(),
        &EventPayload::LineTotalMismatch {
            line: 2,
            stated: zar(110_000),
            recomputed: zar(100_000),
        }
    );
    assert_eq!(event.extractor(), &Extractor::System);
    assert_eq!(event.citation().page.get(), 2);
    assert_eq!(event.confidence().get(), 910);
}

#[test]
fn each_wrong_total_becomes_its_own_event_citing_the_printed_totals() {
    let mut x = clean();
    x.stated_totals = Some(read_at(
        InvoiceTotals {
            net: zar(1_600_000),
            vat: zar(225_001),
            gross: zar(1_825_001),
        },
        4,
        930,
    ));
    let events = derive(&x);
    assert!(!has(&events, "doc_1:totals-mismatch-net"));
    let vat = find(&events, "doc_1:totals-mismatch-vat");
    assert_eq!(
        vat.payload(),
        &EventPayload::TotalsMismatch {
            field: TotalField::Vat,
            stated: zar(225_001),
            recomputed: zar(225_000),
        }
    );
    assert_eq!(vat.citation().page.get(), 4);
    assert_eq!(vat.confidence().get(), 930);
    assert_eq!(vat.extractor(), &Extractor::System);
    assert!(has(&events, "doc_1:totals-mismatch-gross"));
}

#[test]
fn compliance_and_other_findings_become_numbered_warning_events() {
    let mut x = clean();
    x.supplier.as_mut().unwrap().value.address = None;
    x.invoice_number = None;
    let events = derive(&x);
    let first = find(&events, "doc_1:issue-1");
    assert_eq!(
        first.payload(),
        &issue_payload("supplier_address", "supplier address is required")
    );
    assert_eq!(first.extractor(), &Extractor::System);
    assert_eq!(first.confidence().get(), 1000);
    assert_eq!(first.citation().page.get(), 1);
    assert_eq!(first.citation().locator, None);
    assert_eq!(
        find(&events, "doc_1:issue-2").payload(),
        &issue_payload("serial_number", "unique sequential serial number is required")
    );
}

#[test]
fn a_due_date_before_the_invoice_date_cites_the_due_date() {
    let mut x = clean();
    x.due = Some(read_at(x.due.clone().unwrap().value, 2, 900));
    x.due.as_mut().unwrap().value.due = d!(2026 - 03 - 01);
    let events = derive(&x);
    let event = find(&events, "doc_1:issue-1");
    assert_eq!(
        event.payload(),
        &issue_payload("due_date", "due date is before the invoice date")
    );
    assert_eq!(event.citation().page.get(), 2);
}

#[test]
fn an_invoice_with_no_lines_says_so_and_has_no_line_or_vat_events() {
    let mut x = clean();
    x.lines.clear();
    x.stated_totals = None;
    let events = derive(&x);
    assert_eq!(
        find(&events, "doc_1:issue-1").payload(),
        &issue_payload("lines", "no line items were read from the document")
    );
    assert!(events.iter().all(|e| !matches!(
        e.event_type(),
        EventType::LineItemBilled | EventType::VatCalculated
    )));
}

#[test]
fn an_untrustworthy_line_is_reported_not_narrated_and_blocks_the_vat_events() {
    let mut x = clean();
    x.lines[1] = read_at(x.lines[1].value.clone(), 2, 900);
    x.lines[1].value.unit_price = Money::new(100_000, Currency::Usd);
    let events = derive(&x);
    assert!(has(&events, "doc_1:line-1"));
    assert!(!has(&events, "doc_1:line-2"));
    let event = find(&events, "doc_1:issue-1");
    assert_eq!(
        event.payload(),
        &issue_payload("line 2 currency", "line amounts are not in the document currency")
    );
    assert_eq!(event.citation().page.get(), 2);
    // Without trustworthy lines nothing can be calculated.
    assert!(events.iter().all(|e| e.event_type() != EventType::VatCalculated));
}

#[test]
fn a_fact_with_mixed_currencies_becomes_an_issue_instead_of_failing() {
    let mut x = clean();
    x.stated_totals = Some(at(InvoiceTotals {
        net: zar(1_600_000),
        vat: Money::new(225_000, Currency::Usd),
        gross: zar(1_825_000),
    }));
    let events = derive(&x);
    assert_eq!(
        find(&events, "doc_1:totals-stated").payload(),
        &issue_payload("totals-stated", "amounts in one fact must share a currency")
    );
    assert_eq!(
        find(&events, "doc_1:totals-mismatch-vat").payload(),
        &issue_payload("totals-mismatch-vat", "amounts in one fact must share a currency")
    );
}

// ---------- determinism and failure ----------

#[test]
fn ids_are_unique_and_prefixed_with_the_document() {
    let events = derive(&messy());
    let mut seen = std::collections::HashSet::new();
    for event in &events {
        assert!(event.id().as_str().starts_with("doc_1:"));
        assert!(seen.insert(event.id().clone()), "duplicate {:?}", event.id());
    }
    assert!(has(&events, "doc_1:line-mismatch-2"));
    assert!(has(&events, "doc_1:totals-mismatch-vat"));
}

#[test]
fn deriving_twice_gives_the_same_events() {
    assert_eq!(derive(&messy()), derive(&messy()));
}

#[test]
fn a_bad_location_from_the_extractor_is_an_error_not_a_silent_drop() {
    let mut x = clean();
    x.supplier.as_mut().unwrap().locator = Some(Locator::BoundingBox {
        x: 900,
        y: 0,
        width: 200,
        height: 10,
    });
    assert_eq!(
        derive_events(&x).map(|d| d.events.len()),
        Err(Error::InvalidEvent("bounding box must lie within the page"))
    );
}

#[test]
fn an_empty_document_id_is_an_error() {
    let mut x = clean();
    x.document = DocumentId::new("");
    assert_eq!(
        derive_events(&x).map(|d| d.events.len()),
        Err(Error::InvalidEvent("document id must not be empty"))
    );
}

// ---------- end to end with the narrator and the number guard ----------

#[test]
fn derived_events_narrate_and_pass_the_number_guard() {
    for x in [clean(), messy()] {
        let events = derive(&x);
        let narrative = narrate_timeline(&events);
        assert!(!narrative.sentences().is_empty());
        assert_eq!(check(&narrative, &events), Vec::new());
    }
}

#[test]
fn derived_events_can_be_corrected_by_a_later_event() {
    // A reviewer fixes the supplier: the correction supersedes the derived event.
    let mut events = derive(&clean());
    let old = find(&events, "doc_1:supplier").clone();
    let fixed = Event::new(crate::event::EventParts {
        id: EventId::new("doc_1:supplier-corrected"),
        date: old.date(),
        payload: EventPayload::SupplierIdentified {
            name: "Acme Holdings (Pty) Ltd".into(),
            vat_number: Some("4123456789".into()),
        },
        citation: old.citation().clone(),
        extractor: Extractor::Human { user: "kone".into() },
        confidence: Confidence::new(1000).unwrap(),
        supersedes: Some(old.id().clone()),
    })
    .unwrap();
    events.push(fixed);
    let current = crate::event::view::current(&events);
    assert!(!current.iter().any(|e| e.id().as_str() == "doc_1:supplier"));
    assert!(current
        .iter()
        .any(|e| e.id().as_str() == "doc_1:supplier-corrected"));
}
