// Written by Kone Tshivhinda & Claude
//! The one model every export format is made from (P4-02).
//!
//! A document's events are the source of truth. This builds, **once and with no I/O**, everything
//! an export shows: the cited summary, the grouped report and the timeline, one row per event, the
//! line items, the totals, the checks that found something, and where each page was read and by
//! whom. Every format (PDF, Excel, Word, HTML, text, Markdown, CSV, JSON) only lays this out, so
//! they cannot disagree with each other, and each figure appears exactly as the narration states
//! it (the number guard has already checked the sentences against the events).
//!
//! Nothing here comes from anywhere but the events and the four facts about the document that the
//! caller passes in. Text that came off an uploaded document is **data**: a renderer must never
//! treat it as markup, a formula or an instruction.

use crate::event::{Event, EventPayload, Extractor};
use crate::narration::format;
use crate::narration::narrate_event;
use crate::narration::report::{narrate, NarrationMode, Report};
use serde::Serialize;
use std::collections::BTreeMap;

/// What the footer of every export says.
pub const NOTICE: &str =
    "Made by ClearCut from what was read off the document and checked by its own rules. \
Check figures that matter against the original.";

/// The facts about the document the events do not carry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DocumentFacts {
    pub id: String,
    pub filename: String,
    pub status: String,
    pub page_count: Option<u32>,
}

/// A sentence of the narration and the ids of the events it comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelSentence {
    pub text: String,
    pub cites: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelSection {
    pub heading: Option<String>,
    pub sentences: Vec<ModelSentence>,
}

/// An amount: the exact integer for a spreadsheet cell, and the text as the narration writes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Amount {
    /// Minor units (cents).
    pub minor: i64,
    /// `ZAR`, `USD`, ...
    pub currency: String,
    /// `R15 000.00`, `USD 10.00`.
    pub text: String,
}

fn amount(money: crate::Money) -> Amount {
    Amount {
        minor: money.minor,
        currency: money.currency.code().to_string(),
        text: format::amount(money),
    }
}

/// One line per event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EventRow {
    pub id: String,
    pub date: Option<String>,
    pub group: String,
    pub kind: String,
    pub severity: String,
    pub confidence_per_mille: u16,
    pub page: u32,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LineRow {
    pub number: u32,
    pub description: String,
    /// `10`, `2.5`.
    pub quantity: String,
    pub unit_price: Amount,
    pub line_total: Amount,
    pub tax: String,
    pub page: u32,
}

/// A total, and where it comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TotalRow {
    pub label: String,
    pub amount: Amount,
    /// `stated on the document` or `calculated by ClearCut`.
    pub basis: &'static str,
}

/// Something the checks found: a disagreement, a missing field, a possible duplicate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CheckRow {
    pub severity: String,
    pub text: String,
    pub event_id: String,
    pub page: u32,
}

/// What was read from one page, and by whom.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceRow {
    pub page: u32,
    pub events: u32,
    /// The least sure of the page's events, per mille.
    pub lowest_confidence_per_mille: u16,
    /// `claude-haiku-5-5`, `a person`, `ClearCut`, ... in the order first met.
    pub read_by: Vec<String>,
}

/// Everything an export shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExportModel {
    pub document: DocumentFacts,
    pub notice: &'static str,
    pub summary: Vec<ModelSentence>,
    pub sections: Vec<ModelSection>,
    pub timeline: Vec<ModelSentence>,
    pub events: Vec<EventRow>,
    pub lines: Vec<LineRow>,
    pub totals: Vec<TotalRow>,
    pub checks: Vec<CheckRow>,
    pub sources: Vec<SourceRow>,
}

fn sentences(report: &Report) -> Vec<ModelSentence> {
    report
        .sections()
        .iter()
        .flat_map(|section| section.sentences())
        .map(|sentence| ModelSentence {
            text: sentence.text().to_string(),
            cites: sentence
                .cites()
                .iter()
                .map(|id| id.as_str().to_string())
                .collect(),
        })
        .collect()
}

