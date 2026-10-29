// Written by Kone Tshivhinda & Claude
//! The label file: the truth about one gold-set document (ADR-0010).
//!
//! A [`Label`] is what a person writes in `label.json`. Its `truth` part
//! mirrors the core [`Extraction`], so that scoring a run is a plain
//! comparison of two values of the same kind, with two differences made for
//! people: amounts are decimal **text** (`"1500.00"`, `"10"`) that is turned
//! into cents and thousandths exactly, and there is no confidence (a person is
//! certain by definition, so the converted extraction says so).
//!
//! Parsing is strict. Every struct rejects unknown fields, so a typo in a field
//! name is an error and never a silently dropped value. Errors say **where**
//! (line and column, or a field name) and never repeat what the file said,
//! because the file describes a real document.

use crate::decimal::{parse_money, parse_quantity_milli, DecimalError};
use clearcut_core::event::{Confidence, DocumentId, Extractor};
use clearcut_core::extraction::{CreditNoteRef, DueTerms, ExtractedLine, Extraction, Party, Sourced};
use clearcut_core::{Currency, InvoiceTotals, TaxCategory};
use serde::{Deserialize, Serialize};
use std::num::NonZeroU32;
use time::Date;

/// The only label format there is so far.
pub const FORMAT: u32 = 1;

/// Which half of the gold set a case belongs to. Prompts and thresholds are
/// tuned on `dev`; `test` is the release gate and is run sparingly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Split {
    Dev,
    Test,
}

/// Where a document came from. Nothing else is allowed in the set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// A document the operator of ClearCut owns.
    Own,
    /// A document whose issuer and recipient gave written consent.
    Consented,
    /// Made up (or properly licensed), so it contains no one's data.
    Synthetic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentRef {
    /// File name inside the case folder. A plain name, never a path.
    pub file: String,
    /// SHA-256 of the file, 64 lowercase hex digits.
    pub sha256: String,
    pub pages: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub tier: Tier,
    /// Required for `own` and `consented` documents.
    #[serde(default)]
    pub consent_ref: Option<String>,
    /// The document was anonymised before it entered the set.
    pub anonymised: bool,
    /// The date after which the case must be deleted.
    #[serde(default)]
    pub retain_until: Option<Date>,
    pub labelled_by: String,
    #[serde(default)]
    pub reviewed_by: Option<String>,
}

/// What the validator should report for this document. Real suppliers make
/// mistakes (a total that does not add up), and such documents are valuable, so
/// the label records the document as it is and says which findings are right.
/// Keys are in [`crate::keys`] form, for example `totals_mismatch:vat`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expect {
    #[serde(default)]
    pub findings: Vec<String>,
}

/// A value and the page it is on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cited<T> {
    pub value: T,
    pub page: NonZeroU32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartyLabel {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vat_number: Option<String>,
}

/// One line as printed. Amounts are decimal text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LineLabel {
    pub page: NonZeroU32,
    pub description: String,
    /// Units, up to three decimals (`"2.5"`).
    pub quantity: String,
    /// Excluding VAT, up to two decimals.
    pub unit_price: String,
    /// The line total excluding VAT, as printed.
    pub line_total: String,
    pub tax: TaxCategory,
}

/// The totals as printed on the document (not recomputed).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TotalsLabel {
    pub net: String,
    pub vat: String,
    pub gross: String,
    pub page: NonZeroU32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DueLabel {
    pub date: Date,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terms: Option<String>,
    pub page: NonZeroU32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreditNoteLabel {
    pub original_invoice_number: String,
    pub amount: String,
    pub page: NonZeroU32,
}

/// The facts on the document, in the shape of an [`Extraction`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Truth {
    pub currency: Currency,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invoice_number: Option<Cited<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invoice_date: Option<Cited<Date>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supplier: Option<Cited<PartyLabel>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipient: Option<Cited<PartyLabel>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<LineLabel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stated_totals: Option<TotalsLabel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<DueLabel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_note: Option<CreditNoteLabel>,
}

