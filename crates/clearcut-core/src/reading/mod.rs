// Written by Kone Tshivhinda & Claude
//! What a model is asked to return for one page, and the exact conversion of
//! it into an [`Extraction`] (ADR-0013).
//!
//! A [`PageReading`] is a flat shape of **text**: every field is required (an
//! empty string or list means "not shown on this page"), amounts and quantities
//! are decimal text, dates are `YYYY-MM-DD`. That keeps the JSON Schema inside
//! the API's structured-output limits (no optional or union properties) and
//! keeps arithmetic off the model: Rust turns the text into exact integers.
//!
//! The model is **never** asked for the page, a locator, a confidence, the
//! document or the extractor: the caller knows the page and the document, and
//! confidence comes from the tier and the checks, not from the model's opinion
//! of itself.
//!
//! [`PageReading::into_extraction`] never guesses. A value that cannot be read
//! exactly is **left out and reported** as a [`ReadingProblem`] (a path and a
//! fixed kind, never the text), and nothing is defaulted or repaired. Any
//! problem sends the page to review; `validate` then checks what remains.
//!
//! The doc comments on the reading types are the field descriptions in the
//! generated JSON Schema, so they are written as instructions to the model, and
//! changing one changes the schema (and `READING_SHAPE_VERSION` must change).

use crate::decimal::{parse_money, parse_quantity_milli};
use crate::event::{Confidence, DocumentId, Extractor};
use crate::extraction::{CarriedTotals, CreditNoteRef, DueTerms, ExtractedLine, Extraction, Party, Sourced};
use crate::tax::ZA_VAT;
use crate::{Currency, InvoiceTotals, TaxCategory};
use serde::{Deserialize, Serialize};
use std::num::NonZeroU32;
use time::{Date, Month};

/// The version of the shape and its descriptions. It goes into the extractor's
/// `prompt_version`, so every stored extraction says which shape produced it.
pub const READING_SHAPE_VERSION: &str = "reading-v3";

/// At most this many lines are kept from one page.
pub const MAX_LINES_PER_PAGE: usize = 200;
const MAX_INVOICE_NUMBER: usize = 100;
const MAX_NAME: usize = 200;
const MAX_ADDRESS: usize = 500;
const MAX_VAT_NUMBER: usize = 50;
const MAX_DESCRIPTION: usize = 500;
const MAX_TERMS: usize = 500;
/// Dates outside these years are almost certainly a misread digit.
const FIRST_YEAR: i32 = 1990;
const LAST_YEAR: i32 = 2100;

/// The currency printed on the page, or not_shown if no currency is printed or implied on this page.
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReadCurrency {
    #[serde(rename = "ZAR")]
    Zar,
    #[serde(rename = "USD")]
    Usd,
    #[serde(rename = "EUR")]
    Eur,
    #[serde(rename = "GBP")]
    Gbp,
    #[serde(rename = "not_shown")]
    NotShown,
}

/// The VAT treatment shown for a line: standard (15% VAT), zero_rated, exempt, out_of_scope, or
/// not_shown if the page does not show how the line is taxed.
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadTax {
    Standard,
    ZeroRated,
    Exempt,
    OutOfScope,
    NotShown,
}

/// Whether the line amounts on this page include VAT: yes if the page says its prices include
/// VAT, no if they exclude it, not_shown if the page does not say. Never add or remove VAT
/// yourself; write amounts exactly as printed.
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadVatBasis {
    Yes,
    No,
    NotShown,
}

/// A supplier or recipient. Empty text for anything not printed.
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadParty {
    /// The name as printed. Empty if no party is shown.
    pub name: String,
    /// The address as printed. Empty if none.
    pub address: String,
    /// The VAT registration number as printed. Empty if none.
    pub vat_number: String,
}

/// One line item as printed.
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadLine {
    /// The description as printed.
    pub description: String,
    /// The quantity: digits and at most one dot, at most 3 decimals, such as 2 or 1.5.
    /// If the line shows only an amount and no quantity, write 1.
    pub quantity: String,
    /// The price of one unit exactly as printed: digits and at most one dot, at most 2
    /// decimals, no thousands separators, no currency symbol, a leading minus for
    /// a negative amount, such as 1500.00. Do not add or remove VAT. If only a line
    /// amount is printed, repeat it here.
    pub unit_price: String,
    /// The line amount exactly as printed, written the same way.
    pub line_total: String,
    /// How the page shows this line is taxed.
    pub tax: ReadTax,
}

