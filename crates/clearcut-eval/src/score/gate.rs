// Written by Kone Tshivhinda & Claude
//! The release gate: the bar a scored run must clear (ADR-0010, research 05 §7).
//!
//! The defaults are the research targets: **zero** silent critical errors,
//! 97% of critical fields correct before review, 95% of the rest, every job
//! finished, and no kind of document (a tag) far behind the overall figure.
//! Thresholds can be changed in a small JSON file, which rejects unknown keys
//! so a misspelt threshold cannot quietly leave the default in place.
//!
//! Percentages here are **basis points** (hundredths of a percent): 9700 is
//! 97.00%. All comparisons are exact integer comparisons ([`Ratio::at_least`]).

use super::report::TagScore;
use super::tally::{Ratio, Tally};
use crate::label::{ParseError, Split};
use serde::{Deserialize, Serialize, Serializer};
use std::collections::BTreeMap;
use std::fmt;

/// Which half of the gold set a score covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitChoice {
    Dev,
    Test,
    All,
}

impl SplitChoice {
    pub fn includes(self, split: Split) -> bool {
        match self {
            SplitChoice::All => true,
            SplitChoice::Dev => split == Split::Dev,
            SplitChoice::Test => split == Split::Test,
        }
    }
}

impl fmt::Display for SplitChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            SplitChoice::Dev => "dev",
            SplitChoice::Test => "test",
            SplitChoice::All => "all",
        })
    }
}

/// The thresholds. Every field has a default, so a gate file need only name
/// what it changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Gate {
    /// Which split is judged. The test split is the release gate; dev is for tuning.
    pub split: SplitChoice,
    /// Silent errors allowed in critical fields. The target is zero.
    pub max_silent_critical_errors: u64,
    /// Share of critical fields correct before review, in basis points.
    pub min_critical_accuracy_bp: u32,
    pub min_noncritical_accuracy_bp: u32,
    /// Share of cases that reached a result. Every job must end.
    pub min_completion_bp: u32,
    /// How far a tag's critical accuracy may trail the overall figure, in basis points.
    pub max_tag_shortfall_bp: u32,
    /// A tag is only judged once it has at least this many cases.
    pub min_tag_cases: u64,
    /// A value whose confidence (0 to 1000) is below this counts as flagged
    /// for review. A placeholder until P2-09 tunes it on the gold set.
    pub review_confidence: u16,
}

impl Default for Gate {
    fn default() -> Gate {
        Gate {
            split: SplitChoice::Test,
            max_silent_critical_errors: 0,
            min_critical_accuracy_bp: 9_700,
            min_noncritical_accuracy_bp: 9_500,
            min_completion_bp: 10_000,
            max_tag_shortfall_bp: 1_000,
            min_tag_cases: 5,
            review_confidence: 800,
        }
    }
}

/// Why a gate file was refused. Positions and names of settings only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GateError {
    #[error("{0}")]
    Parse(ParseError),
    #[error("the setting {0} is out of range")]
    OutOfRange(&'static str),
}

impl Gate {
    /// Reads a gate file and checks the ranges.
    pub fn parse(bytes: &[u8]) -> Result<Gate, GateError> {
        let gate: Gate = serde_json::from_slice(bytes).map_err(|error| {
            let (line, column) = (error.line(), error.column());
            GateError::Parse(match error.classify() {
                serde_json::error::Category::Data => ParseError::WrongShape { line, column },
                _ => ParseError::NotJson { line, column },
            })
        })?;
        for (name, value) in [
            ("min_critical_accuracy_bp", gate.min_critical_accuracy_bp),
            ("min_noncritical_accuracy_bp", gate.min_noncritical_accuracy_bp),
            ("min_completion_bp", gate.min_completion_bp),
            ("max_tag_shortfall_bp", gate.max_tag_shortfall_bp),
        ] {
            if value > 10_000 {
                return Err(GateError::OutOfRange(name));
            }
        }
        if gate.review_confidence > 1_000 {
            return Err(GateError::OutOfRange("review_confidence"));
        }
        Ok(gate)
    }
}

/// One reason the gate failed. Holds counts and tag names only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateFailure {
    NoCases,
    SilentCriticalErrors {
        found: u64,
        max: u64,
    },
    Completion {
        actual: Ratio,
        min_bp: u32,
    },
    CriticalAccuracy {
        actual: Ratio,
        min_bp: u32,
    },
    NonCriticalAccuracy {
        actual: Ratio,
        min_bp: u32,
    },
    TagShortfall {
        tag: String,
        actual: Ratio,
        overall_bp: u32,
        shortfall_bp: u32,
        max_bp: u32,
    },
}

