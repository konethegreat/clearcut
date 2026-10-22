// Written by Kone Tshivhinda & Claude
//! Sample *results* for the synthetic gold set: what a run of the system might
//! have produced, with deliberate flaws, so the scorer can be shown (and
//! tested) doing every part of its job.
//!
//! The test split is read perfectly, so it passes the gate. The dev split has
//! one of each kind of outcome:
//!
//! | case | flaw | outcome |
//! |---|---|---|
//! | g-0002 | the supplier's VAT number has a different last digit | wrong, **silent** (still a valid-looking number) |
//! | g-0003 | the supplier's name is missing its "CC", read with low confidence | wrong, flagged (by the low confidence alone) |
//! | g-0004 | the invoice date is not found | missing, flagged (a tax invoice needs one) |
//! | g-0005 | a wrong gross total, and the case is sent to review | wrong, flagged |
//! | g-0006 | a recipient the document does not have | spurious, silent (non-critical) |
//! | g-0007 | a description with different case and spacing | still correct |
//! | g-0008 | the credit note's reference is not found | missing, **silent** |
//!
//! So `clearcut-gold score` fails on the dev split with exactly two silent
//! critical errors, and passes on the test split.

use super::{cases, SampleError};
use crate::score::result::{ModelRef, TokenUsage};
use crate::score::{Disposition, RunResult};
use clearcut_core::event::{Confidence, Extractor};
use clearcut_core::extraction::{Extraction, Party, Sourced};
use clearcut_core::Money;
use std::fs;
use std::io;
use std::path::Path;

/// The confidence a typical model result carries.
const USUAL: u16 = 950;

/// One result per sample case, in order.
pub fn results() -> Vec<RunResult> {
    cases()
        .into_iter()
        .map(|case| {
            let id = case.label.case.clone();
            let mut extraction = case
                .label
                .truth
                .to_extraction(&id, "sample-model")
                .expect("the sample truth is exact");
            extraction.extractor = Extractor::Model {
                name: "sample-model".to_string(),
                prompt_version: "p0".to_string(),
            };
            set_confidence(
                &mut extraction,
                Confidence::new(USUAL).expect("a valid confidence"),
            );
            let mut disposition = Disposition::Done;

            match id.as_str() {
                "g-0002" => {
                    let vat = extraction
                        .supplier
                        .as_mut()
                        .and_then(|s| s.value.vat_number.as_mut())
                        .expect("a VAT number");
                    vat.pop();
                    vat.push('2');
                }
                "g-0003" => {
                    let supplier = extraction.supplier.as_mut().expect("a supplier");
                    supplier.value.name = "Tafelberg Print Works".to_string();
                    supplier.confidence = Confidence::new(400).expect("a valid confidence");
                }
                "g-0004" => extraction.invoice_date = None,
                "g-0005" => {
                    disposition = Disposition::NeedsReview;
                    let totals = extraction.stated_totals.as_mut().expect("totals");
                    totals.value.gross =
                        Money::new(totals.value.gross.minor + 100, totals.value.gross.currency);
                }
                "g-0006" => {
                    extraction.recipient = Some(Sourced {
                        value: Party {
                            name: "Invented Client (Pty) Ltd".to_string(),
                            address: Some("9 Nowhere Street, Midrand, 1685".to_string()),
                            vat_number: None,
                        },
                        page: std::num::NonZeroU32::new(1).expect("page 1"),
                        locator: None,
                        confidence: Confidence::new(USUAL).expect("a valid confidence"),
                    });
                }
                "g-0007" => extraction.lines[0].value.description = "CLOUD  hosting, MARCH".to_string(),
                "g-0008" => extraction.credit_note = None,
                _ => {}
            }

            RunResult {
                case: id,
                disposition,
                extraction: Some(extraction),
                model: Some(ModelRef {
                    name: "sample-model".to_string(),
                    prompt_version: "p0".to_string(),
                }),
                usage: Some(TokenUsage {
                    input_tokens: 1_200,
                    output_tokens: 300,
                }),
            }
        })
        .collect()
}

fn set_confidence(extraction: &mut Extraction, confidence: Confidence) {
    if let Some(v) = extraction.invoice_number.as_mut() {
        v.confidence = confidence;
    }
    if let Some(v) = extraction.invoice_date.as_mut() {
        v.confidence = confidence;
    }
    if let Some(v) = extraction.supplier.as_mut() {
        v.confidence = confidence;
    }
    if let Some(v) = extraction.recipient.as_mut() {
        v.confidence = confidence;
    }
    for line in &mut extraction.lines {
        line.confidence = confidence;
    }
    if let Some(v) = extraction.stated_totals.as_mut() {
        v.confidence = confidence;
    }
    if let Some(v) = extraction.due.as_mut() {
        v.confidence = confidence;
    }
    if let Some(v) = extraction.credit_note.as_mut() {
        v.confidence = confidence;
    }
}

/// Writes the sample results into `dir` as `<case>.json`; the folder must not
/// exist or must be empty.
pub fn write_sample_results(dir: &Path) -> Result<(), SampleError> {
    if dir.exists() && fs::read_dir(dir)?.next().is_some() {
        return Err(SampleError::NotEmpty);
    }
    fs::create_dir_all(dir)?;
    for result in results() {
        let mut text =
            serde_json::to_string_pretty(&result).map_err(|_| SampleError::Io(io::ErrorKind::InvalidData))?;
        text.push('\n');
        fs::write(dir.join(format!("{}.json", result.case)), text)?;
    }
    Ok(())
}