/// The contents of one `label.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Label {
    pub format: u32,
    pub case: String,
    pub document: DocumentRef,
    #[serde(default)]
    pub tags: Vec<String>,
    pub split: Split,
    pub provenance: Provenance,
    pub truth: Truth,
    #[serde(default)]
    pub expect: Expect,
}

/// Why a label file could not be read. Positions only, never content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("the file is not valid JSON (line {line}, column {column})")]
    NotJson { line: usize, column: usize },
    #[error("the file does not have the expected shape (line {line}, column {column})")]
    WrongShape { line: usize, column: usize },
}

/// A way a label that parsed is still wrong. No variant holds text from the
/// label, so printing a problem can never leak the document.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LabelProblem {
    #[error("the label format is not supported (only format {FORMAT} is)")]
    UnsupportedFormat,
    #[error("the case id must look like g-0001")]
    BadCaseId,
    #[error("the document file name must be a plain name of letters, digits, dot, dash and underscore")]
    BadFileName,
    #[error("the sha256 must be 64 lowercase hex digits")]
    BadSha256,
    #[error("the page count must be between 1 and 10000")]
    PagesOutOfRange,
    #[error("tag number {index} must be 1 to 32 lowercase letters, digits or inner dashes")]
    BadTag { index: usize },
    #[error("tag number {index} repeats an earlier tag")]
    DuplicateTag { index: usize },
    #[error("{field} cites page {page}, which the document does not have")]
    PageOutsideDocument { field: String, page: u32 },
    #[error("a consent reference is required for this tier")]
    ConsentMissing,
    #[error("a retain_until date is required for this tier")]
    RetentionMissing,
    #[error("a document that is not synthetic must be anonymised before it enters the set")]
    NotAnonymised,
    #[error("expect.findings entry number {index} is not a finding key such as totals_mismatch:vat")]
    BadExpectedFinding { index: usize },
    #[error("labelled_by is blank")]
    LabellerMissing,
    #[error("reviewed_by is blank (leave it out until a review happens)")]
    ReviewerBlank,
}

/// What was wrong with one field of the truth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TruthErrorKind {
    Decimal(DecimalError),
    Empty,
}

/// A field of the truth that cannot be converted exactly. Holds the field's
/// name (`lines[2].unit_price`), never its text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{field}: {}", describe(.kind))]
pub struct TruthError {
    pub field: String,
    pub kind: TruthErrorKind,
}

fn describe(kind: &TruthErrorKind) -> String {
    match kind {
        TruthErrorKind::Decimal(error) => error.to_string(),
        TruthErrorKind::Empty => "the value is blank".to_string(),
    }
}

impl Label {
    /// Reads a `label.json`. Errors carry a line and column only.
    pub fn parse(bytes: &[u8]) -> Result<Label, ParseError> {
        serde_json::from_slice(bytes).map_err(|error| {
            let (line, column) = (error.line(), error.column());
            match error.classify() {
                serde_json::error::Category::Data => ParseError::WrongShape { line, column },
                _ => ParseError::NotJson { line, column },
            }
        })
    }

