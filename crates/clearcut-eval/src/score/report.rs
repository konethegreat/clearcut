// Written by Kone Tshivhinda & Claude
//! The score of a run: what is measured, and how it reads.
//!
//! A [`Report`] holds case ids, field names, counts and the gate's verdict. It
//! never holds a value from a label, a document or a result, so it is safe to
//! print, attach to a pull request or keep in CI logs.

use super::field::Field;
use super::gate::{SplitChoice, Verdict};
use super::result::TokenUsage;
use super::tally::{Ratio, Tally};
use crate::label::ParseError;
use serde::{Serialize, Serializer};
use std::collections::BTreeMap;
use std::fmt::{self, Write as _};

/// How a silent error went wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    Wrong,
    Missing,
    Spurious,
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ErrorKind::Wrong => "wrong",
            ErrorKind::Missing => "missing",
            ErrorKind::Spurious => "spurious",
        })
    }
}

/// A critical field that was wrong, missing or invented, with nobody told.
/// This is the number the release gate is about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SilentError {
    pub case: String,
    pub field: Field,
    pub error: ErrorKind,
}

/// Do the validator's findings match the ones each label says to expect?
/// (`matched` is the number of expected findings that were reported.)
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct ValidationAgreement {
    pub expected: u64,
    pub reported: u64,
    pub matched: u64,
}

impl ValidationAgreement {
    /// Of the findings reported, how many were expected.
    pub fn precision(&self) -> Ratio {
        Ratio::new(self.matched, self.reported)
    }

    /// Of the findings expected, how many were reported.
    pub fn recall(&self) -> Ratio {
        Ratio::new(self.matched, self.expected)
    }
}

/// The results for the cases carrying one tag.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TagScore {
    pub cases: u64,
    pub critical: Tally,
    pub non_critical: Tally,
    pub silent_critical_errors: u64,
}

/// Something wrong with the result files themselves. No variant holds text
/// from a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResultProblem {
    /// The result file for a case could not be read.
    Unreadable { case: String },
    /// The result file for a case could not be understood.
    Parse { case: String, error: ParseError },
    /// The `case` inside a result file is not the case it is named for.
    CaseMismatch { case: String },
    /// Files in the results folder that belong to no case in the gold set.
    OrphanResults { count: usize },
}

impl fmt::Display for ResultProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResultProblem::Unreadable { case } => write!(f, "{case}: the result file cannot be read"),
            ResultProblem::Parse { case, error } => write!(f, "{case}: {error}"),
            ResultProblem::CaseMismatch { case } => {
                write!(
                    f,
                    "{case}: the case named inside the result file is a different one"
                )
            }
            ResultProblem::OrphanResults { count } => {
                write!(
                    f,
                    "{count} file(s) in the results folder belong to no case in the gold set"
                )
            }
        }
    }
}

impl Serialize for ResultProblem {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// Everything measured about one run over one split.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    pub split: SplitChoice,
    pub review_confidence: u16,
    /// Cases in the selected split.
    pub cases: u64,
    /// Cases that reached a result, out of all cases.
    pub completion: Ratio,
    /// Critical fields that were wrong, missing or invented with no flag,
    /// ordered by case then field.
    pub silent_critical_errors: Vec<SilentError>,
    /// Critical fields, all outcomes (the accuracy is before any review).
    pub critical: Tally,
    pub non_critical: Tally,
    /// Cases that ended in review, out of cases that finished.
    pub review_rate: Ratio,
    pub validation: ValidationAgreement,
    /// Correct values that cite the right page, out of correct values that cite a page.
    pub citation: Ratio,
    pub tags: BTreeMap<String, TagScore>,
    pub tokens: TokenUsage,
    pub cases_with_usage: u64,
    pub result_problems: Vec<ResultProblem>,
    pub gate: Verdict,
}

impl Report {
    pub fn passed(&self) -> bool {
        self.gate.passed
    }

    /// The report as Markdown, for a person or a pull request.
    pub fn render_markdown(&self) -> String {
        let mut out = String::new();
        let noun = if self.cases == 1 { "case" } else { "cases" };
        let _ = writeln!(
            out,
            "# Gold-set score: {} split, {} {noun}\n",
            self.split, self.cases
        );

        if self.gate.passed {
            let _ = writeln!(out, "**Gate: PASSED**\n");
        } else {
            let _ = writeln!(out, "**Gate: FAILED**");
            for failure in &self.gate.failures {
                let _ = writeln!(out, "- {failure}");
            }
            out.push('\n');
        }

        let _ = writeln!(out, "## Silent critical errors\n");
        if self.silent_critical_errors.is_empty() {
            let _ = writeln!(out, "None.\n");
        } else {
            let _ = writeln!(out, "| Case | Field | Error |\n|---|---|---|");
            for error in &self.silent_critical_errors {
                let _ = writeln!(out, "| {} | {} | {} |", error.case, error.field, error.error);
            }
            out.push('\n');
        }

        let _ = writeln!(out, "## Accuracy before review\n");
        let _ = writeln!(
            out,
            "| Fields | Total | Correct | Wrong (flagged / silent) | Missing (flagged / silent) | \
             Spurious (flagged / silent) | Accuracy |\n|---|---|---|---|---|---|---|"
        );
        for (name, tally) in [("Critical", &self.critical), ("Non-critical", &self.non_critical)] {
            let _ = writeln!(
                out,
                "| {name} | {} | {} | {} / {} | {} / {} | {} / {} | {} |",
                tally.total(),
                tally.correct,
                tally.wrong_flagged,
                tally.wrong_silent,
                tally.missing_flagged,
                tally.missing_silent,
                tally.spurious_flagged,
                tally.spurious_silent,
                tally.accuracy()
            );
        }

        let _ = writeln!(out, "\n## Other measures\n");
        let _ = writeln!(out, "- Completion: {}", self.completion);
        let _ = writeln!(
            out,
            "- Review rate (cases ending in review): {}",
            self.review_rate
        );
        let _ = writeln!(
            out,
            "- Validation findings: precision {}, recall {}",
            self.validation.precision(),
            self.validation.recall()
        );
        let _ = writeln!(out, "- Citation accuracy: {}", self.citation);
        let _ = writeln!(
            out,
            "- Tokens: {} in, {} out ({} of {} cases reported usage)",
            self.tokens.input_tokens, self.tokens.output_tokens, self.cases_with_usage, self.cases
        );
        let _ = writeln!(
            out,
            "- Review threshold: confidence below {} is flagged",
            self.review_confidence
        );

        let _ = writeln!(out, "\n## By tag\n");
        if self.tags.is_empty() {
            let _ = writeln!(out, "No tags.");
        } else {
            let _ = writeln!(
                out,
                "| Tag | Cases | Critical accuracy | Silent critical errors |\n|---|---|---|---|"
            );
            for (tag, score) in &self.tags {
                let _ = writeln!(
                    out,
                    "| {tag} | {} | {} | {} |",
                    score.cases,
                    score.critical.accuracy(),
                    score.silent_critical_errors
                );
            }
        }

        if !self.result_problems.is_empty() {
            let _ = writeln!(out, "\n## Notes on result files\n");
            for problem in &self.result_problems {
                let _ = writeln!(out, "- {problem}");
            }
        }
        out
    }
}
