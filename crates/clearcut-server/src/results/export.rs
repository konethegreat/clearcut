// Written by Kone Tshivhinda & Claude
//! The files a document is downloaded as, made from its stored events and nothing else.

use clearcut_core::event::Event;
use clearcut_core::narration::narrate_event;
use clearcut_core::narration::report::{narrate, NarrationMode, Report};
use clearcut_store::DocumentInfo;
use serde_json::{json, Value};
use uuid::Uuid;

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

/// The canonical file: the document, its events and the grouped report.
pub(crate) fn json(info: &DocumentInfo, events: &[Event], id: Uuid) -> String {
    let report = narrate(events, NarrationMode::GroupedReport);
    let value = json!({
        "document": {
            "id": id.to_string(),
            "status": info.status.as_str(),
            "page_count": info.page_count,
            "original_filename": info.original_filename,
        },
        "events": events,
        "report": report_json(&report),
    });
    serde_json::to_string_pretty(&value).unwrap_or_default()
}

/// One cell of a CSV, quoted when it must be. A cell that a spreadsheet would run as a formula
/// (it starts with `=`, `+`, `-` or `@`) gets a leading apostrophe, so text read off an
/// uploaded document cannot become a command in the reader's spreadsheet.
fn cell(text: &str) -> String {
    let text = if text.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{text}")
    } else {
        text.to_string()
    };
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text
    }
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

/// One row per event: what it is, when, where it was read and the sentence that tells it.
pub(crate) fn csv(events: &[Event]) -> String {
    let mut out = String::from("event_id,date,group,type,severity,confidence_per_mille,page,text\n");
    for event in events {
        let row = [
            event.id().as_str().to_string(),
            event.date().map(|d| d.to_string()).unwrap_or_default(),
            name_of(&event.group()),
            name_of(&event.event_type()),
            name_of(&event.severity()),
            event.confidence().get().to_string(),
            event.citation().page.to_string(),
            narrate_event(event).text().to_string(),
        ];
        out.push_str(&row.iter().map(|c| cell(c)).collect::<Vec<_>>().join(","));
        out.push('\n');
    }
    out
}

/// A readable report: a heading per group, a line per sentence with the events it cites.
pub(crate) fn markdown(info: &DocumentInfo, events: &[Event]) -> String {
    let mut out = format!(
        "# {}\n\nStatus: {}\n\n",
        info.original_filename.replace(['\n', '\r'], " "),
        info.status.as_str()
    );
    let report = narrate(events, NarrationMode::GroupedReport);
    for section in report.sections() {
        if let Some(heading) = section.heading() {
            out.push_str(&format!("## {heading}\n\n"));
        }
        for sentence in section.sentences() {
            let cites: Vec<&str> = sentence.cites().iter().map(|id| id.as_str()).collect();
            out.push_str(&format!("- {} _[{}]_\n", sentence.text(), cites.join(", ")));
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cell_is_quoted_when_it_must_be() {
        assert_eq!(cell("plain"), "plain");
        assert_eq!(cell("a,b"), "\"a,b\"");
        assert_eq!(cell("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(cell("two\nlines"), "\"two\nlines\"");
    }

    #[test]
    fn text_from_a_document_cannot_become_a_formula() {
        for danger in ["=1+1", "+cmd", "-2+3", "@SUM(A1)", "\tx"] {
            assert!(cell(danger).starts_with('\''), "{danger:?}");
        }
        // A quoted cell that starts with one is still defused inside the quotes.
        assert_eq!(
            cell("=HYPERLINK(\"x\",\"y\")"),
            "\"'=HYPERLINK(\"\"x\"\",\"\"y\"\")\""
        );
    }
}
