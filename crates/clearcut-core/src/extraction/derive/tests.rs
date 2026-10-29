// Written by Kone Tshivhinda & Claude
use super::*;
use crate::event::{
    Confidence, DocumentId, EventGroup, EventId, EventPayload, EventType, Extractor, Locator, TotalField,
};
use crate::extraction::fixtures::{at, clean, party, zar};
use crate::extraction::{validate, CarriedTotals, CreditNoteRef, Sourced};
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

fn run(n: u32) -> NonZeroU32 {
    NonZeroU32::new(n).unwrap()
}

/// The first extraction run of a document.
fn derive(x: &Extraction) -> Vec<Event> {
    derive_events(x, run(1), &[]).unwrap().events
}

/// A later run, given the events that exist from earlier runs.
fn derive_run(x: &Extraction, n: u32, earlier: &[Event]) -> Vec<Event> {
    derive_events(x, run(n), earlier).unwrap().events
}

fn supersedes_of<'a>(events: &'a [Event], id: &str) -> Option<&'a str> {
    find(events, id).supersedes().map(EventId::as_str)
}

/// A reviewer's correction of `old`: a new Human event that supersedes it.
fn corrected_by_human(old: &Event, new_id: &str) -> Event {
    Event::new(crate::event::EventParts {
        id: EventId::new(new_id),
        date: old.date(),
        payload: old.payload().clone(),
        citation: old.citation().clone(),
        extractor: Extractor::Human { user: "kone".into() },
        confidence: Confidence::new(1000).unwrap(),
        supersedes: Some(old.id().clone()),
    })
    .unwrap()
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
            "doc_1:r1:invoice-number",
            "doc_1:r1:supplier",
            "doc_1:r1:recipient",
            "doc_1:r1:line-1",
            "doc_1:r1:line-2",
            "doc_1:r1:vat-standard",
            "doc_1:r1:vat-zero-rated",
            "doc_1:r1:totals-stated",
            "doc_1:r1:due",
        ]
    );
}

#[test]
fn the_invoice_number_and_recipient_are_events_with_their_page_and_confidence() {
    let events = derive(&clean());
    let number = find(&events, "doc_1:r1:invoice-number");
    assert_eq!(
        number.payload(),
        &EventPayload::InvoiceNumberStated {
            number: "INV-000001".into()
        }
    );
    assert_eq!(number.citation().page.get(), 1);
    assert_eq!(number.confidence().get(), 950);

    let recipient = find(&events, "doc_1:r1:recipient");
    assert_eq!(
        recipient.payload(),
        &EventPayload::RecipientIdentified {
            name: "Client Co".into(),
            vat_number: Some("4987654321".into()),
        }
    );
    assert_eq!(recipient.group(), EventGroup::Parties);
}

#[test]
fn a_missing_invoice_number_or_recipient_gives_no_event() {
    let mut x = clean();
    x.invoice_number = None;
    x.recipient = None;
    let events = derive(&x);
    assert!(!has(&events, "doc_1:r1:invoice-number"));
    assert!(!has(&events, "doc_1:r1:recipient"));
}

#[test]
fn a_second_run_supersedes_the_invoice_number_and_recipient_too() {
    let run1 = derive(&clean());
    let mut x = clean();
    x.invoice_number = Some(at("INV-000002".to_string()));
    x.recipient = Some(party("Client Two", "4987654322"));
    let run2 = derive_run(&x, 2, &run1);
    assert_eq!(
        supersedes_of(&run2, "doc_1:r2:invoice-number"),
        Some("doc_1:r1:invoice-number")
    );
    assert_eq!(
        supersedes_of(&run2, "doc_1:r2:recipient"),
        Some("doc_1:r1:recipient")
    );
}

#[test]
fn the_validation_is_returned_with_the_events() {
    let x = messy();
    let derived = derive_events(&x, run(1), &[]).unwrap();
    assert_eq!(derived.validation, validate(&x));
}

