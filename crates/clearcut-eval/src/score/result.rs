// Written by Kone Tshivhinda & Claude
//! What a run of the system produced for one case: `results/<case>.json`.
//!
//! ```json
//! { "case": "g-0001", "disposition": "done",
//!   "extraction": { ... a core `Extraction` ... },
//!   "model": { "name": "...", "prompt_version": "..." },
//!   "usage": { "input_tokens": 0, "output_tokens": 0 } }
//! ```
//!
//! `disposition` is one of the two terminal states a job can end in. The
//! extraction may be absent (a document that could not be read at all). Fields
//! the scorer does not know (a run may record more) are ignored, but the ones
//! it needs must have the right shape. Errors say where, never what, because
//! an extraction holds a real document's content.

use crate::label::ParseError;
use clearcut_core::extraction::Extraction;
use serde::{Deserialize, Serialize};

/// How a job ended. Every job ends in one of these (research 05).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    /// The system finished without asking for a person.
    Done,
    /// The system asked a person to look.
    NeedsReview,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelRef {
    pub name: String,
    pub prompt_version: String,
}

/// Tokens a case used, for cost reporting.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunResult {
    pub case: String,
    pub disposition: Disposition,
    #[serde(default)]
    pub extraction: Option<Extraction>,
    #[serde(default)]
    pub model: Option<ModelRef>,
    #[serde(default)]
    pub usage: Option<TokenUsage>,
}

impl RunResult {
    /// Reads one result file. Errors carry a line and column only.
    pub fn parse(bytes: &[u8]) -> Result<RunResult, ParseError> {
        serde_json::from_slice(bytes).map_err(|error| {
            let (line, column) = (error.line(), error.column());
            match error.classify() {
                serde_json::error::Category::Data => ParseError::WrongShape { line, column },
                _ => ParseError::NotJson { line, column },
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn minimal() -> serde_json::Value {
        json!({ "case": "g-0001", "disposition": "done" })
    }

    fn parse(value: &serde_json::Value) -> Result<RunResult, ParseError> {
        RunResult::parse(value.to_string().as_bytes())
    }

    #[test]
    fn the_smallest_result_is_a_case_and_how_it_ended() {
        let result = parse(&minimal()).unwrap();
        assert_eq!(result.case, "g-0001");
        assert_eq!(result.disposition, Disposition::Done);
        assert!(result.extraction.is_none() && result.model.is_none() && result.usage.is_none());
        let review = parse(&json!({ "case": "g-0002", "disposition": "needs_review" })).unwrap();
        assert_eq!(review.disposition, Disposition::NeedsReview);
    }

    #[test]
    fn model_usage_and_an_extraction_are_read_when_present() {
        let extraction = json!({
            "document": "g-0001",
            "extractor": { "type": "model", "name": "m", "prompt_version": "p1" },
            "currency": "ZAR",
            "invoice_number": { "value": "INV-1", "page": 1, "confidence": 950 }
        });
        let mut value = minimal();
        value["extraction"] = extraction;
        value["model"] = json!({ "name": "sonnet", "prompt_version": "p1" });
        value["usage"] = json!({ "input_tokens": 1200, "output_tokens": 300 });
        let result = parse(&value).unwrap();
        let extraction = result.extraction.unwrap();
        assert_eq!(extraction.invoice_number.unwrap().value, "INV-1");
        assert_eq!(result.model.unwrap().name, "sonnet");
        assert_eq!(
            result.usage.unwrap(),
            TokenUsage {
                input_tokens: 1200,
                output_tokens: 300
            }
        );
    }

    #[test]
    fn a_null_extraction_means_nothing_was_read() {
        let mut value = minimal();
        value["extraction"] = serde_json::Value::Null;
        assert!(parse(&value).unwrap().extraction.is_none());
    }

    #[test]
    fn things_the_scorer_does_not_know_about_are_ignored() {
        let mut value = minimal();
        value["findings"] = json!(["totals_mismatch:vat"]);
        value["trace_id"] = json!("abc");
        assert!(parse(&value).is_ok());
    }

    #[test]
    fn what_it_needs_must_have_the_right_shape() {
        for value in [
            json!({ "disposition": "done" }),
            json!({ "case": "g-0001" }),
            json!({ "case": "g-0001", "disposition": "failed" }),
            json!({ "case": "g-0001", "disposition": "Done" }),
            json!({ "case": 1, "disposition": "done" }),
            json!({ "case": "g-0001", "disposition": "done", "extraction": { "currency": "ZAR" } }),
            json!({ "case": "g-0001", "disposition": "done", "usage": { "input_tokens": -1, "output_tokens": 0 } }),
        ] {
            assert!(
                matches!(parse(&value), Err(ParseError::WrongShape { .. })),
                "{value}"
            );
        }
    }

    #[test]
    fn not_json_is_reported_as_such() {
        assert!(matches!(
            RunResult::parse(b"{ nope"),
            Err(ParseError::NotJson { .. })
        ));
        assert!(matches!(RunResult::parse(b""), Err(ParseError::NotJson { .. })));
    }

    #[test]
    fn errors_never_echo_the_file() {
        let text = RunResult::parse(br#"{"case":"g-0001","disposition":"secretword"}"#)
            .unwrap_err()
            .to_string();
        assert!(
            !text.contains("secretword") && !text.contains("disposition"),
            "{text}"
        );
    }

    #[test]
    fn a_result_survives_a_round_trip() {
        let mut value = minimal();
        value["usage"] = json!({ "input_tokens": 1, "output_tokens": 2 });
        let result = parse(&value).unwrap();
        let again = RunResult::parse(&serde_json::to_vec(&result).unwrap()).unwrap();
        assert_eq!(result, again);
    }
}
