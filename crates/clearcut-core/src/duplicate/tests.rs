// Written by Kone Tshivhinda & Claude
//! The near-duplicate rule (P2-08): the four printed facts, and only earlier documents.

use super::*;
use crate::event::{EventGroup, EventType, Severity};
use crate::extraction::derive_events;
use crate::extraction::fixtures::clean;
use crate::{Currency, Money};
use std::num::NonZeroU32;
use time::macros::date as d;

/// The key of the shared, clean fixture invoice.
fn key() -> DuplicateKey {
    DuplicateKey::of_extraction(&clean()).expect("the fixture has all four facts")
}

fn doc(name: &str) -> DocumentId {
    DocumentId::new(name)
}

fn earlier(name: &str, key: DuplicateKey) -> EarlierDocument {
    EarlierDocument {
        document: doc(name),
        key,
    }
}

/// The events of the shared fixture extraction, as the pipeline would derive them.
fn events_of(extraction: &Extraction) -> Vec<Event> {
    derive_events(extraction, NonZeroU32::MIN, &[])
        .expect("the fixture derives")
        .events
}

#[test]
fn the_key_of_a_clean_invoice_is_its_four_printed_facts() {
    let key = key();
    assert_eq!(key.supplier, "Acme (Pty) Ltd");
    assert_eq!(key.invoice_number, "INV-000001");
    assert_eq!(key.date, d!(2026 - 03 - 31));
    assert_eq!(key.gross, Money::zar(1_825_000));
}

#[test]
fn a_missing_fact_means_no_key_at_all() {
    type Change = fn(&mut Extraction);
    let cases: [(&str, Change); 4] = [
        ("supplier", |x| x.supplier = None),
        ("invoice number", |x| x.invoice_number = None),
        ("invoice date", |x| x.invoice_date = None),
        ("gross total", |x| x.stated_totals = None),
    ];
    for (fact, change) in cases {
        let mut extraction = clean();
        change(&mut extraction);
        assert!(
            DuplicateKey::of_extraction(&extraction).is_none(),
            "a missing {fact} must not claim a duplicate"
        );
    }
}

#[test]
fn the_key_of_a_stored_document_comes_from_its_current_events() {
    let events = events_of(&clean());
    assert_eq!(DuplicateKey::of_events(&events), Some(key()));
}

#[test]
fn a_corrected_fact_is_the_one_the_key_uses() {
    let mut events = events_of(&clean());
    let old = events
        .iter()
        .find(|event| event.event_type() == EventType::InvoiceNumberStated)
        .expect("the fixture states its number")
        .clone();
    let corrected = Event::new(EventParts {
        id: EventId::new("doc_1:r2:invoice-number"),
        date: Some(d!(2026 - 03 - 31)),
        payload: EventPayload::InvoiceNumberStated {
            number: "INV-999999".into(),
        },
        citation: old.citation().clone(),
        extractor: Extractor::Human {
            user: "a person".into(),
        },
        confidence: Confidence::CERTAIN,
        supersedes: Some(old.id().clone()),
    })
    .expect("a valid event");
    events.push(corrected);

    let key = DuplicateKey::of_events(&events).expect("all four facts are still there");
    assert_eq!(key.invoice_number, "INV-999999");
}

#[test]
fn when_two_invoice_numbers_are_current_the_newest_is_used() {
    let mut events = events_of(&clean());
    let old = events
        .iter()
        .find(|event| event.event_type() == EventType::InvoiceNumberStated)
        .expect("the fixture states its number")
        .clone();
    // A second number appended without superseding the first: both stay current, so the newest
    // (last stored) is the one a duplicate is claimed against.
    events.push(
        Event::new(EventParts {
            id: EventId::new("doc_1:r1:invoice-number-repair"),
            date: old.date(),
            payload: EventPayload::InvoiceNumberStated {
                number: "INV-000002".into(),
            },
            citation: old.citation().clone(),
            extractor: Extractor::Human {
                user: "a person".into(),
            },
            confidence: Confidence::CERTAIN,
            supersedes: None,
        })
        .expect("a valid event"),
    );
    let key = DuplicateKey::of_events(&events).expect("all four facts are still there");
    assert_eq!(key.invoice_number, "INV-000002");
}

#[test]
fn a_document_whose_events_do_not_carry_all_four_facts_has_no_key() {
    type Change = fn(&mut Extraction);
    let cases: [(&str, Change); 2] = [
        ("no date", |x| x.invoice_date = None),
        ("no supplier", |x| x.supplier = None),
    ];
    for (case, change) in cases {
        let mut extraction = clean();
        change(&mut extraction);
        let events = events_of(&extraction);
        assert!(
            DuplicateKey::of_events(&events).is_none(),
            "a document with {case} must not claim a duplicate"
        );
    }
}

