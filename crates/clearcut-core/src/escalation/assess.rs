// Written by Kone Tshivhinda & Claude
//! Is this reading good enough? (ADR-0018 section 3.)
//!
//! Only evidence ClearCut can compute takes part: the problems found converting the reading,
//! the checks of [`crate::extraction::validate`], and, where the page has trustworthy text,
//! whether each amount the model read occurs in it. A fact about the *document* (a due date
//! before the invoice date, a tax-invoice rule it breaks) is recorded as a note and never makes
//! a stronger model try again: it could not change what is printed.

use super::{ConfidenceBasis, EscalationReason, Note, ReviewReason};
use crate::extraction::Finding;
use crate::reading::{ProblemKind, ReadingProblem};
use serde::{Deserialize, Serialize};

/// What the check of the model's amounts against the page's own text found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextCheck {
    /// The page has no text worth checking against (a scan with no OCR).
    NotAvailable,
    /// Every amount occurs in the text.
    Passed,
    /// Some amounts do not occur in the text.
    Failed { missing: u32 },
}

/// Everything the decision is made from.
#[derive(Debug, Clone, Copy)]
pub struct ReadingEvidence<'a> {
    /// What could not be converted (`PageReading::into_extraction`).
    pub problems: &'a [ReadingProblem],
    /// What `validate` found on the converted page.
    pub findings: &'a [Finding],
    pub text_check: TextCheck,
    /// The page is the whole document, so the printed totals can be compared with its lines.
    /// On page 3 of 5 the totals are on another page, and a mismatch means nothing yet.
    pub whole_document: bool,
}

/// What to do about a reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "reason", rename_all = "snake_case")]
pub enum Verdict {
    Accept,
    /// Try a stronger model, with a report of what was wrong.
    Escalate(EscalationReason),
    /// A stronger model cannot fix this: a person must look.
    Review(ReviewReason),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assessment {
    pub verdict: Verdict,
    /// How many things stop the reading being accepted. The reading with the fewest is the
    /// best draft when every rung fails.
    pub blocking: u32,
    /// Recorded, not blocking.
    pub notes: Vec<Note>,
    /// Every reason to escalate that applies, first the one that is named.
    pub reasons: Vec<EscalationReason>,
    pub confidence: ConfidenceBasis,
}

/// A problem on these values does not stop a page: they are not critical (ADR-0010), and the
/// value is simply left out. **Everything else, including a path nobody listed, is critical**,
/// so a new path in the converter can never hide a problem.
fn is_critical(path: &str) -> bool {
    let mut parts = path.split('/');
    let first = parts.next();
    let second = parts.next();
    !matches!(
        (first, second),
        (Some("recipient" | "due"), _)
            | (Some("supplier"), Some("address"))
            | (Some("credit_note"), Some("original_invoice_number"))
    )
}

/// Decides what to do with one converted and checked reading.
pub fn assess(evidence: &ReadingEvidence<'_>) -> Assessment {
    let mut blocking = 0u32;
    let mut notes = Vec::new();
    let mut critical_problems = 0u32;
    let mut review = None;
    let mut tax_not_shown = false;

    for problem in evidence.problems {
        match problem.kind {
            // A stronger model reads the same printed amounts, or the same long page.
            ProblemKind::AmountsIncludeVat => {
                review = Some(ReviewReason::AmountsIncludeVat);
                blocking += 1;
            }
            ProblemKind::TooManyLines => {
                review = review.or(Some(ReviewReason::TooLong));
                blocking += 1;
            }
            // What is not printed cannot be read by a stronger model; it can only guess.
            ProblemKind::TaxNotShown => {
                tax_not_shown = true;
                blocking += 1;
            }
            _ if is_critical(&problem.path) => {
                critical_problems += 1;
                blocking += 1;
            }
            kind => notes.push(Note::NonCriticalProblem {
                path: problem.path.clone(),
                kind,
            }),
        }
    }

    let mut line_findings = 0u32;
    let mut totals_findings = 0u32;
    for finding in evidence.findings {
        match finding {
            Finding::LineTotalMismatch { .. }
            | Finding::NonPositiveQuantity { .. }
            | Finding::AmountOutOfRange { .. }
            | Finding::CurrencyMismatch { .. } => line_findings += 1,
            Finding::TotalsMismatch { .. } | Finding::TotalsOutOfRange => {
                // A line left out for its missing tax makes the sum short for that reason alone.
                if evidence.whole_document && !tax_not_shown {
                    totals_findings += 1;
                }
            }
            // A carried-balance mismatch is only found once the pages are put together, so a
            // single page's extraction cannot raise it; if it ever does, treat it like a total.
            Finding::CarriedBalanceMismatch { .. } => {
                if evidence.whole_document && !tax_not_shown {
                    totals_findings += 1;
                }
            }
            // A terms page or a continuation page has no lines. Whether the document has any
            // is decided after its pages are put together.
            Finding::NoLines => {}
            Finding::DueBeforeIssue => notes.push(Note::DueBeforeIssue),
            Finding::Compliance(issue) => notes.push(Note::Compliance {
                field: issue.field.to_string(),
            }),
        }
    }
    blocking += line_findings + totals_findings;

    let text_failed = matches!(evidence.text_check, TextCheck::Failed { .. });
    if text_failed {
        blocking += 1;
    }

    let mut reasons = Vec::new();
    if critical_problems > 0 {
        reasons.push(EscalationReason::ReadingProblems);
    }
    if line_findings > 0 {
        reasons.push(EscalationReason::LineArithmetic);
    }
    if totals_findings > 0 {
        reasons.push(EscalationReason::TotalsMismatch);
    }
    if text_failed {
        reasons.push(EscalationReason::TextCheckFailed);
    }

    // Only once everything a stronger model could fix is fixed: then the missing tax is all
    // that is left, and a person decides it.
    if review.is_none() && reasons.is_empty() && tax_not_shown {
        review = Some(ReviewReason::TaxNotShown);
    }

    let verdict = match (review, reasons.first()) {
        (Some(reason), _) => Verdict::Review(reason),
        (None, Some(first)) => Verdict::Escalate(*first),
        (None, None) => Verdict::Accept,
    };
    let confidence = match (verdict, evidence.text_check) {
        (Verdict::Accept, TextCheck::Passed) => ConfidenceBasis::TextConfirmed,
        (Verdict::Accept, _) => ConfidenceBasis::ArithmeticOnly,
        _ => ConfidenceBasis::Unresolved,
    };
    Assessment {
        verdict,
        blocking,
        notes,
        reasons,
        confidence,
    }
}