/// The totals as printed. Write all three, or leave all three empty.
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadTotals {
    /// The total excluding VAT, written like a unit price. Empty if not printed.
    pub net: String,
    /// The VAT amount, written like a unit price. Empty if not printed.
    pub vat: String,
    /// The total including VAT, written like a unit price. Empty if not printed.
    pub gross: String,
}

/// The page-continuation figures printed at the top and foot of a long invoice's page
/// (ADR-0022). These are not the invoice's net, VAT or gross totals: they are the amount a page
/// starts with and the amount it hands to the next page. Empty text for either that is not
/// printed.
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadCarried {
    /// The amount brought forward at the top of this page, for example "Brought forward",
    /// "Balance brought forward" or "B/F", written like a unit price. Empty if this page does
    /// not print one.
    pub brought_forward: String,
    /// The amount carried forward at the foot of this page, for example "Carried forward",
    /// "Balance carried forward" or "C/F", written like a unit price. Empty if this page does
    /// not print one.
    pub carried_forward: String,
}

/// The due date and payment terms as printed.
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadDue {
    /// The due date as YYYY-MM-DD (South African dates are day/month/year).
    /// Empty if no due date is printed.
    pub date: String,
    /// The payment terms as printed, such as "30 days". Empty if none.
    pub terms: String,
}

/// For a credit note: the invoice it adjusts. Empty text if this is not a credit note.
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadCreditNote {
    /// The number of the invoice being credited, as printed.
    pub original_invoice_number: String,
    /// The credited amount, written like a unit price.
    pub amount: String,
}

/// Everything printed on one page of a document, as text. Empty text or an
/// empty list means "not shown on this page". Never guess a value that is not
/// printed, and copy numbers exactly.
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageReading {
    /// The currency of the amounts on this page.
    pub currency: ReadCurrency,
    /// Whether the line amounts on this page include VAT.
    pub amounts_include_vat: ReadVatBasis,
    /// The invoice or credit note number as printed. Empty if not on this page.
    pub invoice_number: String,
    /// The invoice date as YYYY-MM-DD (South African dates are day/month/year).
    /// Empty if not on this page.
    pub invoice_date: String,
    /// Who issued the document.
    pub supplier: ReadParty,
    /// Who the document is addressed to.
    pub recipient: ReadParty,
    /// The line items on this page, in order. Empty if none.
    pub lines: Vec<ReadLine>,
    /// The totals, if printed on this page.
    pub totals: ReadTotals,
    /// The page-continuation figures, if this page prints any (ADR-0022).
    pub carried: ReadCarried,
    /// The due date and terms, if printed on this page.
    pub due: ReadDue,
    /// The credited invoice, if this is a credit note.
    pub credit_note: ReadCreditNote,
}

/// The JSON Schema sent to the model (only with the `schemars` feature).
#[cfg(feature = "schemars")]
pub fn reading_schema() -> schemars::Schema {
    // Subschemas are written in place (no `$ref`), so the schema is one self-contained
    // tree and every enum is a plain `enum`, not a union.
    let settings = schemars::generate::SchemaSettings::draft2020_12().with(|s| s.inline_subschemas = true);
    settings.into_generator().into_root_schema_for::<PageReading>()
}

/// What the caller knows that the model must not decide.
#[derive(Debug, Clone)]
pub struct ConversionContext {
    pub document: DocumentId,
    /// The page that was read (1-based).
    pub page: NonZeroU32,
    pub extractor: Extractor,
    /// Attached to every value; comes from the tier and the checks.
    pub confidence: Confidence,
    /// A currency already established by **another page** of this document.
    /// Never a tenant default.
    pub currency_known: Option<Currency>,
}

/// Why a value was left out. These words are stored and shown to people, so
/// they are fixed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProblemKind {
    BadAmount,
    BadQuantity,
    BadDate,
    BadCharacters,
    TooLong,
    TooManyLines,
    MissingRequired,
    TaxNotShown,
    /// Only part of a group that must be complete was printed.
    Incomplete,
    /// The line amounts include VAT, which is not backed out here (ADR-0014 §6).
    AmountsIncludeVat,
    /// A figure that does not occur in the page's own text. The conversion never produces
    /// this: it is named to the next model in a failure report (ADR-0018 section 3).
    NotOnPage,
}

/// A value that could not be read exactly. `path` names where
/// (`lines/2/unit_price`); neither field ever holds text from the document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReadingProblem {
    pub path: String,
    pub kind: ProblemKind,
}

/// The extraction, and everything that was left out of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversion {
    pub extraction: Extraction,
    /// In a fixed order: header fields, supplier, recipient, lines, totals, due, credit note.
    pub problems: Vec<ReadingProblem>,
}

