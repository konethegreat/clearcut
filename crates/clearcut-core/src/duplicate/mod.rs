// Written by Kone Tshivhinda & Claude
//! Near-duplicate detection (P2-08, LAUNCH item 6b).
//!
//! The same invoice scanned or saved twice arrives as two uploads with different bytes, so
//! intake's hash check cannot see it. The rule here is deterministic and uses printed facts
//! only: the supplier's name, the invoice number, the invoice date and the gross total must
//! all be present **and** all be equal. If any of the four is missing, no duplicate is
//! claimed.
//!
//! **Where the date comes from.** There is no event that states the invoice date as a fact,
//! but every derived event already carries it in `Event::date` (ADR-0013's reading is dated
//! with the invoice date by `derive_events`). So a document that was read before this rule
//! existed can still be compared: its current events say what it printed.
//!
//! **Only earlier documents count.** The store lists candidates oldest first, so two reads can
//! never point at each other. The result is one `possible_duplicate` warning event naming the
//! earliest matching document. A warning never changes what the document says and never stops
//! it being `done` (ADR-0004, ADR-0008).

use crate::error::Result;
use crate::event::{
    Citation, Confidence, DocumentId, DuplicateReason, Event, EventId, EventParts, EventPayload, Extractor,
};
use crate::extraction::Extraction;
use crate::Money;
use std::num::NonZeroU32;
use time::Date;

/// The four printed facts that make two documents the same invoice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateKey {
    /// The supplier's name as printed.
    pub supplier: String,
    /// The invoice's own number as printed.
    pub invoice_number: String,
    /// The invoice date as printed.
    pub date: Date,
    /// The gross total as printed, in cents.
    pub gross: Money,
}

impl DuplicateKey {
    /// The key of an extraction this run read, or `None` unless all four facts were read.
    pub fn of_extraction(extraction: &Extraction) -> Option<Self> {
        Some(Self {
            supplier: extraction.supplier.as_ref()?.value.name.clone(),
            invoice_number: extraction.invoice_number.as_ref()?.value.clone(),
            date: extraction.invoice_date.as_ref()?.value,
            gross: extraction.stated_totals.as_ref()?.value.gross,
        })
    }

    /// The key of a document that was read earlier, from its current stored events, or `None`
    /// unless all four facts are there.
    ///
    /// `events` may include superseded events: only the current ones are read, so a person's
    /// correction wins over the model's first reading. When a fact somehow appears more than
    /// once among the current events, the newest one (the last in the stored order) is used.
    ///
    /// Facts are taken one kind at a time, so if a document had two derivation runs whose
    /// events are both current the key could mix a fact from each. That is not reachable today:
    /// the pipeline derives run 1 only. It is written down rather than hidden.
    pub fn of_events(events: &[Event]) -> Option<Self> {
        let current = crate::event::view::current(events);
        let supplier = current.iter().rev().find_map(|event| match event.payload() {
            EventPayload::SupplierIdentified { name, .. } => Some(name.clone()),
            _ => None,
        })?;
        let invoice_number = current.iter().rev().find_map(|event| match event.payload() {
            EventPayload::InvoiceNumberStated { number } => Some(number.clone()),
            _ => None,
        })?;
        let gross = current.iter().rev().find_map(|event| match event.payload() {
            EventPayload::TotalsStated { gross, .. } => Some(*gross),
            _ => None,
        })?;
        // Every derived event carries the invoice date (see the module docs), so any dated
        // current event gives it.
        let date = current.iter().rev().find_map(|event| event.date())?;
        Some(Self {
            supplier,
            invoice_number,
            date,
            gross,
        })
    }
}

/// An earlier document of the same account and the four facts it states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EarlierDocument {
    pub document: DocumentId,
    pub key: DuplicateKey,
}

impl EarlierDocument {
    /// What an earlier document says, from its current stored events.
    pub fn read(document: &DocumentId, events: &[Event]) -> Option<Self> {
        Some(Self {
            document: document.clone(),
            key: DuplicateKey::of_events(events)?,
        })
    }
}

/// The earliest of `earlier` that states the same four facts. `earlier` is given oldest first,
/// so the first match is the one named.
pub fn find(key: &DuplicateKey, earlier: &[EarlierDocument]) -> Option<DocumentId> {
    earlier
        .iter()
        .find(|candidate| &candidate.key == key)
        .map(|candidate| candidate.document.clone())
}

/// True when the document already has a current `possible_duplicate` event (recorded when it
/// was read before). A re-read keeps that one instead of recording a second. A warning a person
/// has superseded is not a flag; a re-read can then re-derive the same id, which is a no-op when
/// the match is unchanged.
pub fn already_flagged(events: &[Event]) -> bool {
    crate::event::view::current(events)
        .iter()
        .any(|event| matches!(event.payload(), EventPayload::PossibleDuplicate { .. }))
}

/// The one warning event that names `matching_document` as the same invoice.
///
/// The id is `<document>:r<run>:possible-duplicate`, so deriving or appending it twice cannot
/// make a second one (the store's append is idempotent for the same id and content).
pub fn possible_duplicate_event(
    document: &DocumentId,
    run: NonZeroU32,
    key: &DuplicateKey,
    matching_document: DocumentId,
) -> Result<Event> {
    Event::new(EventParts {
        id: EventId::new(format!("{}:r{}:possible-duplicate", document.as_str(), run.get())),
        // The fact belongs to the invoice's own date, like every other event of the document.
        date: Some(key.date),
        payload: EventPayload::PossibleDuplicate {
            matching_document,
            reason: DuplicateReason::SupplierNumberDateTotal,
        },
        // The finding is about the document as a whole, so it cites page one. It is `CERTAIN`
        // because *the comparison* is exact given the stored facts; it does not claim the facts
        // are right (their own events carry their confidence), and it is only a Warning.
        citation: Citation {
            document: document.clone(),
            page: NonZeroU32::MIN,
            locator: None,
        },
        extractor: Extractor::System,
        confidence: Confidence::CERTAIN,
        supersedes: None,
    })
}

#[cfg(test)]
mod tests;