/// Basis points as a percentage, `9700` as `97.00%`.
pub fn percent(bp: u32) -> String {
    format!("{}%", points(bp))
}

/// Basis points as percentage points, `682` as `6.82`.
fn points(bp: u32) -> String {
    format!("{}.{:02}", bp / 100, bp % 100)
}

impl fmt::Display for GateFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GateFailure::NoCases => f.write_str("There are no cases in the selected split to score."),
            GateFailure::SilentCriticalErrors { found, max } => {
                write!(f, "Silent critical errors: {found} found, at most {max} allowed.")
            }
            GateFailure::Completion { actual, min_bp } => {
                write!(f, "Completion: {actual}, at least {} required.", percent(*min_bp))
            }
            GateFailure::CriticalAccuracy { actual, min_bp } => {
                write!(f, "Critical-field accuracy: {actual}, at least {} required.", percent(*min_bp))
            }
            GateFailure::NonCriticalAccuracy { actual, min_bp } => {
                write!(f, "Non-critical accuracy: {actual}, at least {} required.", percent(*min_bp))
            }
            GateFailure::TagShortfall { tag, actual, overall_bp, shortfall_bp, max_bp } => write!(
                f,
                "Tag {tag}: critical accuracy {actual} is {} points below the overall {}; at most {} allowed.",
                points(*shortfall_bp),
                percent(*overall_bp),
                points(*max_bp),
            ),
        }
    }
}

impl Serialize for GateFailure {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// Whether the gate passed, and if not, why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Verdict {
    pub passed: bool,
    pub failures: Vec<GateFailure>,
}

