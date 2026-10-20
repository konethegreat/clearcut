// Written by Kone Tshivhinda & Claude
use super::*;
use crate::event::{
    Citation, Confidence, DocumentId, DuplicateReason, Event, EventGroup, EventId, EventParts, EventPayload,
    Extractor, TotalField,
};
use crate::narration::guard::check;
use crate::narration::{narrate_event, narrate_timeline};
use crate::{Money, TaxCategory};
use std::num::NonZeroU32;
use time::macros::date as d;
use time::Date;

fn zar(minor: i64) -> Money {
    Money::zar(minor)
}

fn mk(id: &str, when: Option<Date>, payload: EventPayload, supersedes: Option<&str>) -> Event {
    Event::new(EventParts {
        id: EventId::new(id),
        date: when,
        payload,
        citation: Citation {
            document: DocumentId::new("doc_1"),
            page: NonZeroU32::new(1).unwrap(),
            locator: None,
        },
        extractor: Extractor::System,
        confidence: Confidence::new(1000).unwrap(),
        supersedes: supersedes.map(EventId::new),
    })
    .unwrap()
}

fn supplier() -> EventPayload {
    EventPayload::SupplierIdentified {
        name: "Acme (Pty) Ltd".into(),
        vat_number: Some("4123456789".into()),
    }
}

fn line_item() -> EventPayload {
    EventPayload::LineItemBilled {
        description: "Consulting".into(),
        quantity_milli: 10_000,
        unit_price: zar(150_000),
        line_total: zar(1_500_000),
        tax_category: TaxCategory::Standard,
    }
}

fn vat() -> EventPayload {
    EventPayload::VatCalculated {
        category: TaxCategory::Standard,
        taxable: zar(1_500_000),
        rate_bps: 1_500,
        vat: zar(225_000),
    }
}

fn totals() -> EventPayload {
    EventPayload::TotalsStated {
        net: zar(1_500_000),
        vat: zar(225_000),
        gross: zar(1_725_000),
    }
}

fn mismatch() -> EventPayload {
    EventPayload::TotalsMismatch {
        field: TotalField::Vat,
        stated: zar(225_000),
        recomputed: zar(225_001),
    }
}

fn due() -> EventPayload {
    EventPayload::DueDateStated {
        due: d!(2026 - 04 - 30),
        terms: Some("30 days".into()),
    }
}

fn credit() -> EventPayload {
    EventPayload::CreditNoteReferenced {
        original_invoice_number: "INV-0042".into(),
        amount: zar(50_000),
    }
}

fn compliance() -> EventPayload {
    EventPayload::ComplianceIssueFound {
        field: "vat_number".into(),
        message: "supplier VAT number is missing".into(),
    }
}

fn duplicate() -> EventPayload {
    EventPayload::PossibleDuplicate {
        matching_document: DocumentId::new("doc_9"),
        reason: DuplicateReason::ContentHash,
    }
}

/// One event of every kind, deliberately listed out of reading order.
fn rich_events() -> Vec<Event> {
    vec![
        mk("dup", None, duplicate(), None),
        mk("line", Some(d!(2026 - 03 - 31)), line_item(), None),
        mk("mismatch", None, mismatch(), None),
        mk("credit", None, credit(), None),
        mk("vat", Some(d!(2026 - 03 - 31)), vat(), None),
        mk("comp", None, compliance(), None),
        mk("due", None, due(), None),
        mk("totals", Some(d!(2026 - 03 - 31)), totals(), None),
        mk("sup", Some(d!(2026 - 03 - 31)), supplier(), None),
    ]
}

fn cited(report: &Report) -> Vec<&str> {
    report
        .sections()
        .iter()
        .flat_map(|s| s.sentences())
        .map(|s| s.cites()[0].as_str())
        .collect()
}

// ---------- titles ----------