    /// Everything wrong with this label that does not need the document or the
    /// rest of the set. All problems are reported, not just the first.
    pub fn problems(&self) -> Vec<LabelProblem> {
        let mut out = Vec::new();
        if self.format != FORMAT {
            out.push(LabelProblem::UnsupportedFormat);
        }
        if !is_case_id(&self.case) {
            out.push(LabelProblem::BadCaseId);
        }
        if !is_plain_file_name(&self.document.file) {
            out.push(LabelProblem::BadFileName);
        }
        if !is_sha256_hex(&self.document.sha256) {
            out.push(LabelProblem::BadSha256);
        }
        let pages_ok = (1..=MAX_PAGES).contains(&self.document.pages);
        if !pages_ok {
            out.push(LabelProblem::PagesOutOfRange);
        }

        for (index, tag) in self.tags.iter().enumerate() {
            if !is_tag(tag) {
                out.push(LabelProblem::BadTag { index });
            } else if self.tags[..index].contains(tag) {
                out.push(LabelProblem::DuplicateTag { index });
            }
        }

        if pages_ok {
            for (field, page) in self.truth.cited_pages() {
                if page > self.document.pages {
                    out.push(LabelProblem::PageOutsideDocument { field, page });
                }
            }
        }

        for (index, key) in self.expect.findings.iter().enumerate() {
            if !is_finding_key(key) {
                out.push(LabelProblem::BadExpectedFinding { index });
            }
        }

        let provenance = &self.provenance;
        if provenance.tier != Tier::Synthetic {
            if is_blank(provenance.consent_ref.as_deref()) {
                out.push(LabelProblem::ConsentMissing);
            }
            if provenance.retain_until.is_none() {
                out.push(LabelProblem::RetentionMissing);
            }
            if !provenance.anonymised {
                out.push(LabelProblem::NotAnonymised);
            }
        }
        if provenance.labelled_by.trim().is_empty() {
            out.push(LabelProblem::LabellerMissing);
        }
        if provenance
            .reviewed_by
            .as_deref()
            .is_some_and(|name| name.trim().is_empty())
        {
            out.push(LabelProblem::ReviewerBlank);
        }
        out
    }
}

/// Whether `key` has the shape of a finding key (see [`crate::keys`]). Keys
/// from a label are printed in reports, so only this fixed vocabulary is
/// ever echoed back; anything else is reported by position.
pub fn is_finding_key(key: &str) -> bool {
    let digits = |s: &str| (1..=10).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit());
    match key.split_once(':') {
        None => matches!(key, "no_lines" | "totals_out_of_range" | "due_before_issue"),
        Some((kind, rest)) => match kind {
            "currency_mismatch"
            | "non_positive_quantity"
            | "amount_out_of_range"
            | "line_total_mismatch"
            | "balance_mismatch" => digits(rest),
            "totals_mismatch" => matches!(rest, "net" | "vat" | "gross"),
            "compliance" => {
                (1..=40).contains(&rest.len()) && rest.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
            }
            _ => false,
        },
    }
}

const MAX_PAGES: u32 = 10_000;

/// A fact a person is certain of. (A plain function, not a closure: a closure
/// has one fixed type for its arguments, and this is used for many types.)
fn certain<T>(value: T, page: NonZeroU32) -> Sourced<T> {
    Sourced {
        value,
        page,
        locator: None,
        confidence: Confidence::CERTAIN,
    }
}

fn is_blank(text: Option<&str>) -> bool {
    text.is_none_or(|text| text.trim().is_empty())
}