/// Why no extraction can be made at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ConversionError {
    #[error("no currency is shown on this page or known from another page")]
    CurrencyNotShown,
    #[error("this page shows a different currency from the rest of the document")]
    CurrencyMismatch,
}

impl PageReading {
    /// Converts this reading into an [`Extraction`], leaving out and reporting
    /// whatever cannot be read exactly. Pure: no I/O, no clock, no defaults.
    pub fn into_extraction(&self, context: &ConversionContext) -> Result<Conversion, ConversionError> {
        let shown = match self.currency {
            ReadCurrency::Zar => Some(Currency::Zar),
            ReadCurrency::Usd => Some(Currency::Usd),
            ReadCurrency::Eur => Some(Currency::Eur),
            ReadCurrency::Gbp => Some(Currency::Gbp),
            ReadCurrency::NotShown => None,
        };
        let currency = match (shown, context.currency_known) {
            (Some(shown), Some(known)) if shown != known => return Err(ConversionError::CurrencyMismatch),
            (Some(currency), _) | (None, Some(currency)) => currency,
            (None, None) => return Err(ConversionError::CurrencyNotShown),
        };

        let mut converter = Converter {
            currency,
            problems: Vec::new(),
        };
        let invoice_number = converter.text("invoice_number", &self.invoice_number, MAX_INVOICE_NUMBER);
        let invoice_date = converter.date("invoice_date", &self.invoice_date);
        let supplier = converter.party("supplier", &self.supplier);
        let recipient = converter.party("recipient", &self.recipient);
        let derived_tax = converter.derive_tax(&self.lines, &self.totals);
        let lines = converter.lines(&self.lines, self.amounts_include_vat, derived_tax);
        let stated_totals = converter.totals(&self.totals);
        let carried = converter.carried(&self.carried);
        let due = converter.due(&self.due);
        let credit_note = converter.credit_note(&self.credit_note);

        let extraction = Extraction {
            document: context.document.clone(),
            extractor: context.extractor.clone(),
            currency,
            invoice_number: invoice_number.map(|v| sourced(context, v)),
            invoice_date: invoice_date.map(|v| sourced(context, v)),
            supplier: supplier.map(|v| sourced(context, v)),
            recipient: recipient.map(|v| sourced(context, v)),
            lines: lines.into_iter().map(|v| sourced(context, v)).collect(),
            stated_totals: stated_totals.map(|v| sourced(context, v)),
            carried: carried.into_iter().map(|v| sourced(context, v)).collect(),
            due: due.map(|v| sourced(context, v)),
            credit_note: credit_note.map(|v| sourced(context, v)),
        };
        Ok(Conversion {
            extraction,
            problems: converter.problems,
        })
    }
}

/// Attaches the page and confidence the caller knows; no locator in v1.
fn sourced<T>(context: &ConversionContext, value: T) -> Sourced<T> {
    Sourced {
        value,
        page: context.page,
        locator: None,
        confidence: context.confidence,
    }
}

/// What `text` made of a string.
enum Text {
    Absent,
    Value(String),
    Bad,
}

struct Converter {
    currency: Currency,
    problems: Vec<ReadingProblem>,
}

impl Converter {
    fn problem(&mut self, path: impl Into<String>, kind: ProblemKind) {
        self.problems.push(ReadingProblem {
            path: path.into(),
            kind,
        });
    }

