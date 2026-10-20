// Written by Kone Tshivhinda & Claude
//! Template narration: turns events into plain, cited sentences.
//!
//! Rules (research 05 §5, ADR-0004):
//! - The narrator sees only events, never the source document.
//! - Every sentence cites the events it states; an uncited [`Sentence`] cannot
//!   be built.
//! - Templates print only numbers that are in the cited events. They never
//!   compute new ones (for example a difference), because the number guard
//!   (P3-06) would reject a figure that no event contains.
//! - No model is involved and the output is deterministic. An LLM may later
//!   smooth the prose *between* these facts (P3-05).

pub mod format;
pub mod guard;
pub mod report;

use crate::error::{Error, Result};
use crate::event::view;
use crate::event::{Event, EventId, EventPayload};
use serde::Serialize;

/// One sentence of narration and the events it is based on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Sentence {
    text: String,
    cites: Vec<EventId>,
}

impl Sentence {
    /// Builds a sentence. It must cite at least one event.
    pub fn new(text: impl Into<String>, cites: Vec<EventId>) -> Result<Self> {
        if cites.is_empty() {
            return Err(Error::InvalidNarration("a sentence must cite at least one event"));
        }
        Ok(Self {
            text: text.into(),
            cites,
        })
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cites(&self) -> &[EventId] {
        &self.cites
    }
}

/// A sequence of sentences.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Narrative {
    sentences: Vec<Sentence>,
}

impl Narrative {
    /// Wraps sentences into a narrative. Each [`Sentence`] already carries its
    /// citations, so there is nothing more to check here.
    pub fn new(sentences: Vec<Sentence>) -> Self {
        Self { sentences }
    }

    pub fn sentences(&self) -> &[Sentence] {
        &self.sentences
    }
}

/// Narrates one event as one sentence citing that event.
pub fn narrate_event(event: &Event) -> Sentence {
    Sentence {
        text: template(event.payload()),
        cites: vec![event.id().clone()],
    }
}

/// Narrates the current (not superseded) events in timeline order, one
/// sentence each. Summary and grouped-report modes come later (P3-07).
pub fn narrate_timeline(events: &[Event]) -> Narrative {
    Narrative {
        sentences: view::timeline(events).into_iter().map(narrate_event).collect(),
    }
}

/// The wording for each kind of event. No wildcard arm: a new `EventPayload`
/// variant will not compile until it has a template.
fn template(payload: &EventPayload) -> String {
    use format::{amount, date, duplicate_reason, quantity_milli, rate_bps, tax_category, total_field};
    match payload {
        EventPayload::SupplierIdentified { name, vat_number } => match vat_number {
            Some(vat) => format!("The supplier is {name} (VAT number {vat})."),
            None => format!("The supplier is {name}."),
        },
        EventPayload::LineItemBilled {
            description,
            quantity_milli: qty,
            unit_price,
            line_total,
            tax_category: category,
        } => format!(
            "Billed {} × {description} at {} each, {} in total ({}).",
            quantity_milli(*qty),
            amount(*unit_price),
            amount(*line_total),
            tax_category(*category),
        ),
        EventPayload::LineTotalMismatch {
            line,
            stated,
            recomputed,
        } => format!(
            "On line {line} the stated total of {} does not match the recomputed {}.",
            amount(*stated),
            amount(*recomputed),
        ),
        EventPayload::VatCalculated {
            category,
            taxable,
            rate_bps: rate,
            vat,
        } => format!(
            "VAT on {} supplies of {} at {} is {}.",
            tax_category(*category),
            amount(*taxable),
            rate_bps(*rate),
            amount(*vat),
        ),
        EventPayload::TotalsStated { net, vat, gross } => format!(
            "The document states a net total of {}, VAT of {} and a gross total of {}.",
            amount(*net),
            amount(*vat),
            amount(*gross),
        ),
        EventPayload::TotalsMismatch {
            field,
            stated,
            recomputed,
        } => format!(
            "The stated {} of {} does not match the recomputed {}.",
            total_field(*field),
            amount(*stated),
            amount(*recomputed),
        ),
        EventPayload::DueDateStated { due, terms } => match terms {
            Some(terms) => format!("Payment is due on {} (terms: {terms}).", date(*due)),
            None => format!("Payment is due on {}.", date(*due)),
        },
        EventPayload::CreditNoteReferenced {
            original_invoice_number,
            amount: credit,
        } => format!(
            "A credit note of {} refers to invoice {original_invoice_number}.",
            amount(*credit),
        ),
        EventPayload::ComplianceIssueFound { field, message } => format!(
            "Compliance issue with {field}: {}.",
            message.trim().trim_end_matches('.'),
        ),
        EventPayload::PossibleDuplicate {
            matching_document,
            reason,
        } => format!(
            "This document may duplicate {}: {}.",
            matching_document.as_str(),
            duplicate_reason(*reason),
        ),
    }
}

#[cfg(test)]
mod tests;