#[test]
fn every_group_has_a_heading() {
    let expected = [
        (EventGroup::Document, "Document"),
        (EventGroup::Parties, "Parties"),
        (EventGroup::Charges, "Charges"),
        (EventGroup::Tax, "VAT"),
        (EventGroup::Totals, "Totals"),
        (EventGroup::Terms, "Payment terms"),
        (EventGroup::Adjustments, "Adjustments"),
        (EventGroup::Payments, "Payments"),
        (EventGroup::Compliance, "Compliance"),
        (EventGroup::Risk, "Risks"),
    ];
    for (group, title) in expected {
        assert_eq!(super::group_heading(group), title, "{group:?}");
    }
    assert_eq!(expected.len(), EventGroup::ALL.len());
}

// ---------- timeline ----------

#[test]
fn timeline_mode_is_one_untitled_section_of_the_timeline_sentences() {
    let events = rich_events();
    let report = narrate(&events, NarrationMode::Timeline);
    assert_eq!(report.mode(), NarrationMode::Timeline);
    assert_eq!(report.sections().len(), 1);
    assert_eq!(report.sections()[0].group(), None);
    assert_eq!(report.sections()[0].heading(), None);
    assert_eq!(report.to_narrative(), narrate_timeline(&events));
}

// ---------- grouped report ----------

#[test]
fn grouped_mode_has_one_titled_section_per_group_in_taxonomy_order() {
    let report = narrate(&rich_events(), NarrationMode::GroupedReport);
    let groups: Vec<Option<EventGroup>> = report.sections().iter().map(Section::group).collect();
    assert_eq!(
        groups,
        [
            Some(EventGroup::Parties),
            Some(EventGroup::Charges),
            Some(EventGroup::Tax),
            Some(EventGroup::Totals),
            Some(EventGroup::Terms),
            Some(EventGroup::Adjustments),
            Some(EventGroup::Compliance),
            Some(EventGroup::Risk),
        ]
    );
    let headings: Vec<Option<&str>> = report.sections().iter().map(Section::heading).collect();
    assert_eq!(headings[0], Some("Parties"));
    assert_eq!(headings[3], Some("Totals"));
}

#[test]
fn grouped_mode_orders_events_inside_a_group_by_date() {
    let events = vec![
        mk("late", Some(d!(2026 - 04 - 02)), supplier(), None),
        mk("early", Some(d!(2026 - 04 - 01)), supplier(), None),
    ];
    let report = narrate(&events, NarrationMode::GroupedReport);
    assert_eq!(report.sections().len(), 1);
    assert_eq!(cited(&report), ["early", "late"]);
}

#[test]
fn grouped_mode_puts_both_totals_events_in_the_totals_section() {
    let events = vec![
        mk("mismatch", None, mismatch(), None),
        mk("totals", None, totals(), None),
    ];
    let report = narrate(&events, NarrationMode::GroupedReport);
    assert_eq!(report.sections().len(), 1);
    assert_eq!(report.sections()[0].sentences().len(), 2);
}

// ---------- summary ----------

#[test]
fn summary_leads_with_the_key_facts_then_what_needs_attention() {
    let report = narrate(&rich_events(), NarrationMode::Summary);
    // Supplier, stated totals, due date, credit note; then Critical, then
    // Warnings (reading order: compliance and duplicate are both undated, so
    // taxonomy group order puts Compliance before Risk).
    assert_eq!(
        cited(&report),
        ["sup", "totals", "due", "credit", "mismatch", "comp", "dup"]
    );
    assert_eq!(report.sections().len(), 1);
    assert_eq!(report.sections()[0].heading(), None);
}

#[test]
fn summary_leaves_out_line_detail() {
    let events = vec![mk("line", None, line_item(), None), mk("vat", None, vat(), None)];
    assert!(narrate(&events, NarrationMode::Summary).sections().is_empty());
}

