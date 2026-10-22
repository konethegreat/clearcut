// Written by Kone Tshivhinda & Claude
//! Scoring a run of the system against the gold set (ADR-0010).
//!
//! Every field of every case in the selected split gets one [`Outcome`]:
//! correct, or wrong / missing / spurious, each either *flagged* (the system
//! said a person must look) or *silent*. A field is flagged when
//!
//! 1. the case ended `needs_review`, or
//! 2. a validator finding is about it ([`flags`]), or
//! 3. its confidence is below the gate's review threshold.
//!
//! The validator's findings are **recomputed here** from the extraction, with
//! the same code the pipeline uses; a run cannot hide a finding by leaving it
//! out of its result file.
//!
//! A *silent critical error* is a critical field that was wrong, missing or
//! invented, and not flagged. The release gate allows none.
//!
//! [`score`] is a pure function over values, so it is tested without files;
//! [`load`] reads a gold set and a results folder and calls it.

pub mod compare;
pub mod field;
pub mod flags;
pub mod gate;
pub mod load;
pub mod report;
pub mod result;
pub mod tally;

pub use field::Field;
pub use gate::{Gate, GateError, GateFailure, SplitChoice, Verdict};
pub use load::{score_folders, ScoreError};
pub use report::{ErrorKind, Report, ResultProblem, SilentError, TagScore, ValidationAgreement};
pub use result::{Disposition, RunResult, TokenUsage};
pub use tally::{Outcome, Ratio, Tally};

use crate::keys::{compare as compare_keys, finding_keys};
use crate::label::Split;
use clearcut_core::extraction::{validate, Extraction};
use compare::{compare, Compared, Verdict as Match};
use std::collections::BTreeMap;

/// One case to score: the truth, what the label says to expect of the
/// validator, and what the run produced (`None` if it produced nothing).
#[derive(Debug, Clone)]
pub struct CaseInput {
    pub id: String,
    pub split: Split,
    pub tags: Vec<String>,
    pub expected_findings: Vec<String>,
    pub truth: Extraction,
    pub result: Option<RunResult>,
}

/// Scores `cases` (those in the gate's split) and applies the gate. `problems`
/// are notes about the result files, carried into the report.
pub fn score(cases: &[CaseInput], gate: &Gate, problems: Vec<ResultProblem>) -> Report {
    let mut critical = Tally::default();
    let mut non_critical = Tally::default();
    let mut silent_errors = Vec::new();
    let mut tags: BTreeMap<String, TagScore> = BTreeMap::new();
    let mut validation = ValidationAgreement::default();
    let mut citation = Ratio::default();
    let mut tokens = TokenUsage::default();
    let (mut selected, mut completed, mut in_review, mut with_usage) = (0u64, 0u64, 0u64, 0u64);

    for case in cases.iter().filter(|c| gate.split.includes(c.split)) {
        selected += 1;
        let mut case_critical = Tally::default();
        let mut case_non_critical = Tally::default();
        let mut case_silent = 0u64;

        // What did the run say, and what does the validator make of it?
        let (disposition, extraction) = match &case.result {
            Some(result) => (Some(result.disposition), result.extraction.as_ref()),
            None => (None, None),
        };
        let findings = extraction.map(|e| validate(e).findings).unwrap_or_default();

        for compared in compare(&case.truth, extraction) {
            let flagged = disposition.is_none() // no result at all: the gate fails on completion
                || disposition == Some(Disposition::NeedsReview)
                || flags::is_flagged_by(compared.field, &findings)
                || compared.confidence.is_some_and(|c| c.get() < gate.review_confidence);
            let outcome = outcome_of(&compared, flagged);

            if let Some(correct) = compared.page_correct {
                citation.den += 1;
                citation.num += u64::from(correct);
            }
            if compared.field.is_critical() {
                case_critical.add(outcome);
                if outcome.is_silent_error() {
                    case_silent += 1;
                    silent_errors.push(SilentError {
                        case: case.id.clone(),
                        field: compared.field,
                        error: match compared.verdict {
                            Match::Wrong => ErrorKind::Wrong,
                            Match::Missing => ErrorKind::Missing,
                            _ => ErrorKind::Spurious,
                        },
                    });
                }
            } else {
                case_non_critical.add(outcome);
            }
        }

        if let Some(result) = &case.result {
            completed += 1;
            in_review += u64::from(result.disposition == Disposition::NeedsReview);
            if let Some(usage) = result.usage {
                with_usage += 1;
                tokens.input_tokens += usage.input_tokens;
                tokens.output_tokens += usage.output_tokens;
            }
            let actual = finding_keys(&findings);
            let difference = compare_keys(&case.expected_findings, &actual);
            validation.expected += case.expected_findings.len() as u64;
            validation.reported += actual.len() as u64;
            validation.matched += (case.expected_findings.len() - difference.missing.len()) as u64;
        }

        critical.merge(&case_critical);
        non_critical.merge(&case_non_critical);
        for tag in &case.tags {
            let entry = tags.entry(tag.clone()).or_default();
            entry.cases += 1;
            entry.critical.merge(&case_critical);
            entry.non_critical.merge(&case_non_critical);
            entry.silent_critical_errors += case_silent;
        }
    }

    silent_errors.sort_by(|a, b| a.case.cmp(&b.case).then(a.field.cmp(&b.field)));
    let completion = Ratio::new(completed, selected);
    let verdict = gate::evaluate(
        gate,
        selected,
        completion,
        silent_errors.len() as u64,
        &critical,
        &non_critical,
        &tags,
    );

    Report {
        split: gate.split,
        review_confidence: gate.review_confidence,
        cases: selected,
        completion,
        silent_critical_errors: silent_errors,
        critical,
        non_critical,
        review_rate: Ratio::new(in_review, completed),
        validation,
        citation,
        tags,
        tokens,
        cases_with_usage: with_usage,
        result_problems: problems,
        gate: verdict,
    }
}

fn outcome_of(compared: &Compared, flagged: bool) -> Outcome {
    match (compared.verdict, flagged) {
        (Match::Correct, _) => Outcome::Correct,
        (Match::Wrong, true) => Outcome::WrongFlagged,
        (Match::Wrong, false) => Outcome::WrongSilent,
        (Match::Missing, true) => Outcome::MissingFlagged,
        (Match::Missing, false) => Outcome::MissingSilent,
        (Match::Spurious, true) => Outcome::SpuriousFlagged,
        (Match::Spurious, false) => Outcome::SpuriousSilent,
    }
}

#[cfg(test)]
mod tests;