#[test]
fn the_earliest_earlier_document_with_the_same_facts_is_the_one_named() {
    let same = key();
    let mut other = same.clone();
    other.invoice_number = "INV-000002".into();

    assert_eq!(
        find(&same, &[earlier("doc_0", other), earlier("doc_1", same.clone())]),
        Some(doc("doc_1"))
    );
    assert_eq!(
        find(
            &same,
            &[earlier("doc_0", same.clone()), earlier("doc_1", same.clone())]
        ),
        Some(doc("doc_0")),
        "when several match, the first given is the earliest"
    );
    assert_eq!(find(&same, &[]), None);
}

#[test]
fn a_difference_in_any_of_the_four_facts_is_not_a_match() {
    let same = key();
    type Change = fn(&mut DuplicateKey);
    let cases: [(&str, Change); 4] = [
        ("supplier", |k| k.supplier.push('X')),
        ("invoice number", |k| k.invoice_number.push('9')),
        ("invoice date", |k| k.date = d!(2026 - 04 - 01)),
        ("gross total", |k| k.gross = Money::zar(k.gross.minor + 1)),
    ];
    for (fact, change) in cases {
        let mut other = same.clone();
        change(&mut other);
        assert_eq!(
            find(&same, &[earlier("doc_0", other)]),
            None,
            "a different {fact} must not claim a duplicate"
        );
    }
}

#[test]
fn a_total_in_another_currency_is_not_the_same_invoice() {
    let same = key();
    let mut other = same.clone();
    other.gross = Money::new(same.gross.minor, Currency::Usd);
    assert_eq!(find(&same, &[earlier("doc_0", other)]), None);
}

#[test]
fn a_supplier_name_that_differs_in_case_is_not_the_same_invoice() {
    // Exact printed facts only: a warning must not be claimed on a guess. This is the
    // conservative side of the rule, written down so nobody "fixes" it by accident.
    let same = key();
    let mut other = same.clone();
    other.supplier = "ACME (PTY) LTD".into();
    assert_eq!(find(&same, &[earlier("doc_0", other)]), None);
}

#[test]
fn the_event_is_a_certain_warning_that_names_the_earlier_document() {
    let key = key();
    let event = possible_duplicate_event(
        &doc("doc_2"),
        NonZeroU32::new(3).expect("three is not zero"),
        &key,
        doc("doc_1"),
    )
    .expect("a valid event");

    assert_eq!(event.id().as_str(), "doc_2:r3:possible-duplicate");
    assert_eq!(event.date(), Some(key.date));
    assert_eq!(
        event.payload(),
        &EventPayload::PossibleDuplicate {
            matching_document: doc("doc_1"),
            reason: DuplicateReason::SupplierNumberDateTotal,
        }
    );
    assert_eq!(event.citation().document, doc("doc_2"));
    assert_eq!(event.citation().page, NonZeroU32::MIN);
    assert_eq!(event.citation().locator, None);
    assert_eq!(event.extractor(), &Extractor::System);
    assert_eq!(event.confidence(), Confidence::CERTAIN);
    assert_eq!(event.supersedes(), None);
    assert_eq!(event.event_type(), EventType::PossibleDuplicate);
    assert_eq!(event.group(), EventGroup::Risk);
    assert_eq!(event.severity(), Severity::Warning);
}

#[test]
fn a_document_already_flagged_stays_flagged_until_the_warning_is_corrected() {
    let mut events = events_of(&clean());
    let warning = possible_duplicate_event(&clean().document, NonZeroU32::MIN, &key(), doc("doc_0"))
        .expect("a valid event");
    events.push(warning.clone());
    assert!(already_flagged(&events));

    let corrected = Event::new(EventParts {
        id: EventId::new("doc_1:r1:possible-duplicate-2"),
        date: warning.date(),
        payload: EventPayload::ComplianceIssueFound {
            field: "duplicate".into(),
            message: "the two documents are not the same invoice".into(),
        },
        citation: warning.citation().clone(),
        extractor: Extractor::Human {
            user: "a person".into(),
        },
        confidence: Confidence::CERTAIN,
        supersedes: Some(warning.id().clone()),
    })
    .expect("a valid event");
    events.push(corrected);
    assert!(!already_flagged(&events));
}