/// `g-` and at least four digits.
pub fn is_case_id(id: &str) -> bool {
    id.strip_prefix("g-")
        .is_some_and(|digits| digits.len() >= 4 && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// Letters, digits, dot, dash and underscore only, so a label can never point
/// outside its own folder, and no name is special on Windows (`C:x`, `x:stream`).
fn is_plain_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && !name.starts_with('.')
        && !name.eq_ignore_ascii_case("label.json")
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

fn is_sha256_hex(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

pub fn is_tag(tag: &str) -> bool {
    !tag.is_empty()
        && tag.len() <= 32
        && !tag.starts_with('-')
        && !tag.ends_with('-')
        && tag
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

impl Truth {
    /// The pages the truth points at, named by field, in a fixed order.
    pub fn cited_pages(&self) -> Vec<(String, u32)> {
        let mut out = Vec::new();
        if let Some(c) = &self.invoice_number {
            out.push(("invoice_number".to_string(), c.page.get()));
        }
        if let Some(c) = &self.invoice_date {
            out.push(("invoice_date".to_string(), c.page.get()));
        }
        if let Some(c) = &self.supplier {
            out.push(("supplier".to_string(), c.page.get()));
        }
        if let Some(c) = &self.recipient {
            out.push(("recipient".to_string(), c.page.get()));
        }
        for (index, line) in self.lines.iter().enumerate() {
            out.push((format!("lines[{index}]"), line.page.get()));
        }
        if let Some(t) = &self.stated_totals {
            out.push(("stated_totals".to_string(), t.page.get()));
        }
        if let Some(d) = &self.due {
            out.push(("due".to_string(), d.page.get()));
        }
        if let Some(c) = &self.credit_note {
            out.push(("credit_note".to_string(), c.page.get()));
        }
        out
    }

    /// Converts the truth into the [`Extraction`] a perfect extractor would
    /// have produced, so scoring can compare like with like. `case` becomes the
    /// document id and `labelled_by` the extractor. Fails, naming the field, on
    /// any amount that is not exact or any blank name or number.
    pub fn to_extraction(&self, case: &str, labelled_by: &str) -> Result<Extraction, TruthError> {
        let currency = self.currency;
        let money = |field: &str, text: &str| {
            parse_money(text, currency).map_err(|error| TruthError {
                field: field.to_string(),
                kind: TruthErrorKind::Decimal(error),
            })
        };
        let nonblank = |field: &str, text: &str| {
            if text.trim().is_empty() {
                Err(TruthError {
                    field: field.to_string(),
                    kind: TruthErrorKind::Empty,
                })
            } else {
                Ok(text.to_string())
            }
        };
        let party = |prefix: &str, cited: &Cited<PartyLabel>| -> Result<Sourced<Party>, TruthError> {
            let label = &cited.value;
            let party = Party {
                name: nonblank(&format!("{prefix}.name"), &label.name)?,
                address: label.address.clone(),
                vat_number: label.vat_number.clone(),
            };
            Ok(certain(party, cited.page))
        };

        let invoice_number = self
            .invoice_number
            .as_ref()
            .map(|c| Ok(certain(nonblank("invoice_number", &c.value)?, c.page)))
            .transpose()?;
        let supplier = self.supplier.as_ref().map(|c| party("supplier", c)).transpose()?;
        let recipient = self
            .recipient
            .as_ref()
            .map(|c| party("recipient", c))
            .transpose()?;

        let mut lines = Vec::with_capacity(self.lines.len());
        for (index, line) in self.lines.iter().enumerate() {
            let at = |name: &str| format!("lines[{index}].{name}");
            let quantity_milli = parse_quantity_milli(&line.quantity).map_err(|error| TruthError {
                field: at("quantity"),
                kind: TruthErrorKind::Decimal(error),
            })?;
            let extracted = ExtractedLine {
                description: line.description.clone(),
                quantity_milli,
                unit_price: money(&at("unit_price"), &line.unit_price)?,
                line_total: money(&at("line_total"), &line.line_total)?,
                tax: line.tax,
            };
            lines.push(certain(extracted, line.page));
        }

        let stated_totals = self
            .stated_totals
            .as_ref()
            .map(|t| {
                let totals = InvoiceTotals {
                    net: money("stated_totals.net", &t.net)?,
                    vat: money("stated_totals.vat", &t.vat)?,
                    gross: money("stated_totals.gross", &t.gross)?,
                };
                Ok(certain(totals, t.page))
            })
            .transpose()?;

        let due = self.due.as_ref().map(|d| {
            certain(
                DueTerms {
                    due: d.date,
                    terms: d.terms.clone(),
                },
                d.page,
            )
        });

        let credit_note = self
            .credit_note
            .as_ref()
            .map(|c| {
                let note = CreditNoteRef {
                    original_invoice_number: c.original_invoice_number.clone(),
                    amount: money("credit_note.amount", &c.amount)?,
                };
                Ok(certain(note, c.page))
            })
            .transpose()?;

        Ok(Extraction {
            document: DocumentId::new(case),
            extractor: Extractor::Human {
                user: labelled_by.to_string(),
            },
            currency,
            invoice_number,
            invoice_date: self.invoice_date.as_ref().map(|c| certain(c.value, c.page)),
            supplier,
            recipient,
            lines,
            stated_totals,
            carried: Vec::new(),
            due,
            credit_note,
        })
    }
}

#[cfg(test)]
pub(crate) mod tests;
