// Written by Kone Tshivhinda & Claude
//! Narration modes (research 05 §5, rule 6): the same events told as a short
//! summary, a chronological timeline, or a report grouped by topic.
//!
//! Every mode is built from the plain template sentences of
//! [`narrate_event`], so each sentence still cites its event and every mode
//! passes the number guard. In particular, a mode never adds a sentence of
//! its own such as "3 issues were found": a count is in no event, so the guard
//! would reject it.
//!
//! All modes use only current events (superseded ones are skipped) and are
//! fully deterministic.

use super::{narrate_event, narrate_timeline, Narrative, Sentence};
use crate::event::view;
use crate::event::{Event, EventGroup, EventType, Severity};
use serde::{Deserialize, Serialize};

/// How the events are told.
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NarrationMode {
    /// The key facts, then anything that needs attention. Line detail is left out.
    Summary,
    /// Every event, oldest first, undated facts last.
    Timeline,
    /// Every event under a heading per topic, in taxonomy order.
    GroupedReport,
}

/// A run of sentences, optionally under a heading.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Section {
    group: Option<EventGroup>,
    heading: Option<&'static str>,
    sentences: Vec<Sentence>,
}

impl Section {
    fn untitled(sentences: Vec<Sentence>) -> Self {
        Self {
            group: None,
            heading: None,
            sentences,
        }
    }

    fn for_group(group: EventGroup, sentences: Vec<Sentence>) -> Self {
        Self {
            group: Some(group),
            heading: Some(group_heading(group)),
            sentences,
        }
    }

    /// The taxonomy group, for a grouped report; `None` otherwise.
    pub fn group(&self) -> Option<EventGroup> {
        self.group
    }

    /// The heading to print above the section, if it has one.
    pub fn heading(&self) -> Option<&'static str> {
        self.heading
    }

    pub fn sentences(&self) -> &[Sentence] {
        &self.sentences
    }
}

/// A narration in one of the [`NarrationMode`]s. Sections are never empty, and
/// there are no sections at all when there is nothing to narrate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    mode: NarrationMode,
    sections: Vec<Section>,
}

impl Report {
    pub fn mode(&self) -> NarrationMode {
        self.mode
    }

    pub fn sections(&self) -> &[Section] {
        &self.sections
    }

    /// All sentences in reading order with the headings dropped. This is what
    /// goes to the number guard, whatever the mode. (Headings are fixed words
    /// with no figures, so there is nothing in them to check.)
    pub fn to_narrative(&self) -> Narrative {
        Narrative::new(
            self.sections
                .iter()
                .flat_map(|section| section.sentences.iter().cloned())
                .collect(),
        )
    }
}

/// Narrates the current events in the given mode.
pub fn narrate(events: &[Event], mode: NarrationMode) -> Report {
    let sections = match mode {
        NarrationMode::Timeline => untitled(narrate_timeline(events).sentences().to_vec()),
        NarrationMode::Summary => untitled(summary_sentences(events)),
        NarrationMode::GroupedReport => view::by_group(events)
            .into_iter()
            .map(|(group, group_events)| {
                let sentences = group_events.into_iter().map(narrate_event).collect();
                Section::for_group(group, sentences)
            })
            .collect(),
    };
    Report { mode, sections }
}

/// One untitled section, or none at all when there are no sentences.
fn untitled(sentences: Vec<Sentence>) -> Vec<Section> {
    if sentences.is_empty() {
        Vec::new()
    } else {
        vec![Section::untitled(sentences)]
    }
}

/// The key facts first (supplier, stated totals, due date, credit notes),
/// then what needs attention, most severe first. Within each rank the order is
/// the timeline's.
fn summary_sentences(events: &[Event]) -> Vec<Sentence> {
    let mut picked: Vec<(u8, &Event)> = view::timeline(events)
        .into_iter()
        .filter_map(|event| summary_rank(event.event_type()).map(|rank| (rank, event)))
        .collect();
    // `sort_by_key` is stable, so events of equal rank keep timeline order.
    picked.sort_by_key(|(rank, _)| *rank);
    picked
        .into_iter()
        .map(|(_, event)| narrate_event(event))
        .collect()
}

/// Where a kind of event sits in a summary, or `None` to leave it out.
fn summary_rank(event_type: EventType) -> Option<u8> {
    match event_type {
        EventType::SupplierIdentified => Some(0),
        EventType::TotalsStated => Some(1),
        EventType::DueDateStated => Some(2),
        EventType::CreditNoteReferenced => Some(3),
        // Everything else is included only if it needs attention.
        other => match other.severity() {
            Severity::Critical => Some(4),
            Severity::Warning => Some(5),
            Severity::Info => None,
        },
    }
}

/// The heading printed above a group in a grouped report.
fn group_heading(group: EventGroup) -> &'static str {
    match group {
        EventGroup::Document => "Document",
        EventGroup::Parties => "Parties",
        EventGroup::Charges => "Charges",
        EventGroup::Tax => "VAT",
        EventGroup::Totals => "Totals",
        EventGroup::Terms => "Payment terms",
        EventGroup::Adjustments => "Adjustments",
        EventGroup::Payments => "Payments",
        EventGroup::Compliance => "Compliance",
        EventGroup::Risk => "Risks",
    }
}

#[cfg(test)]
mod tests;
