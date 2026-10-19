// Written by Kone Tshivhinda & Claude
//! Events: atomic, dated, sourced facts about a document (research 05 §4).
//!
//! The event store is the source of truth; narration and every export are
//! derived from events (ADR-0004). An [`Event`] is immutable once built: a
//! correction is a *new* event whose `supersedes` points at the old one.
//!
//! This module is pure: it does no I/O and never invents IDs. The storage
//! layer assigns `EventId`s and `DocumentId`s and passes them in.

mod kind;
mod payload;
pub mod view;

pub use kind::{EventGroup, EventType, Severity};
pub use payload::{DuplicateReason, EventPayload, TotalField};

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::num::NonZeroU32;
use time::Date;

/// Largest value of [`Confidence`] and the scale of [`Locator::BoundingBox`]:
/// per-mille (thousandths), so no floats are needed anywhere (ADR-0002).
pub const PER_MILLE: u16 = 1000;

/// Identifies one event. A "newtype": a one-field struct that stops an event ID
/// being mixed up with a document ID or any other string.
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EventId(String);

impl EventId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Identifies the uploaded document an event was read from.
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DocumentId(String);

impl DocumentId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// How sure the extractor is, as an integer from 0 to 1000 (1000 = certain).
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "u16", into = "u16")]
pub struct Confidence(u16);

impl Confidence {
    pub fn new(per_mille: u16) -> Result<Self> {
        if per_mille > PER_MILLE {
            return Err(Error::InvalidEvent("confidence must be between 0 and 1000"));
        }
        Ok(Self(per_mille))
    }

    pub fn get(self) -> u16 {
        self.0
    }
}

impl TryFrom<u16> for Confidence {
    type Error = Error;

    fn try_from(value: u16) -> Result<Self> {
        Self::new(value)
    }
}

impl From<Confidence> for u16 {
    fn from(value: Confidence) -> u16 {
        value.0
    }
}

/// Where on a page a fact was found.
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Locator {
    /// A range of characters in the page's text layer (`start` inclusive, `end` exclusive).
    TextSpan { start: u32, end: u32 },
    /// A box on the page in per-mille of its width and height; `x` and `y` are
    /// the top-left corner. Page-size independent, so it survives re-rendering.
    BoundingBox { x: u16, y: u16, width: u16, height: u16 },
}

impl Locator {
    fn validate(self) -> Result<()> {
        match self {
            Locator::TextSpan { start, end } if start >= end => {
                Err(Error::InvalidEvent("text span must end after it starts"))
            }
            Locator::BoundingBox { x, y, width, height }
                if width == 0
                    || height == 0
                    || u32::from(x) + u32::from(width) > u32::from(PER_MILLE)
                    || u32::from(y) + u32::from(height) > u32::from(PER_MILLE) =>
            {
                Err(Error::InvalidEvent("bounding box must lie within the page"))
            }
            _ => Ok(()),
        }
    }
}

/// The evidence for an event: which document, which page, and optionally where
/// on the page. Every event has one, so every narrated sentence can link back
/// to its source.
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Citation {
    pub document: DocumentId,
    /// 1-based page number. `NonZeroU32` makes page 0 impossible to represent.
    pub page: NonZeroU32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locator: Option<Locator>,
}

/// Who or what produced the event.
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Extractor {
    /// A deterministic rule or parser (e.g. the embedded-XML reader).
    Rule { name: String },
    /// A language model. The version is stored for audit reproducibility (P3-09).
    Model { name: String, prompt_version: String },
    /// A person confirmed or corrected the fact.
    Human { user: String },
    /// ClearCut itself (e.g. recomputed totals).
    System,
}

/// The raw ingredients of an [`Event`]. Fill this in and pass it to
/// [`Event::new`], which checks it. (A struct with named fields is clearer than
/// a function with seven positional arguments.)
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Deserialize)]
pub struct EventParts {
    pub id: EventId,
    /// The date the *fact* belongs to (e.g. the invoice date), not the date it
    /// was processed. `None` when the fact has no date of its own.
    #[serde(default)]
    #[cfg_attr(feature = "schemars", schemars(with = "Option<String>"))]
    pub date: Option<Date>,
    pub payload: EventPayload,
    pub citation: Citation,
    pub extractor: Extractor,
    pub confidence: Confidence,
    /// The earlier event this one corrects, if any.
    #[serde(default)]
    pub supersedes: Option<EventId>,
}

/// An atomic, dated, sourced fact. Fields are private and there are no
/// setters, so an `Event` that exists is always valid and never changes.
///
/// Deserialising goes through [`Event::new`] (`try_from`), so JSON from outside
/// is held to the same rules as code.
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "EventParts")]
pub struct Event {
    id: EventId,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schemars", schemars(with = "Option<String>"))]
    date: Option<Date>,
    payload: EventPayload,
    citation: Citation,
    extractor: Extractor,
    confidence: Confidence,
    #[serde(skip_serializing_if = "Option::is_none")]
    supersedes: Option<EventId>,
}

impl Event {
    /// Validates the parts and builds an event.
    pub fn new(parts: EventParts) -> Result<Self> {
        if parts.id.as_str().is_empty() {
            return Err(Error::InvalidEvent("event id must not be empty"));
        }
        if parts.citation.document.as_str().is_empty() {
            return Err(Error::InvalidEvent("document id must not be empty"));
        }
        if parts.supersedes.as_ref() == Some(&parts.id) {
            return Err(Error::InvalidEvent("an event cannot supersede itself"));
        }
        if !parts.payload.has_single_currency() {
            return Err(Error::InvalidEvent(
                "all amounts in one event must share a currency",
            ));
        }
        if let Some(locator) = parts.citation.locator {
            locator.validate()?;
        }
        Ok(Self {
            id: parts.id,
            date: parts.date,
            payload: parts.payload,
            citation: parts.citation,
            extractor: parts.extractor,
            confidence: parts.confidence,
            supersedes: parts.supersedes,
        })
    }

    pub fn id(&self) -> &EventId {
        &self.id
    }

    pub fn date(&self) -> Option<Date> {
        self.date
    }

    pub fn payload(&self) -> &EventPayload {
        &self.payload
    }

    pub fn citation(&self) -> &Citation {
        &self.citation
    }

    pub fn extractor(&self) -> &Extractor {
        &self.extractor
    }

    pub fn confidence(&self) -> Confidence {
        self.confidence
    }

    pub fn supersedes(&self) -> Option<&EventId> {
        self.supersedes.as_ref()
    }

    pub fn event_type(&self) -> EventType {
        self.payload.event_type()
    }

    /// The group is derived from the type, never stored, so the two cannot disagree.
    pub fn group(&self) -> EventGroup {
        self.event_type().group()
    }

    /// Likewise derived from the type.
    pub fn severity(&self) -> Severity {
        self.event_type().severity()
    }
}

impl TryFrom<EventParts> for Event {
    type Error = Error;

    fn try_from(parts: EventParts) -> Result<Self> {
        Event::new(parts)
    }
}

#[cfg(test)]
mod tests;
