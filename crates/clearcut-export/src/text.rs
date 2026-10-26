// Written by Kone Tshivhinda & Claude
//! The text formats: JSON, CSV, Markdown, HTML and plain text.

use clearcut_core::event::Event;
use clearcut_core::export::{ExportModel, ModelSentence};
use clearcut_core::narration::report::NarrationMode;
use serde_json::{json, Value};

/// The canonical file: the document, its events and the grouped report.
pub fn json(model: &ExportModel, events: &[Event]) -> String {
    let value = json!({
        "document": {
            "id": model.document.id,
            "status": model.document.status,
            "page_count": model.document.page_count,
            "original_filename": model.document.filename,
        },
        "events": events,
        "report": {
            "mode": serde_json::to_value(NarrationMode::GroupedReport).unwrap_or(Value::Null),
            "sections": model.sections.iter().map(|section| json!({
                "heading": section.heading,
                "sentences": section.sentences.iter().map(|s| json!({ "text": s.text, "cites": s.cites })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        },
    });
    serde_json::to_string_pretty(&value).unwrap_or_default()
}

/// One cell of a CSV, quoted when it must be. A cell that a spreadsheet would run as a formula
/// (it starts with `=`, `+`, `-` or `@`) gets a leading apostrophe, so text read off an
/// uploaded document cannot become a command in the reader's spreadsheet.
pub fn cell(text: &str) -> String {
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

/// One row per event: what it is, when, where it was read and the sentence that tells it.
pub fn csv(model: &ExportModel) -> String {
    let mut out = String::from("event_id,date,group,type,severity,confidence_per_mille,page,text\n");
    for event in &model.events {
        let row = [
            event.id.clone(),
            event.date.clone().unwrap_or_default(),
            event.group.clone(),
            event.kind.clone(),
            event.severity.clone(),
            event.confidence_per_mille.to_string(),
            event.page.to_string(),
            event.text.clone(),
        ];
        out.push_str(&row.iter().map(|c| cell(c)).collect::<Vec<_>>().join(","));
        out.push('\n');
    }
    out
}

fn one_line(text: &str) -> String {
    text.replace(['\n', '\r'], " ")
}

/// A readable report: a heading per group, a line per sentence with the events it cites.
pub fn markdown(model: &ExportModel) -> String {
    let mut out = format!(
        "# {}\n\nStatus: {}\n\n",
        one_line(&model.document.filename),
        model.document.status
    );
    for section in &model.sections {
        if let Some(heading) = &section.heading {
            out.push_str(&format!("## {heading}\n\n"));
        }
        for sentence in &section.sentences {
            out.push_str(&format!(
                "- {} _[{}]_\n",
                sentence.text,
                sentence.cites.join(", ")
            ));
        }
        out.push('\n');
    }
    out
}

/// Every character that could start markup or leave an attribute is escaped.
pub fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

fn html_sentences(out: &mut String, sentences: &[ModelSentence]) {
    out.push_str("<ul>\n");
    for sentence in sentences {
        out.push_str(&format!(
            "<li>{} <span class=\"cite\">[{}]</span></li>\n",
            esc(&sentence.text),
            esc(&sentence.cites.join(", "))
        ));
    }
    out.push_str("</ul>\n");
}

fn html_table(out: &mut String, head: &[&str], numeric: &[bool], rows: Vec<Vec<String>>) {
    out.push_str("<table>\n<thead><tr>");
    for (i, h) in head.iter().enumerate() {
        let class = if numeric.get(i) == Some(&true) {
            " class=\"num\""
        } else {
            ""
        };
        out.push_str(&format!("<th{class}>{}</th>", esc(h)));
    }
    out.push_str("</tr></thead>\n<tbody>\n");
    for row in rows {
        out.push_str("<tr>");
        for (i, c) in row.iter().enumerate() {
            let class = if numeric.get(i) == Some(&true) {
                " class=\"num\""
            } else {
                ""
            };
            out.push_str(&format!("<td{class}>{}</td>", esc(c)));
        }
        out.push_str("</tr>\n");
    }
    out.push_str("</tbody>\n</table>\n");
}

const STYLE: &str = "body{font:16px/1.5 system-ui,-apple-system,'Segoe UI',Roboto,sans-serif;max-width:60rem;margin:2rem auto;padding:0 1rem;color:#1d2327}\
h1{font-size:1.6rem;margin:0 0 .25rem}h2{font-size:1.15rem;margin:1.8rem 0 .5rem;border-bottom:1px solid #d9d6cf;padding-bottom:.2rem}\
table{border-collapse:collapse;width:100%;margin:.5rem 0}th,td{text-align:left;padding:.35rem .6rem;border-bottom:1px solid #e4e1da;vertical-align:top}\
th{font-size:.8rem;text-transform:uppercase;letter-spacing:.03em;color:#5f6b73}.num{text-align:right;white-space:nowrap}\
.cite{color:#5f6b73;font-size:.8rem}.muted{color:#5f6b73}.notice{margin-top:2rem;color:#5f6b73;font-size:.85rem}";

/// One self-contained page: no script, no outside file.
pub fn html(model: &ExportModel) -> String {
    let mut out = String::new();
    out.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    out.push_str(&format!(
        "<title>{}</title>\n<style>{STYLE}</style>\n</head>\n<body>\n",
        esc(&model.document.filename)
    ));
    out.push_str(&format!(
        "<h1>{}</h1>\n<p class=\"muted\">Status: {}{}</p>\n",
        esc(&model.document.filename),
        esc(&model.document.status),
        model
            .document
            .page_count
            .map(|n| format!(" &middot; {n} page{}", if n == 1 { "" } else { "s" }))
            .unwrap_or_default()
    ));
    if !model.summary.is_empty() {
        out.push_str("<h2>Summary</h2>\n");
        html_sentences(&mut out, &model.summary);
    }
    if !model.sections.is_empty() {
        out.push_str("<h2>Report</h2>\n");
        for section in &model.sections {
            if let Some(heading) = &section.heading {
                out.push_str(&format!("<h3>{}</h3>\n", esc(heading)));
            }
            html_sentences(&mut out, &section.sentences);
        }
    }
    if !model.lines.is_empty() {
        out.push_str("<h2>Line items</h2>\n");
        html_table(
            &mut out,
            &[
                "No",
                "Description",
                "Quantity",
                "Unit price",
                "Line total",
                "Tax",
                "Page",
            ],
            &[true, false, true, true, true, false, true],
            model
                .lines
                .iter()
                .map(|l| {
                    vec![
                        l.number.to_string(),
                        l.description.clone(),
                        l.quantity.clone(),
                        l.unit_price.text.clone(),
                        l.line_total.text.clone(),
                        l.tax.clone(),
                        l.page.to_string(),
                    ]
                })
                .collect(),
        );
    }
    if !model.totals.is_empty() {
        out.push_str("<h2>Totals</h2>\n");
        html_table(
            &mut out,
            &["", "Amount", "Basis"],
            &[false, true, false],
            model
                .totals
                .iter()
                .map(|t| vec![t.label.clone(), t.amount.text.clone(), t.basis.to_string()])
                .collect(),
        );
    }
    if !model.checks.is_empty() {
        out.push_str("<h2>What the checks found</h2>\n");
        html_table(
            &mut out,
            &["Severity", "Statement", "Page"],
            &[false, false, true],
            model
                .checks
                .iter()
                .map(|c| vec![c.severity.clone(), c.text.clone(), c.page.to_string()])
                .collect(),
        );
    }
    if !model.events.is_empty() {
        out.push_str("<h2>Everything that was read</h2>\n");
        html_table(
            &mut out,
            &["What", "Statement", "Page", "Sure"],
            &[false, false, true, true],
            model
                .events
                .iter()
                .map(|e| {
                    vec![
                        format!("{} · {}", e.group, e.kind.replace('_', " ")),
                        e.text.clone(),
                        e.page.to_string(),
                        format!("{}%", e.confidence_per_mille / 10),
                    ]
                })
                .collect(),
        );
    }
    if !model.sources.is_empty() {
        out.push_str("<h2>Where it was read</h2>\n");
        html_table(
            &mut out,
            &["Page", "Events", "Least sure", "Read by"],
            &[true, true, true, false],
            model
                .sources
                .iter()
                .map(|s| {
                    vec![
                        s.page.to_string(),
                        s.events.to_string(),
                        format!("{}%", s.lowest_confidence_per_mille / 10),
                        s.read_by.join(", "),
                    ]
                })
                .collect(),
        );
    }
    out.push_str(&format!(
        "<p class=\"notice\">{}</p>\n</body>\n</html>\n",
        esc(model.notice)
    ));
    out
}

fn underlined(out: &mut String, title: &str, mark: char) {
    out.push_str(title);
    out.push('\n');
    out.push_str(&mark.to_string().repeat(title.chars().count().max(3)));
    out.push_str("\n\n");
}

/// Plain text for an email or a terminal: headings underlined, one line per fact.
pub fn plain(model: &ExportModel) -> String {
    let mut out = String::new();
    underlined(&mut out, &one_line(&model.document.filename), '=');
    out.push_str(&format!("Status: {}\n", model.document.status));
    if let Some(pages) = model.document.page_count {
        out.push_str(&format!("Pages: {pages}\n"));
    }
    out.push('\n');
    if !model.summary.is_empty() {
        underlined(&mut out, "Summary", '-');
        for s in &model.summary {
            out.push_str(&format!("* {}\n", s.text));
        }
        out.push('\n');
    }
    for section in &model.sections {
        if let Some(heading) = &section.heading {
            underlined(&mut out, heading, '-');
        }
        for s in &section.sentences {
            out.push_str(&format!("* {} [{}]\n", s.text, s.cites.join(", ")));
        }
        out.push('\n');
    }
    if !model.lines.is_empty() {
        underlined(&mut out, "Line items", '-');
        for l in &model.lines {
            out.push_str(&format!(
                "{}. {} | {} x {} = {} | {}\n",
                l.number, l.description, l.quantity, l.unit_price.text, l.line_total.text, l.tax
            ));
        }
        out.push('\n');
    }
    if !model.totals.is_empty() {
        underlined(&mut out, "Totals", '-');
        for t in &model.totals {
            out.push_str(&format!("{}: {} ({})\n", t.label, t.amount.text, t.basis));
        }
        out.push('\n');
    }
    if !model.checks.is_empty() {
        underlined(&mut out, "What the checks found", '-');
        for c in &model.checks {
            out.push_str(&format!("[{}] {} (page {})\n", c.severity, c.text, c.page));
        }
        out.push('\n');
    }
    out.push_str(model.notice);
    out.push('\n');
    out
}