#[test]
fn facts_keep_their_printed_values() {
    let events = derive(&clean());
    assert_eq!(
        find(&events, "doc_1:r1:supplier").payload(),
        &EventPayload::SupplierIdentified {
            name: "Acme (Pty) Ltd".into(),
            vat_number: Some("4123456789".into()),
        }
    );
    assert_eq!(
        find(&events, "doc_1:r1:line-1").payload(),
        &EventPayload::LineItemBilled {
            description: "Consulting".into(),
            quantity_milli: 10_000,
            unit_price: zar(150_000),
            line_total: zar(1_500_000),
            tax_category: TaxCategory::Standard,
        }
    );
    assert_eq!(
        find(&events, "doc_1:r1:totals-stated").payload(),
        &EventPayload::TotalsStated {
            net: zar(1_600_000),
            vat: zar(225_000),
            gross: zar(1_825_000),
        }
    );
    assert_eq!(
        find(&events, "doc_1:r1:due").payload(),
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
    match find(&events, "doc_1:r1:line-2").payload() {
        EventPayload::LineItemBilled { line_total, .. } => assert_eq!(*line_total, zar(110_000)),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn vat_is_calculated_per_tax_category_that_is_present() {
    let events = derive(&clean());
    assert_eq!(
        find(&events, "doc_1:r1:vat-standard").payload(),
        &EventPayload::VatCalculated {
            category: TaxCategory::Standard,
            taxable: zar(1_500_000),
            rate_bps: 1_500,
            vat: zar(225_000),
        }
    );
    assert_eq!(
        find(&events, "doc_1:r1:vat-zero-rated").payload(),
        &EventPayload::VatCalculated {
            category: TaxCategory::ZeroRated,
            taxable: zar(100_000),
            rate_bps: 0,
            vat: zar(0),
        }
    );
    assert!(!has(&events, "doc_1:r1:vat-exempt"));
    assert!(!has(&events, "doc_1:r1:vat-out-of-scope"));
}

#[test]
fn a_credit_note_reference_becomes_an_adjustment_event() {
    let events = derive(&messy());
    let event = find(&events, "doc_1:r1:credit-note");
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
    let supplier = find(&events, "doc_1:r1:supplier");
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
    let standard = find(&events, "doc_1:r1:vat-standard");
    assert_eq!(standard.extractor(), &Extractor::System);
    assert_eq!(standard.confidence().get(), 600);
    assert_eq!(standard.citation().page.get(), 2); // first line of the category
    let zero = find(&events, "doc_1:r1:vat-zero-rated");
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
    let event = find(&events, "doc_1:r1:line-mismatch-2");
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
    assert!(!has(&events, "doc_1:r1:totals-mismatch-net"));
    let vat = find(&events, "doc_1:r1:totals-mismatch-vat");
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
    assert!(has(&events, "doc_1:r1:totals-mismatch-gross"));
}

#[test]
fn compliance_and_other_findings_become_numbered_warning_events() {
    let mut x = clean();
    x.supplier.as_mut().unwrap().value.address = None;
    x.invoice_number = None;
    let events = derive(&x);
    let first = find(&events, "doc_1:r1:issue-1");
    assert_eq!(
        first.payload(),
        &issue_payload("supplier_address", "supplier address is required")
    );
    assert_eq!(first.extractor(), &Extractor::System);
    assert_eq!(first.confidence().get(), 1000);
    assert_eq!(first.citation().page.get(), 1);
    assert_eq!(first.citation().locator, None);
    assert_eq!(
        find(&events, "doc_1:r1:issue-2").payload(),
        &issue_payload("serial_number", "unique sequential serial number is required")
    );
}

#[test]
fn a_due_date_before_the_invoice_date_cites_the_due_date() {
    let mut x = clean();
    x.due = Some(read_at(x.due.clone().unwrap().value, 2, 900));
    x.due.as_mut().unwrap().value.due = d!(2026 - 03 - 01);
    let events = derive(&x);
    let event = find(&events, "doc_1:r1:issue-1");
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
        find(&events, "doc_1:r1:issue-1").payload(),
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
    assert!(has(&events, "doc_1:r1:line-1"));
    assert!(!has(&events, "doc_1:r1:line-2"));
    let event = find(&events, "doc_1:r1:issue-1");
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
        find(&events, "doc_1:r1:totals-stated").payload(),
        &issue_payload("totals-stated", "amounts in one fact must share a currency")
    );
    assert_eq!(
        find(&events, "doc_1:r1:totals-mismatch-vat").payload(),
        &issue_payload("totals-mismatch-vat", "amounts in one fact must share a currency")
    );
}

// ---------- determinism and failure ----------

#[test]
fn ids_are_unique_and_prefixed_with_the_document() {
    let events = derive(&messy());
    let mut seen = std::collections::HashSet::new();
    for event in &events {
        assert!(event.id().as_str().starts_with("doc_1:r1:"));
        assert!(seen.insert(event.id().clone()), "duplicate {:?}", event.id());
    }
    assert!(has(&events, "doc_1:r1:line-mismatch-2"));
    assert!(has(&events, "doc_1:r1:totals-mismatch-vat"));
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
        derive_events(&x, run(1), &[]).map(|d| d.events.len()),
        Err(Error::InvalidEvent("bounding box must lie within the page"))
    );
}

#[test]
fn an_empty_document_id_is_an_error() {
    let mut x = clean();
    x.document = DocumentId::new("");
    assert_eq!(
        derive_events(&x, run(1), &[]).map(|d| d.events.len()),
        Err(Error::InvalidEvent("document id must not be empty"))
    );
}

// ---------- re-extraction: run numbers and superseding ----------

#[test]
fn ids_carry_the_run_number() {
    let run2 = derive_run(&clean(), 2, &[]);
    assert!(run2.iter().all(|e| e.id().as_str().starts_with("doc_1:r2:")));
    assert!(has(&run2, "doc_1:r2:line-1"));
}

#[test]
fn a_second_run_supersedes_the_matching_events_of_the_first() {
    let run1 = derive(&clean());
    let run2 = derive_run(&clean(), 2, &run1);
    assert_eq!(run2.len(), run1.len());
    for event in &run2 {
        let slug = event.id().as_str().trim_start_matches("doc_1:r2:");
        assert_eq!(
            event.supersedes().map(EventId::as_str),
            Some(format!("doc_1:r1:{slug}").as_str()),
            "{slug}"
        );
    }
    let all: Vec<Event> = run1.into_iter().chain(run2.clone()).collect();
    let current = crate::event::view::current(&all);
    assert_eq!(current.len(), run2.len());
    assert!(current.iter().all(|e| e.id().as_str().starts_with("doc_1:r2:")));
}

#[test]
fn a_third_run_supersedes_the_latest_event_not_the_first() {
    let run1 = derive(&clean());
    let run2 = derive_run(&clean(), 2, &run1);
    let earlier: Vec<Event> = run1.into_iter().chain(run2).collect();
    let run3 = derive_run(&clean(), 3, &earlier);
    assert_eq!(
        supersedes_of(&run3, "doc_1:r3:supplier"),
        Some("doc_1:r2:supplier")
    );
    assert_eq!(supersedes_of(&run3, "doc_1:r3:line-2"), Some("doc_1:r2:line-2"));
}

#[test]
fn a_fact_that_is_new_in_the_second_run_supersedes_nothing() {
    let run1 = derive(&clean());
    let mut x = clean();
    x.credit_note = Some(at(CreditNoteRef {
        original_invoice_number: "INV-0042".into(),
        amount: zar(50_000),
    }));
    let run2 = derive_run(&x, 2, &run1);
    assert_eq!(supersedes_of(&run2, "doc_1:r2:credit-note"), None);
    assert_eq!(
        supersedes_of(&run2, "doc_1:r2:supplier"),
        Some("doc_1:r1:supplier")
    );
}

#[test]
fn a_fact_the_second_run_no_longer_finds_stays_current() {
    // Known limit, kept visible on purpose: nothing is retracted silently.
    let run1 = derive(&clean());
    let mut x = clean();
    x.due = None;
    let run2 = derive_run(&x, 2, &run1);
    assert!(!has(&run2, "doc_1:r2:due"));
    let all: Vec<Event> = run1.into_iter().chain(run2).collect();
    let current = crate::event::view::current(&all);
    assert!(current.iter().any(|e| e.id().as_str() == "doc_1:r1:due"));
}

#[test]
fn a_human_correction_is_never_superseded_by_a_machine_re_run() {
    let run1 = derive(&clean());
    let old_supplier = find(&run1, "doc_1:r1:supplier").clone();
    let fixed = corrected_by_human(&old_supplier, "doc_1:review-1");
    let earlier: Vec<Event> = run1.into_iter().chain([fixed]).collect();
    let run2 = derive_run(&clean(), 2, &earlier);
    // The new machine event does not replace the reviewer's fact...
    assert_eq!(supersedes_of(&run2, "doc_1:r2:supplier"), None);
    // ...but other facts still move on to the new run.
    assert_eq!(supersedes_of(&run2, "doc_1:r2:line-1"), Some("doc_1:r1:line-1"));
    // Both are visible, so a reviewer can reconcile them.
    let all: Vec<Event> = earlier.into_iter().chain(run2).collect();
    let current = crate::event::view::current(&all);
    assert!(current.iter().any(|e| e.id().as_str() == "doc_1:review-1"));
    assert!(current.iter().any(|e| e.id().as_str() == "doc_1:r2:supplier"));
}

#[test]
fn events_from_other_documents_and_later_runs_are_ignored() {
    let run1 = derive(&clean());
    let mut other = clean();
    other.document = DocumentId::new("doc_2");
    let other_run1 = derive(&other);
    let run3 = derive_run(&clean(), 3, &run1);
    // Deriving run 2 sees run 1 of this document only: not doc_2, not run 3.
    let earlier: Vec<Event> = run1.into_iter().chain(other_run1).chain(run3).collect();
    let run2 = derive_run(&clean(), 2, &earlier);
    assert_eq!(
        supersedes_of(&run2, "doc_1:r2:supplier"),
        Some("doc_1:r1:supplier")
    );
}

#[test]
fn deriving_the_same_run_again_is_harmless() {
    // A retried job derives run 2 again with run 2's events already stored.
    let run1 = derive(&clean());
    let run2 = derive_run(&clean(), 2, &run1);
    let stored: Vec<Event> = run1.into_iter().chain(run2.clone()).collect();
    assert_eq!(derive_run(&clean(), 2, &stored), run2);
}

#[test]
fn the_first_run_supersedes_nothing() {
    assert!(derive(&messy()).iter().all(|e| e.supersedes().is_none()));
}

#[test]
fn a_rerun_of_a_messy_document_stays_valid_and_passes_the_number_guard() {
    let run1 = derive(&messy());
    let run2 = derive_run(&messy(), 2, &run1);
    let all: Vec<Event> = run1.into_iter().chain(run2).collect();
    let narrative = narrate_timeline(&all); // narrates current events only
    assert!(!narrative.sentences().is_empty());
    assert_eq!(check(&narrative, &all), Vec::new());
    assert!(narrative
        .sentences()
        .iter()
        .all(|s| s.cites()[0].as_str().starts_with("doc_1:r2:")));
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
    let old = find(&events, "doc_1:r1:supplier").clone();
    let fixed = Event::new(crate::event::EventParts {
        id: EventId::new("doc_1:r1:supplier-corrected"),
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
    assert!(!current.iter().any(|e| e.id().as_str() == "doc_1:r1:supplier"));
    assert!(current
        .iter()
        .any(|e| e.id().as_str() == "doc_1:r1:supplier-corrected"));
}

// ---------- carried figures across pages (ADR-0022) ----------

fn carried_on(page: u32, brought: Option<i64>, forward: Option<i64>) -> Sourced<CarriedTotals> {
    Sourced {
        value: CarriedTotals {
            brought_forward: brought.map(zar),
            carried_forward: forward.map(zar),
        },
        page: NonZeroU32::new(page).unwrap(),
        locator: None,
        confidence: Confidence::new(950).unwrap(),
    }
}

#[test]
fn the_printed_carried_figures_become_info_events_cited_to_their_page() {
    let mut x = clean();
    x.carried = vec![
        carried_on(1, None, Some(100_000)),
        carried_on(2, Some(100_000), None),
    ];
    let events = derive(&x);
    let forwarded = find(&events, "doc_1:r1:carried-forward-1");
    assert_eq!(
        forwarded.payload(),
        &EventPayload::BalanceStated {
            role: crate::event::BalanceRole::CarriedForward,
            amount: zar(100_000),
        }
    );
    assert_eq!(forwarded.citation().page.get(), 1);
    assert_eq!(forwarded.severity(), crate::event::Severity::Info);
    let brought = find(&events, "doc_1:r1:brought-forward-2");
    assert_eq!(
        brought.payload(),
        &EventPayload::BalanceStated {
            role: crate::event::BalanceRole::BroughtForward,
            amount: zar(100_000),
        }
    );
    assert_eq!(brought.citation().page.get(), 2);
}

#[test]
fn a_broken_carried_chain_becomes_a_critical_mismatch_event() {
    let mut x = clean();
    x.carried = vec![
        carried_on(1, None, Some(100_000)),
        carried_on(2, Some(99_999), None),
    ];
    let derived = derive_events(&x, run(1), &[]).unwrap();
    assert_eq!(
        derived.validation.findings,
        [Finding::CarriedBalanceMismatch {
            after_page: 1,
            carried_forward: zar(100_000),
            brought_forward: zar(99_999),
        }]
    );
    let event = find(&derived.events, "doc_1:r1:carried-mismatch-1");
    assert_eq!(
        event.payload(),
        &EventPayload::BalanceMismatch {
            after_page: 1,
            carried_forward: zar(100_000),
            brought_forward: zar(99_999),
        }
    );
    assert_eq!(event.severity(), crate::event::Severity::Critical);
    assert_eq!(event.citation().page.get(), 1);
}
