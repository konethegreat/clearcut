// Written by Kone Tshivhinda & Claude
//! The rules of the model router (ADR-0018): when a model's reading of a page is good enough,
//! when to ask a stronger model, and when to hand the page to a person.
//!
//! Three parts, all pure (no I/O, no clock, no model, no money):
//!
//! - [`RouterPolicy`]: the ladder as data (which models, in which order, at what effort,
//!   how big an answer, how large a picture) and the caps on calls.
//! - [`assess`]: from evidence ClearCut can compute (problems converting the reading, the
//!   arithmetic, a check against the page's own text) decide **accept**, **escalate** (and
//!   why) or **review**. Nothing the model says about itself, and nothing the page says,
//!   takes part.
//! - [`PageReader`]: a small state machine for one page. The async driver (a later crate)
//!   asks it what to do next, does it, and tells it what happened. Because it is a plain
//!   value, "it always ends within the caps" is a property the tests check over every
//!   sequence of outcomes, and it can be stored between two calls (a retry resumes where
//!   the page stopped).
//!
//! Nothing here holds a word of the document: reasons, notes and paths are fixed names.

mod assess;
mod policy;
mod reader;

#[cfg(test)]
mod tests_assess;
#[cfg(test)]
mod tests_names;
#[cfg(test)]
mod tests_policy;
#[cfg(test)]
mod tests_reader;

pub use assess::{assess, Assessment, ReadingEvidence, TextCheck, Verdict};
pub(crate) use policy::plain_model_id;
pub use policy::{Picture, RouterPolicy, Rung};
pub use reader::{CallKind, CallPlan, Outcome, PageOutcome, PageReader, Step};

use crate::reading::ProblemKind;
use serde::{Deserialize, Serialize};

/// How hard the model is asked to think (the only thinking control; ADR-0005 section 3).
/// The provider crate maps it onto its own effort levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingEffort {
    Low,
    Medium,
    High,
}

impl ThinkingEffort {
    /// The name that is stored (the same as the serialised form).
    pub fn as_str(self) -> &'static str {
        match self {
            ThinkingEffort::Low => "low",
            ThinkingEffort::Medium => "medium",
            ThinkingEffort::High => "high",
        }
    }
}

/// Why a page goes to the next rung. The names are stored in the call log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EscalationReason {
    /// A critical value could not be read exactly.
    ReadingProblems,
    /// A line's quantity times price is not its total (or a quantity or amount is impossible).
    LineArithmetic,
    /// The printed totals are not the sum of the lines (checked on a one-page document).
    TotalsMismatch,
    /// An amount the model read does not occur in the page's own text.
    TextCheckFailed,
    /// The answer was not valid against the schema, twice.
    SchemaInvalid,
    /// The key cannot use the model; the next one is tried.
    RungUnavailable,
}

impl EscalationReason {
    pub const ALL: [EscalationReason; 6] = [
        EscalationReason::ReadingProblems,
        EscalationReason::LineArithmetic,
        EscalationReason::TotalsMismatch,
        EscalationReason::TextCheckFailed,
        EscalationReason::SchemaInvalid,
        EscalationReason::RungUnavailable,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            EscalationReason::ReadingProblems => "reading_problems",
            EscalationReason::LineArithmetic => "line_arithmetic",
            EscalationReason::TotalsMismatch => "totals_mismatch",
            EscalationReason::TextCheckFailed => "text_check_failed",
            EscalationReason::SchemaInvalid => "schema_invalid",
            EscalationReason::RungUnavailable => "rung_unavailable",
        }
    }
}