#[test]
fn summary_uses_template_sentences_unchanged() {
    let events = vec![mk("sup", None, supplier(), None)];
    let report = narrate(&events, NarrationMode::Summary);
    assert_eq!(report.sections()[0].sentences(), [narrate_event(&events[0])]);
}

#[test]
fn summary_never_states_a_count_or_other_derived_number() {
    // A sentence like "3 issues were found" would contain a number that is in
    // no event, which the number guard would reject. Every summary sentence
    // is therefore a plain event sentence.
    let events = rich_events();
    let report = narrate(&events, NarrationMode::Summary);
    for sentence in report.sections()[0].sentences() {
        assert_eq!(sentence.cites().len(), 1);
    }
}

// ---------- all modes ----------

#[test]
fn superseded_events_are_left_out_of_every_mode() {
    let events = vec![
        mk("old", None, supplier(), None),
        mk("new", None, supplier(), Some("old")),
    ];
    for mode in [
        NarrationMode::Summary,
        NarrationMode::Timeline,
        NarrationMode::GroupedReport,
    ] {
        assert_eq!(cited(&narrate(&events, mode)), ["new"], "{mode:?}");
    }
}

#[test]
fn no_events_gives_no_sections_in_any_mode() {
    for mode in [
        NarrationMode::Summary,
        NarrationMode::Timeline,
        NarrationMode::GroupedReport,
    ] {
        let report = narrate(&[], mode);
        assert!(report.sections().is_empty(), "{mode:?}");
        assert!(report.to_narrative().sentences().is_empty());
    }
}

#[test]
fn every_mode_passes_the_number_guard() {
    let events = rich_events();
    for mode in [
        NarrationMode::Summary,
        NarrationMode::Timeline,
        NarrationMode::GroupedReport,
    ] {
        let narrative = narrate(&events, mode).to_narrative();
        assert!(!narrative.sentences().is_empty(), "{mode:?}");
        assert_eq!(check(&narrative, &events), Vec::new(), "{mode:?}");
    }
}

#[test]
fn flattening_keeps_section_order() {
    let report = narrate(&rich_events(), NarrationMode::GroupedReport);
    let flattened: Vec<String> = report
        .to_narrative()
        .sentences()
        .iter()
        .map(|s| s.cites()[0].as_str().to_owned())
        .collect();
    let in_sections: Vec<String> = cited(&report).into_iter().map(str::to_owned).collect();
    assert_eq!(flattened, in_sections);
}

#[test]
fn narration_is_deterministic_in_every_mode() {
    let events = rich_events();
    for mode in [
        NarrationMode::Summary,
        NarrationMode::Timeline,
        NarrationMode::GroupedReport,
    ] {
        assert_eq!(narrate(&events, mode), narrate(&events, mode));
    }
}

// ---------- serde ----------

#[test]
fn modes_use_snake_case_names() {
    assert_eq!(
        serde_json::to_string(&NarrationMode::GroupedReport).unwrap(),
        "\"grouped_report\""
    );
    let parsed: NarrationMode = serde_json::from_str("\"summary\"").unwrap();
    assert_eq!(parsed, NarrationMode::Summary);
}

#[test]
fn a_report_serialises_with_headings_and_cited_sentences() {
    let events = vec![mk("e1", None, supplier(), None)];
    let value = serde_json::to_value(narrate(&events, NarrationMode::GroupedReport)).unwrap();
    assert_eq!(
        value,
        serde_json::json!({
            "mode": "grouped_report",
            "sections": [{
                "group": "parties",
                "heading": "Parties",
                "sentences": [{
                    "text": "The supplier is Acme (Pty) Ltd (VAT number 4123456789).",
                    "cites": ["e1"]
                }]
            }]
        })
    );
    let flat = serde_json::to_value(narrate(&events, NarrationMode::Timeline)).unwrap();
    assert_eq!(flat["sections"][0]["group"], serde_json::Value::Null);
    assert_eq!(flat["sections"][0]["heading"], serde_json::Value::Null);
}
