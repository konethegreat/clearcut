// Written by Kone Tshivhinda & Claude
use super::*;
use crate::extraction::{derive_events, fixtures};
use std::num::NonZeroU32;

fn facts() -> DocumentFacts {
    DocumentFacts {
        id: "doc_1".into(),
        filename: "invoice.pdf".into(),
        status: "done".into(),
        page_count: Some(1),
    }
}

fn model_of(extraction: &crate::extraction::Extraction) -> ExportModel {
    let events = derive_events(extraction, NonZeroU32::MIN, &[]).unwrap().events;
    build(facts(), &events)
}

#[test]
fn a_clean_invoice_gives_its_lines_totals_and_sources_and_no_checks() {
    let model = model_of(&fixtures::clean());
    assert_eq!(model.notice, NOTICE);
    assert_eq!(model.document.filename, "invoice.pdf");

    // The lines, in order, with exact integers and the text as the narration writes it.
    assert_eq!(model.lines.len(), 2);
    let first = &model.lines[0];
    assert_eq!(
        (first.number, first.description.as_str(), first.quantity.as_str()),
        (1, "Consulting", "10")
    );
    assert_eq!(
        (first.unit_price.minor, first.line_total.minor),
        (150_000, 1_500_000)
    );
    assert_eq!(first.line_total.text, "R15\u{a0}000.00");
    assert_eq!(model.lines[1].tax, "zero-rated");

    // Stated totals and the VAT ClearCut calculated, each marked for what it is.
    let stated: Vec<_> = model
        .totals
        .iter()
        .filter(|t| t.basis == "stated on the document")
        .collect();
    assert_eq!(stated.len(), 3);
    assert_eq!(stated[2].amount.minor, 1_825_000);
    assert!(model
        .totals
        .iter()
        .any(|t| t.basis == "calculated by ClearCut" && t.label.starts_with("VAT")));

    // Nothing was found wrong, one page was read, and every event is listed once.
    assert!(model.checks.is_empty(), "{:?}", model.checks);
    assert_eq!(model.sources.len(), 1);
    assert_eq!(model.sources[0].events as usize, model.events.len());
    assert!(!model.summary.is_empty() && !model.sections.is_empty() && !model.timeline.is_empty());
}

#[test]
fn what_the_checks_found_is_listed_with_its_severity() {
    let mut wrong = fixtures::clean();
    wrong.stated_totals = Some(fixtures::at(crate::InvoiceTotals {
        net: fixtures::zar(1_600_000),
        vat: fixtures::zar(225_000),
        gross: fixtures::zar(1_999_999),
    }));
    let model = model_of(&wrong);
    assert!(
        model
            .checks
            .iter()
            .any(|c| c.severity == "critical" && c.page == 1),
        "{:?}",
        model.checks
    );
}

#[test]
fn every_sentence_cites_events_that_are_in_the_events_table() {
    let model = model_of(&fixtures::clean());
    let ids: Vec<&str> = model.events.iter().map(|e| e.id.as_str()).collect();
    let cited: Vec<&String> = model
        .summary
        .iter()
        .chain(model.sections.iter().flat_map(|s| &s.sentences))
        .chain(model.timeline.iter())
        .flat_map(|s| &s.cites)
        .collect();
    assert!(!cited.is_empty());
    assert!(cited.iter().all(|id| ids.contains(&id.as_str())));
}

#[test]
fn the_event_names_in_files_are_the_serialised_names() {
    use crate::event::{EventGroup, EventType, Severity};
    for group in EventGroup::ALL {
        assert_eq!(serde_json::to_value(group).unwrap(), group.as_str());
    }
    for kind in EventType::ALL {
        assert_eq!(serde_json::to_value(kind).unwrap(), kind.as_str());
    }
    for severity in [Severity::Info, Severity::Warning, Severity::Critical] {
        assert_eq!(serde_json::to_value(severity).unwrap(), severity.as_str());
    }
}
