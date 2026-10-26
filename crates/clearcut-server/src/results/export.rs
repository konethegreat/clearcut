// Written by Kone Tshivhinda & Claude
//! The files a document is downloaded as, made from its stored events and nothing else.

use clearcut_core::event::Event;
use clearcut_core::narration::narrate_event;
use clearcut_core::narration::report::Report;
use serde_json::{json, Value};

/// A report as JSON: each section with its heading and its sentences with the events they cite.
pub(crate) fn report_json(report: &Report) -> Value {
    json!({
        "mode": serde_json::to_value(report.mode()).unwrap_or(Value::Null),
        "sections": report.sections().iter().map(|section| json!({
            "heading": section.heading(),
            "sentences": section.sentences().iter().map(|sentence| json!({
                "text": sentence.text(),
                "cites": sentence.cites().iter().map(|id| id.as_str()).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

fn name_of<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// One line per event, for a table: what it is, when, where it was read and how sure, and the
/// sentence that tells it.
pub(crate) fn rows(events: &[Event]) -> Vec<Value> {
    events
        .iter()
        .map(|event| {
            json!({
                "id": event.id().as_str(),
                "date": event.date().map(|d| d.to_string()),
                "group": name_of(&event.group()),
                "type": name_of(&event.event_type()),
                "severity": name_of(&event.severity()),
                "confidence_per_mille": event.confidence().get(),
                "page": event.citation().page.get(),
                "text": narrate_event(event).text(),
            })
        })
        .collect()
}