    /// Trimmed text, with control characters (other than a new line) and
    /// over-long values refused. A refused value is reported at `path`.
    fn read_text(&mut self, path: &str, raw: &str, max_chars: usize) -> Text {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Text::Absent;
        }
        if trimmed.chars().any(|c| c.is_control() && c != '\n') {
            self.problem(path, ProblemKind::BadCharacters);
            return Text::Bad;
        }
        if trimmed.chars().count() > max_chars {
            self.problem(path, ProblemKind::TooLong);
            return Text::Bad;
        }
        Text::Value(trimmed.to_string())
    }

    fn text(&mut self, path: &str, raw: &str, max_chars: usize) -> Option<String> {
        match self.read_text(path, raw, max_chars) {
            Text::Value(text) => Some(text),
            Text::Absent | Text::Bad => None,
        }
    }

    /// `YYYY-MM-DD` and nothing else: a real calendar date in a believable year.
    fn date(&mut self, path: &str, raw: &str) -> Option<Date> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        let parsed = parse_iso_date(trimmed);
        if parsed.is_none() {
            self.problem(path, ProblemKind::BadDate);
        }
        parsed
    }

    /// Money in the page's currency. `Err(())` means a problem was reported;
    /// `Ok(None)` means nothing was printed.
    fn amount(&mut self, path: &str, raw: &str) -> Result<Option<crate::Money>, ()> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }
        match parse_money(trimmed, self.currency) {
            Ok(money) => Ok(Some(money)),
            Err(_) => {
                self.problem(path, ProblemKind::BadAmount);
                Err(())
            }
        }
    }

    fn party(&mut self, path: &str, raw: &ReadParty) -> Option<Party> {
        let name = self.read_text(&format!("{path}/name"), &raw.name, MAX_NAME);
        let address = self.text(&format!("{path}/address"), &raw.address, MAX_ADDRESS);
        let vat_number = self.text(&format!("{path}/vat_number"), &raw.vat_number, MAX_VAT_NUMBER);
        match name {
            Text::Value(name) => Some(Party {
                name,
                address,
                vat_number,
            }),
            Text::Bad => None,
            Text::Absent => {
                if address.is_some() || vat_number.is_some() {
                    self.problem(path, ProblemKind::Incomplete);
                }
                None
            }
        }
    }

    /// The tax category of every line, when no line shows one but the page's own totals prove
    /// it: the lines add up to the printed net, and the printed VAT is exactly the standard
    /// rate of that net, so every line is standard-rated. A total with no VAT fits exempt,
    /// zero-rated and out-of-scope alike, so it proves nothing and nothing is derived.
    /// Nothing is reported here; a line left without a category is reported by `line`.
    fn derive_tax(&self, raw: &[ReadLine], totals: &ReadTotals) -> Option<TaxCategory> {
        if raw.is_empty() || raw.iter().any(|line| line.tax != ReadTax::NotShown) {
            return None;
        }
        let money = |text: &str| parse_money(text.trim(), self.currency).ok();
        let (net, vat, gross) = (money(&totals.net)?, money(&totals.vat)?, money(&totals.gross)?);
        let mut sum = crate::Money::new(0, self.currency);
        for line in raw {
            sum = sum.checked_add(money(&line.line_total)?).ok()?;
        }
        let standard = net.percent_bps(ZA_VAT.rate_bps(TaxCategory::Standard)).ok()?;
        let agrees = vat.minor != 0 && sum == net && vat == standard && net.checked_add(vat).ok()? == gross;
        agrees.then_some(TaxCategory::Standard)
    }

    fn lines(
        &mut self,
        raw: &[ReadLine],
        basis: ReadVatBasis,
        derived_tax: Option<TaxCategory>,
    ) -> Vec<ExtractedLine> {
        // VAT-inclusive amounts cannot be used as amounts excluding VAT, and backing the
        // VAT out is a rule that has not been agreed yet: report it and read no lines.
        if basis == ReadVatBasis::Yes && !raw.is_empty() {
            self.problem("amounts_include_vat", ProblemKind::AmountsIncludeVat);
            return Vec::new();
        }
        let mut lines = Vec::new();
        for (index, line) in raw.iter().take(MAX_LINES_PER_PAGE).enumerate() {
            if let Some(line) = self.line(index + 1, line, derived_tax) {
                lines.push(line);
            }
        }
        if raw.len() > MAX_LINES_PER_PAGE {
            self.problem("lines", ProblemKind::TooManyLines);
        }
        lines
    }

    /// Reports every problem in the line, and returns it only if there were none.
    fn line(
        &mut self,
        number: usize,
        raw: &ReadLine,
        derived_tax: Option<TaxCategory>,
    ) -> Option<ExtractedLine> {
        let at = |field: &str| format!("lines/{number}/{field}");
        let mut complete = true;

        let description = match self.read_text(&at("description"), &raw.description, MAX_DESCRIPTION) {
            Text::Value(text) => text,
            Text::Absent => String::new(),
            Text::Bad => {
                complete = false;
                String::new()
            }
        };

        let quantity_text = raw.quantity.trim();
        let quantity_milli = if quantity_text.is_empty() {
            self.problem(at("quantity"), ProblemKind::MissingRequired);
            complete = false;
            0
        } else {
            match parse_quantity_milli(quantity_text) {
                Ok(milli) => milli,
                Err(_) => {
                    self.problem(at("quantity"), ProblemKind::BadQuantity);
                    complete = false;
                    0
                }
            }
        };

        let mut required_amount = |converter: &mut Self, field: &str, text: &str| -> crate::Money {
            match converter.amount(&at(field), text) {
                Ok(Some(money)) => money,
                Ok(None) => {
                    converter.problem(at(field), ProblemKind::MissingRequired);
                    complete = false;
                    crate::Money::new(0, converter.currency)
                }
                Err(()) => {
                    complete = false;
                    crate::Money::new(0, converter.currency)
                }
            }
        };
        let unit_price = required_amount(self, "unit_price", &raw.unit_price);
        let line_total = required_amount(self, "line_total", &raw.line_total);

        let tax = match raw.tax {
            ReadTax::Standard => Some(TaxCategory::Standard),
            ReadTax::ZeroRated => Some(TaxCategory::ZeroRated),
            ReadTax::Exempt => Some(TaxCategory::Exempt),
            ReadTax::OutOfScope => Some(TaxCategory::OutOfScope),
            ReadTax::NotShown => derived_tax,
        };
        let Some(tax) = tax else {
            self.problem(at("tax"), ProblemKind::TaxNotShown);
            return None;
        };

        complete.then_some(ExtractedLine {
            description,
            quantity_milli,
            unit_price,
            line_total,
            tax,
        })
    }

    /// All three or nothing: `InvoiceTotals` cannot hold a part.
    fn totals(&mut self, raw: &ReadTotals) -> Option<InvoiceTotals> {
        let net = self.amount("totals/net", &raw.net);
        let vat = self.amount("totals/vat", &raw.vat);
        let gross = self.amount("totals/gross", &raw.gross);
        let (Ok(net), Ok(vat), Ok(gross)) = (net, vat, gross) else {
            return None;
        };
        match (net, vat, gross) {
            (Some(net), Some(vat), Some(gross)) => Some(InvoiceTotals { net, vat, gross }),
            (None, None, None) => None,
            _ => {
                self.problem("totals", ProblemKind::Incomplete);
                None
            }
        }
    }

    /// The page-continuation figures (ADR-0022). Each figure is independent, so a page may
    /// print one and not the other; a figure that cannot be read is reported and left out.
    /// No figure at all means no carried fact.
    fn carried(&mut self, raw: &ReadCarried) -> Option<CarriedTotals> {
        let brought_forward = self
            .amount("carried/brought_forward", &raw.brought_forward)
            .ok()
            .flatten();
        let carried_forward = self
            .amount("carried/carried_forward", &raw.carried_forward)
            .ok()
            .flatten();
        (brought_forward.is_some() || carried_forward.is_some()).then_some(CarriedTotals {
            brought_forward,
            carried_forward,
        })
    }

    fn due(&mut self, raw: &ReadDue) -> Option<DueTerms> {
        let terms = self.text("due/terms", &raw.terms, MAX_TERMS);
        if raw.date.trim().is_empty() {
            if terms.is_some() {
                self.problem("due", ProblemKind::Incomplete);
            }
            return None;
        }
        let due = self.date("due/date", &raw.date)?;
        Some(DueTerms { due, terms })
    }

    fn credit_note(&mut self, raw: &ReadCreditNote) -> Option<CreditNoteRef> {
        let number = self.read_text(
            "credit_note/original_invoice_number",
            &raw.original_invoice_number,
            MAX_INVOICE_NUMBER,
        );
        let amount = self.amount("credit_note/amount", &raw.amount);
        match (number, amount) {
            (Text::Value(original_invoice_number), Ok(Some(amount))) => Some(CreditNoteRef {
                original_invoice_number,
                amount,
            }),
            (Text::Absent, Ok(None)) => None,
            (Text::Bad, _) | (_, Err(())) => None,
            _ => {
                self.problem("credit_note", ProblemKind::Incomplete);
                None
            }
        }
    }
}

/// Exactly `YYYY-MM-DD`, ASCII digits only, a real date, in a believable year.
fn parse_iso_date(text: &str) -> Option<Date> {
    let bytes = text.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    let number = |range: std::ops::Range<usize>| -> Option<u32> {
        let part = &bytes[range];
        if !part.iter().all(u8::is_ascii_digit) {
            return None;
        }
        part.iter()
            .try_fold(0u32, |n, d| Some(n * 10 + u32::from(d - b'0')))
    };
    let year = i32::try_from(number(0..4)?).ok()?;
    let month = Month::try_from(u8::try_from(number(5..7)?).ok()?).ok()?;
    let day = u8::try_from(number(8..10)?).ok()?;
    if !(FIRST_YEAR..=LAST_YEAR).contains(&year) {
        return None;
    }
    Date::from_calendar_date(year, month, day).ok()
}

#[cfg(test)]
mod tests;

mod legacy;
pub use legacy::{StoredReading, LEGACY_PROMPT_VERSION};