fn who(extractor: &Extractor) -> String {
    match extractor {
        Extractor::Model { name, .. } => name.clone(),
        Extractor::Human { .. } => "a person".to_string(),
        Extractor::Rule { name } => name.clone(),
        Extractor::System => "ClearCut".to_string(),
    }
}

/// Builds the model from a document's events (oldest first, as stored).
pub fn build(document: DocumentFacts, events: &[Event]) -> ExportModel {
    let sections = narrate(events, NarrationMode::GroupedReport)
        .sections()
        .iter()
        .map(|section| ModelSection {
            heading: section.heading().map(str::to_string),
            sentences: section
                .sentences()
                .iter()
                .map(|sentence| ModelSentence {
                    text: sentence.text().to_string(),
                    cites: sentence
                        .cites()
                        .iter()
                        .map(|id| id.as_str().to_string())
                        .collect(),
                })
                .collect(),
        })
        .collect();

    let mut rows = Vec::new();
    let mut lines = Vec::new();
    let mut totals = Vec::new();
    let mut checks = Vec::new();
    let mut pages: BTreeMap<u32, SourceRow> = BTreeMap::new();

    for event in events {
        let page = event.citation().page.get();
        let text = narrate_event(event).text().to_string();
        let severity = event.severity().as_str().to_string();
        rows.push(EventRow {
            id: event.id().as_str().to_string(),
            date: event.date().map(|d| d.to_string()),
            group: event.group().as_str().to_string(),
            kind: event.event_type().as_str().to_string(),
            severity: severity.clone(),
            confidence_per_mille: event.confidence().get(),
            page,
            text: text.clone(),
        });

        match event.payload() {
            EventPayload::LineItemBilled {
                description,
                quantity_milli,
                unit_price,
                line_total,
                tax_category,
            } => lines.push(LineRow {
                number: u32::try_from(lines.len() + 1).unwrap_or(u32::MAX),
                description: description.clone(),
                quantity: format::quantity_milli(*quantity_milli),
                unit_price: amount(*unit_price),
                line_total: amount(*line_total),
                tax: format::tax_category(*tax_category).to_string(),
                page,
            }),
            EventPayload::TotalsStated { net, vat, gross } => {
                for (label, money) in [("Net", net), ("VAT", vat), ("Gross", gross)] {
                    totals.push(TotalRow {
                        label: format!("{label} (stated)"),
                        amount: amount(*money),
                        basis: "stated on the document",
                    });
                }
            }
            EventPayload::VatCalculated {
                category,
                taxable,
                rate_bps,
                vat,
            } => {
                let at = format::rate_bps(*rate_bps);
                let on = format::tax_category(*category);
                totals.push(TotalRow {
                    label: format!("Taxable amount, {on} ({at})"),
                    amount: amount(*taxable),
                    basis: "calculated by ClearCut",
                });
                totals.push(TotalRow {
                    label: format!("VAT, {on} ({at})"),
                    amount: amount(*vat),
                    basis: "calculated by ClearCut",
                });
            }
            _ => {}
        }

        // Anything that is not a plain fact is something the checks found.
        if severity != "info" {
            checks.push(CheckRow {
                severity,
                text,
                event_id: event.id().as_str().to_string(),
                page,
            });
        }

        let source = pages.entry(page).or_insert_with(|| SourceRow {
            page,
            events: 0,
            lowest_confidence_per_mille: 1000,
            read_by: Vec::new(),
        });
        source.events += 1;
        source.lowest_confidence_per_mille = source.lowest_confidence_per_mille.min(event.confidence().get());
        let name = who(event.extractor());
        if !source.read_by.contains(&name) {
            source.read_by.push(name);
        }
    }

    ExportModel {
        document,
        notice: NOTICE,
        summary: sentences(&narrate(events, NarrationMode::Summary)),
        sections,
        timeline: sentences(&narrate(events, NarrationMode::Timeline)),
        events: rows,
        lines,
        totals,
        checks,
        sources: pages.into_values().collect(),
    }
}

#[cfg(test)]
mod tests;