/// Measures against the gate. `cases` is the number of cases in the selected
/// split; `silent` the silent critical errors among them.
pub(super) fn evaluate(
    gate: &Gate,
    cases: u64,
    completion: Ratio,
    silent: u64,
    critical: &Tally,
    non_critical: &Tally,
    tags: &BTreeMap<String, TagScore>,
) -> Verdict {
    let mut failures = Vec::new();
    if cases == 0 {
        failures.push(GateFailure::NoCases);
    }
    if silent > gate.max_silent_critical_errors {
        failures.push(GateFailure::SilentCriticalErrors {
            found: silent,
            max: gate.max_silent_critical_errors,
        });
    }
    if !completion.at_least(gate.min_completion_bp) {
        failures.push(GateFailure::Completion {
            actual: completion,
            min_bp: gate.min_completion_bp,
        });
    }
    if !critical.accuracy().at_least(gate.min_critical_accuracy_bp) {
        failures.push(GateFailure::CriticalAccuracy {
            actual: critical.accuracy(),
            min_bp: gate.min_critical_accuracy_bp,
        });
    }
    if !non_critical.accuracy().at_least(gate.min_noncritical_accuracy_bp) {
        failures.push(GateFailure::NonCriticalAccuracy {
            actual: non_critical.accuracy(),
            min_bp: gate.min_noncritical_accuracy_bp,
        });
    }
    if let Some(overall_bp) = critical.accuracy().basis_points() {
        for (tag, score) in tags {
            if score.cases < gate.min_tag_cases {
                continue;
            }
            let actual = score.critical.accuracy();
            let Some(tag_bp) = actual.basis_points() else {
                continue;
            };
            let shortfall_bp = overall_bp.saturating_sub(tag_bp);
            if shortfall_bp > gate.max_tag_shortfall_bp {
                failures.push(GateFailure::TagShortfall {
                    tag: tag.clone(),
                    actual,
                    overall_bp,
                    shortfall_bp,
                    max_bp: gate.max_tag_shortfall_bp,
                });
            }
        }
    }
    Verdict {
        passed: failures.is_empty(),
        failures,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_the_research_targets() {
        let gate = Gate::default();
        assert_eq!(gate.split, SplitChoice::Test);
        assert_eq!(gate.max_silent_critical_errors, 0);
        assert_eq!(gate.min_critical_accuracy_bp, 9_700);
        assert_eq!(gate.min_noncritical_accuracy_bp, 9_500);
        assert_eq!(gate.min_completion_bp, 10_000);
        assert_eq!(gate.max_tag_shortfall_bp, 1_000);
        assert_eq!(gate.min_tag_cases, 5);
        assert_eq!(gate.review_confidence, 800);
    }

    #[test]
    fn an_empty_file_means_all_defaults_and_a_file_changes_only_what_it_names() {
        assert_eq!(Gate::parse(b"{}").unwrap(), Gate::default());
        let gate =
            Gate::parse(br#"{ "split": "dev", "max_silent_critical_errors": 2, "review_confidence": 900 }"#)
                .unwrap();
        assert_eq!(gate.split, SplitChoice::Dev);
        assert_eq!(gate.max_silent_critical_errors, 2);
        assert_eq!(gate.review_confidence, 900);
        assert_eq!(gate.min_critical_accuracy_bp, 9_700);
    }

    #[test]
    fn a_misspelt_or_malformed_setting_is_refused() {
        for text in [
            r#"{ "max_silent_errors": 0 }"#,
            r#"{ "split": "train" }"#,
            r#"{ "min_completion_bp": -1 }"#,
            r#"{ "min_completion_bp": "10000" }"#,
            r#"{ "min_completion_bp": 99.5 }"#,
            r#"{ "review_confidence": 70000 }"#,
        ] {
            assert!(
                matches!(
                    Gate::parse(text.as_bytes()),
                    Err(GateError::Parse(ParseError::WrongShape { .. }))
                ),
                "{text}"
            );
        }
        assert!(matches!(
            Gate::parse(b"{ nope"),
            Err(GateError::Parse(ParseError::NotJson { .. }))
        ));
    }

    #[test]
    fn percentages_and_confidences_must_be_in_range() {
        for name in [
            "min_critical_accuracy_bp",
            "min_noncritical_accuracy_bp",
            "min_completion_bp",
            "max_tag_shortfall_bp",
        ] {
            assert!(Gate::parse(format!(r#"{{ "{name}": 10000 }}"#).as_bytes()).is_ok());
            assert_eq!(
                Gate::parse(format!(r#"{{ "{name}": 10001 }}"#).as_bytes()),
                Err(GateError::OutOfRange(name))
            );
        }
        assert!(Gate::parse(br#"{ "review_confidence": 1000 }"#).is_ok());
        assert_eq!(
            Gate::parse(br#"{ "review_confidence": 1001 }"#),
            Err(GateError::OutOfRange("review_confidence"))
        );
    }

    #[test]
    fn errors_never_echo_the_file() {
        let text = Gate::parse(br#"{ "secretsetting": 1 }"#).unwrap_err().to_string();
        assert!(!text.contains("secretsetting"), "{text}");
    }

    #[test]
    fn a_split_choice_includes_the_right_cases() {
        assert!(SplitChoice::All.includes(Split::Dev) && SplitChoice::All.includes(Split::Test));
        assert!(SplitChoice::Dev.includes(Split::Dev) && !SplitChoice::Dev.includes(Split::Test));
        assert!(SplitChoice::Test.includes(Split::Test) && !SplitChoice::Test.includes(Split::Dev));
    }

    #[test]
    fn percentages_print_with_two_decimals() {
        assert_eq!(percent(9_700), "97.00%");
        assert_eq!(percent(682), "6.82%");
        assert_eq!(percent(0), "0.00%");
        assert_eq!(percent(10_000), "100.00%");
    }

    #[test]
    fn failures_read_as_plain_sentences() {
        let ratio = Ratio::new(48, 50);
        assert_eq!(
            GateFailure::NoCases.to_string(),
            "There are no cases in the selected split to score."
        );
        assert_eq!(
            GateFailure::SilentCriticalErrors { found: 1, max: 0 }.to_string(),
            "Silent critical errors: 1 found, at most 0 allowed."
        );
        assert_eq!(
            GateFailure::Completion {
                actual: Ratio::new(4, 5),
                min_bp: 10_000
            }
            .to_string(),
            "Completion: 80.00% (4/5), at least 100.00% required."
        );
        assert_eq!(
            GateFailure::CriticalAccuracy {
                actual: ratio,
                min_bp: 9_700
            }
            .to_string(),
            "Critical-field accuracy: 96.00% (48/50), at least 97.00% required."
        );
        assert_eq!(
            GateFailure::NonCriticalAccuracy {
                actual: ratio,
                min_bp: 9_500
            }
            .to_string(),
            "Non-critical accuracy: 96.00% (48/50), at least 95.00% required."
        );
        assert_eq!(
            GateFailure::TagShortfall {
                tag: "scan".into(),
                actual: Ratio::new(50, 55),
                overall_bp: 9_772,
                shortfall_bp: 682,
                max_bp: 500
            }
            .to_string(),
            "Tag scan: critical accuracy 90.90% (50/55) is 6.82 points below the overall 97.72%; at most 5.00 allowed."
        );
    }
}