/// Why a page goes to a person. The names are stored and shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewReason {
    /// Every rung was tried and the last reading still failed the checks.
    TopRungFailed,
    /// Too much on one page for one answer (cut off at the ceiling, or over 200 lines).
    TooLong,
    /// The model declined to answer.
    Refused,
    /// The next call would have broken a budget.
    Budget,
    /// The page or the document used its calls.
    CallCap,
    /// Nothing was sent: the page could not be sent safely (strict redaction).
    CannotBeSentSafely,
    /// The provider rejected our request: a bug.
    RequestRejected,
    /// The key can use none of the rungs.
    NoRungAvailable,
    /// The line amounts include VAT, which is not backed out (ADR-0014 section 6).
    AmountsIncludeVat,
    /// No line says how it is taxed and the page's totals do not settle it. A stronger model
    /// cannot read what is not printed, and pressed to answer it would guess.
    TaxNotShown,
}

impl ReviewReason {
    pub const ALL: [ReviewReason; 10] = [
        ReviewReason::TopRungFailed,
        ReviewReason::TooLong,
        ReviewReason::Refused,
        ReviewReason::Budget,
        ReviewReason::CallCap,
        ReviewReason::CannotBeSentSafely,
        ReviewReason::RequestRejected,
        ReviewReason::NoRungAvailable,
        ReviewReason::AmountsIncludeVat,
        ReviewReason::TaxNotShown,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ReviewReason::TopRungFailed => "top_rung_failed",
            ReviewReason::TooLong => "too_long",
            ReviewReason::Refused => "refused",
            ReviewReason::Budget => "budget",
            ReviewReason::CallCap => "call_cap",
            ReviewReason::CannotBeSentSafely => "cannot_be_sent_safely",
            ReviewReason::RequestRejected => "request_rejected",
            ReviewReason::NoRungAvailable => "no_rung_available",
            ReviewReason::AmountsIncludeVat => "amounts_include_vat",
            ReviewReason::TaxNotShown => "tax_not_shown",
        }
    }

    /// Parses exactly one name: no trimming, no case folding.
    pub fn parse(name: &str) -> Option<ReviewReason> {
        ReviewReason::ALL
            .into_iter()
            .find(|reason| reason.as_str() == name)
    }
}

/// What a value's confidence rests on. It comes from **evidence**, not from which model read
/// it: a stronger model is not a reason to trust a number (ADR-0018 section 3). Provisional
/// constants, for the confidence work (P2-09) to refine against the gold set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfidenceBasis {
    /// The arithmetic holds and every amount occurs in the page's own text.
    TextConfirmed,
    /// The arithmetic holds, and there is no usable text to confirm it against.
    ArithmeticOnly,
    /// The page ended with open problems.
    Unresolved,
}

impl ConfidenceBasis {
    /// The name that is stored (the same as the serialised form).
    pub fn as_str(self) -> &'static str {
        match self {
            ConfidenceBasis::TextConfirmed => "text_confirmed",
            ConfidenceBasis::ArithmeticOnly => "arithmetic_only",
            ConfidenceBasis::Unresolved => "unresolved",
        }
    }

    /// Parses exactly one name: no trimming, no case folding.
    pub fn parse(name: &str) -> Option<ConfidenceBasis> {
        [
            ConfidenceBasis::TextConfirmed,
            ConfidenceBasis::ArithmeticOnly,
            ConfidenceBasis::Unresolved,
        ]
        .into_iter()
        .find(|basis| basis.as_str() == name)
    }

    /// Per mille, as [`crate::event::Confidence`] takes it. Under 800 is flagged for review
    /// by the gold set's gate (ADR-0010).
    pub fn per_mille(self) -> u16 {
        match self {
            ConfidenceBasis::TextConfirmed => 950,
            ConfidenceBasis::ArithmeticOnly => 850,
            ConfidenceBasis::Unresolved => 500,
        }
    }
}

/// Something recorded on an accepted page that did not stop it. Fixed names and paths only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Note {
    /// A value that is not critical could not be read exactly and was left out.
    NonCriticalProblem { path: String, kind: ProblemKind },
    /// The due date is before the invoice date: a fact about the document.
    DueBeforeIssue,
    /// A South African tax-invoice rule the document breaks: a fact about the document.
    Compliance { field: String },
}
