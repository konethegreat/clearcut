// Written by Kone Tshivhinda & Claude
use super::*;
use crate::event::{
    Citation, Confidence, DocumentId, EventGroup, EventId, EventParts, EventPayload, Extractor, Severity,
    TotalField,
};
use crate::Money;
use std::num::NonZeroU32;
use time::macros::date;
use time::Date;

fn payload_totals() -> EventPayload {
    EventPayload::TotalsMismatch {
        field: TotalField::Vat,
        stated: Money::zar(100),
        recomputed: Money::zar(101),
    }
}

fn payload_supplier() -> EventPayload {
    EventPayload::SupplierIdentified {
        name: "Acme (Pty) Ltd".into(),
        vat_number: None,
    }
}

fn payload_compliance() -> EventPayload {
    EventPayload::ComplianceIssueFound {
        field: "vat_number".into(),
        message: "supplier VAT number is missing".into(),
    }
}

fn mk(id: &str, doc: &str, date: Option<Date>, payload: EventPayload) -> Event {
    Event::new(EventParts {
        id: EventId::new(id),
        date,
        payload,
        citation: Citation {
            document: DocumentId::new(doc),
            page: NonZeroU32::new(1).unwrap(),
            locator: None,
        },
        extractor: Extractor::System,
        confidence: Confidence::new(1000).unwrap(),
        supersedes: None,
    })
    .unwrap()
}

fn correcting(old: &str, mut e: Event) -> Event {
    // Events are immutable, so rebuild with a `supersedes` link.
    e = Event::new(EventParts {
        id: e.id().clone(),
        date: e.date(),
        payload: e.payload().clone(),
        citation: e.citation().clone(),
        extractor: e.extractor().clone(),
        confidence: e.confidence(),
        supersedes: Some(EventId::new(old)),
    })
    .unwrap();
    e
}

fn ids<'a>(events: impl IntoIterator<Item = &'a Event>) -> Vec<&'a str> {
    events.into_iter().map(|e| e.id().as_str()).collect()
}

// ---------- current ----------

#[test]
fn current_drops_events_that_were_superseded() {
    let events = vec![
        mk("a", "d1", None, payload_supplier()),
        mk("b", "d1", None, payload_totals()),
        correcting("b", mk("b2", "d1", None, payload_totals())),
    ];
    assert_eq!(ids(current(&events)), ["a", "b2"]);
}

#[test]
fn current_follows_a_chain_of_corrections() {
    let events = vec![
        mk("v1", "d1", None, payload_totals()),
        correcting("v1", mk("v2", "d1", None, payload_totals())),
        correcting("v2", mk("v3", "d1", None, payload_totals())),
    ];
    assert_eq!(ids(current(&events)), ["v3"]);
}

#[test]
fn current_ignores_a_link_to_an_unknown_event() {
    let events = vec![correcting("not-here", mk("x", "d1", None, payload_totals()))];
    assert_eq!(ids(current(&events)), ["x"]);
}

#[test]
fn current_keeps_the_input_order() {
    let events = vec![
        mk("z", "d1", None, payload_totals()),
        mk("a", "d1", None, payload_totals()),
    ];
    assert_eq!(ids(current(&events)), ["z", "a"]);
}

// ---------- timeline ----------

#[test]
fn timeline_orders_by_date_then_group_then_id_with_undated_last() {
    let events = vec![
        mk("undated", "d1", None, payload_supplier()),
        mk("late", "d1", Some(date!(2026 - 04 - 01)), payload_supplier()),
        mk("tie-totals", "d1", Some(date!(2026 - 03 - 31)), payload_totals()),
        mk(
            "tie-parties-b",
            "d1",
            Some(date!(2026 - 03 - 31)),
            payload_supplier(),
        ),
        mk(
            "tie-parties-a",
            "d1",
            Some(date!(2026 - 03 - 31)),
            payload_supplier(),
        ),
    ];
    assert_eq!(
        ids(timeline(&events)),
        ["tie-parties-a", "tie-parties-b", "tie-totals", "late", "undated"]
    );
}

#[test]
fn timeline_leaves_out_superseded_events() {
    let events = vec![
        mk("old", "d1", Some(date!(2026 - 03 - 31)), payload_totals()),
        correcting(
            "old",
            mk("new", "d1", Some(date!(2026 - 03 - 31)), payload_totals()),
        ),
    ];
    assert_eq!(ids(timeline(&events)), ["new"]);
}

// ---------- by group ----------

#[test]
fn by_group_follows_taxonomy_order_and_skips_empty_groups() {
    let events = vec![
        mk("t", "d1", None, payload_totals()),
        mk("p", "d1", None, payload_supplier()),
        mk("c", "d1", None, payload_compliance()),
    ];
    let view = by_group(&events);
    let groups: Vec<EventGroup> = view.iter().map(|(g, _)| *g).collect();
    assert_eq!(
        groups,
        [EventGroup::Parties, EventGroup::Totals, EventGroup::Compliance]
    );
    assert_eq!(ids(view[1].1.iter().copied()), ["t"]);
}

#[test]
fn by_group_orders_events_inside_a_group_by_date_then_id() {
    let events = vec![
        mk("b", "d1", Some(date!(2026 - 03 - 02)), payload_totals()),
        mk("c", "d1", Some(date!(2026 - 03 - 01)), payload_totals()),
        mk("a", "d1", Some(date!(2026 - 03 - 02)), payload_totals()),
    ];
    let view = by_group(&events);
    assert_eq!(view.len(), 1);
    assert_eq!(ids(view[0].1.iter().copied()), ["c", "a", "b"]);
}

// ---------- by document ----------

#[test]
fn by_document_sorts_documents_by_id() {
    let events = vec![
        mk("e1", "doc_b", None, payload_totals()),
        mk("e2", "doc_a", None, payload_supplier()),
        mk("e3", "doc_b", None, payload_supplier()),
    ];
    let view = by_document(&events);
    let docs: Vec<&str> = view.iter().map(|(d, _)| d.as_str()).collect();
    assert_eq!(docs, ["doc_a", "doc_b"]);
    assert_eq!(ids(view[1].1.iter().copied()), ["e3", "e1"]); // parties before totals
}

// ---------- by severity ----------

#[test]
fn by_severity_lists_the_most_severe_first_and_skips_empty() {
    let events = vec![
        mk("info", "d1", None, payload_supplier()),
        mk("crit", "d1", None, payload_totals()),
        mk("warn", "d1", None, payload_compliance()),
    ];
    let view = by_severity(&events);
    let levels: Vec<Severity> = view.iter().map(|(s, _)| *s).collect();
    assert_eq!(levels, [Severity::Critical, Severity::Warning, Severity::Info]);
    assert_eq!(ids(view[0].1.iter().copied()), ["crit"]);

    let only_info = vec![mk("i", "d1", None, payload_supplier())];
    assert_eq!(by_severity(&only_info).len(), 1);
}

// ---------- empty input ----------

#[test]
fn every_view_of_nothing_is_empty() {
    let none: Vec<Event> = Vec::new();
    assert!(current(&none).is_empty());
    assert!(timeline(&none).is_empty());
    assert!(by_group(&none).is_empty());
    assert!(by_document(&none).is_empty());
    assert!(by_severity(&none).is_empty());
}
