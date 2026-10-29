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
    /// The name used in files and the API (the same as the serialised form).
    pub fn as_str(self) -> &'static str {
        match self {
            EventGroup::Document => "document",
            EventGroup::Parties => "parties",
            EventGroup::Charges => "charges",
            EventGroup::Tax => "tax",
            EventGroup::Totals => "totals",
            EventGroup::Terms => "terms",
            EventGroup::Adjustments => "adjustments",
            EventGroup::Payments => "payments",
            EventGroup::Compliance => "compliance",
            EventGroup::Risk => "risk",
        }
    }

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
    InvoiceNumberStated,
    SupplierIdentified,
    RecipientIdentified,
    LineItemBilled,
    LineTotalMismatch,
    VatCalculated,
    TotalsStated,
    TotalsMismatch,
    /// A page-continuation figure printed on a long invoice's page (ADR-0022).
    BalanceStated,
    /// A carried-forward figure that does not match the next page's brought-forward one.
    BalanceMismatch,
    DueDateStated,
    CreditNoteReferenced,
    ComplianceIssueFound,
    PossibleDuplicate,
}

impl EventType {
    /// The name used in files and the API (the same as the serialised form).
    pub fn as_str(self) -> &'static str {
        match self {
            EventType::InvoiceNumberStated => "invoice_number_stated",
            EventType::SupplierIdentified => "supplier_identified",
            EventType::RecipientIdentified => "recipient_identified",
            EventType::LineItemBilled => "line_item_billed",
            EventType::LineTotalMismatch => "line_total_mismatch",
            EventType::VatCalculated => "vat_calculated",
            EventType::TotalsStated => "totals_stated",
            EventType::TotalsMismatch => "totals_mismatch",
            EventType::BalanceStated => "balance_stated",
            EventType::BalanceMismatch => "balance_mismatch",
            EventType::DueDateStated => "due_date_stated",
            EventType::CreditNoteReferenced => "credit_note_referenced",
            EventType::ComplianceIssueFound => "compliance_issue_found",
            EventType::PossibleDuplicate => "possible_duplicate",
        }
    }

    /// Every kind currently defined.
    pub const ALL: [EventType; 14] = [
        EventType::InvoiceNumberStated,
        EventType::SupplierIdentified,
        EventType::RecipientIdentified,
        EventType::LineItemBilled,
        EventType::LineTotalMismatch,
        EventType::VatCalculated,
        EventType::TotalsStated,
        EventType::TotalsMismatch,
        EventType::BalanceStated,
        EventType::BalanceMismatch,
        EventType::DueDateStated,
        EventType::CreditNoteReferenced,
        EventType::ComplianceIssueFound,
        EventType::PossibleDuplicate,
    ];

    /// The group this kind belongs to. The `match` has no wildcard arm, so
    /// adding a new `EventType` without choosing a group will not compile.
    pub fn group(self) -> EventGroup {
        match self {
            EventType::InvoiceNumberStated => EventGroup::Document,
            EventType::SupplierIdentified | EventType::RecipientIdentified => EventGroup::Parties,
            EventType::LineItemBilled | EventType::LineTotalMismatch => EventGroup::Charges,
            EventType::VatCalculated => EventGroup::Tax,
            EventType::TotalsStated | EventType::TotalsMismatch => EventGroup::Totals,
            EventType::BalanceStated | EventType::BalanceMismatch => EventGroup::Totals,
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
            EventType::TotalsMismatch
            | EventType::LineTotalMismatch
            | EventType::BalanceMismatch => Severity::Critical,
            EventType::ComplianceIssueFound | EventType::PossibleDuplicate => Severity::Warning,
            EventType::InvoiceNumberStated
            | EventType::SupplierIdentified
            | EventType::RecipientIdentified
            | EventType::LineItemBilled
            | EventType::VatCalculated
            | EventType::TotalsStated
            | EventType::BalanceStated
            | EventType::DueDateStated
            | EventType::CreditNoteReferenced => Severity::Info,
        }
    }
}

impl Severity {
    /// The name used in files and the API (the same as the serialised form).
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Critical => "critical",
        }
    }
}
