// Written by Kone Tshivhinda & Claude
//! The fixed event taxonomy: what kinds of event exist and which group each
//! belongs to. Grouping is deterministic code over event types, never an LLM
//! judgement (ADR-0004, research 05 §4).

use serde::{Deserialize, Serialize};

/// The ten fixed groups events are organised into (research 05 §4).
///
/// `Ord` follows the declaration order, so sorting by group gives the
/// reading order of a report.
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventGroup {
    /// Received, classified, pages processed, extraction completed.
    Document,
    /// Supplier and recipient identity.
    Parties,
    /// Line items, discounts, fees.
    Charges,
    /// VAT per category, zero-rated and exempt lines.
    Tax,
    /// Stated totals and stated-vs-recomputed mismatches.
    Totals,
    /// Due date, payment terms, bank details.
    Terms,
    /// Credit notes, debit notes, corrections.
    Adjustments,
    /// Payments recorded, partial payments, overdue.
    Payments,
    /// Missing fields, threshold requirements, 21-day breach.
    Compliance,
    /// Possible duplicates, bank-detail changes, unusual amounts.
    Risk,
}

impl EventGroup {
    /// Every group, in taxonomy order.
    pub const ALL: [EventGroup; 10] = [
        EventGroup::Document,
        EventGroup::Parties,
        EventGroup::Charges,
        EventGroup::Tax,
        EventGroup::Totals,
        EventGroup::Terms,
        EventGroup::Adjustments,
        EventGroup::Payments,
        EventGroup::Compliance,
        EventGroup::Risk,
    ];
}

/// How much attention an event needs. Derived from the event type, never
/// stored or judged by a model. `Ord` runs from least to most severe, so
/// `Info < Warning < Critical`.
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// A plain fact.
    Info,
    /// Needs a look: a compliance gap or a possible duplicate.
    Warning,
    /// The document contradicts itself (a stated total differs from the recomputed one).
    Critical,
}

/// The kind of an event. This is the payload's variant name without its data,
/// which makes it cheap to copy, compare, sort and use as a map key.
///
/// More kinds are added as the pipeline needs them (see PHASES.md P3-03).
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    SupplierIdentified,
    LineItemBilled,
    VatCalculated,
    TotalsStated,
    TotalsMismatch,
    DueDateStated,
    CreditNoteReferenced,
    ComplianceIssueFound,
    PossibleDuplicate,
}

impl EventType {
    /// Every kind currently defined.
    pub const ALL: [EventType; 9] = [
        EventType::SupplierIdentified,
        EventType::LineItemBilled,
        EventType::VatCalculated,
        EventType::TotalsStated,
        EventType::TotalsMismatch,
        EventType::DueDateStated,
        EventType::CreditNoteReferenced,
        EventType::ComplianceIssueFound,
        EventType::PossibleDuplicate,
    ];

    /// The group this kind belongs to. The `match` has no wildcard arm, so
    /// adding a new `EventType` without choosing a group will not compile.
    pub fn group(self) -> EventGroup {
        match self {
            EventType::SupplierIdentified => EventGroup::Parties,
            EventType::LineItemBilled => EventGroup::Charges,
            EventType::VatCalculated => EventGroup::Tax,
            EventType::TotalsStated | EventType::TotalsMismatch => EventGroup::Totals,
            EventType::DueDateStated => EventGroup::Terms,
            EventType::CreditNoteReferenced => EventGroup::Adjustments,
            EventType::ComplianceIssueFound => EventGroup::Compliance,
            EventType::PossibleDuplicate => EventGroup::Risk,
        }
    }

    /// How much attention this kind of event needs. As with `group`, there is
    /// no wildcard arm, so every new kind must be given a severity.
    pub fn severity(self) -> Severity {
        match self {
            EventType::TotalsMismatch => Severity::Critical,
            EventType::ComplianceIssueFound | EventType::PossibleDuplicate => Severity::Warning,
            EventType::SupplierIdentified
            | EventType::LineItemBilled
            | EventType::VatCalculated
            | EventType::TotalsStated
            | EventType::DueDateStated
            | EventType::CreditNoteReferenced => Severity::Info,
        }
    }
}
