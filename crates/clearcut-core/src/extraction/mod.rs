// Written by Kone Tshivhinda & Claude
//! What an extractor reports for one uploaded document, before ClearCut
//! trusts any of it.
//!
//! An [`Extraction`] is **untrusted data**: it comes from OCR or a language
//! model reading a document that may contain anything. It is plain data with
//! public fields so it can be filled from JSON (the worker's structured
//! output). Nothing here is believed until [`validate`] has checked it and
//! the events derived from it have passed [`Event::new`](crate::event::Event::new).
//!
//! Every value is a [`Sourced`] value: it carries the page it was read from,
//! optionally where on the page, and the extractor's confidence, so each fact
//! can later be cited.

mod validate;

pub use validate::{validate, Finding, Validation};

use crate::event::{Citation, Confidence, DocumentId, Extractor, Locator};
use crate::{Currency, InvoiceTotals, Money, TaxCategory};
use serde::{Deserialize, Serialize};
use std::num::NonZeroU32;
use time::Date;

/// A value read from a document, with where it came from.
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sourced<T> {
    pub value: T,
    /// 1-based page the value was read from.
    pub page: NonZeroU32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locator: Option<Locator>,
    pub confidence: Confidence,
}

impl<T> Sourced<T> {
    /// The citation for this value in `document`.
    pub fn citation(&self, document: &DocumentId) -> Citation {
        Citation {
            document: document.clone(),
            page: self.page,
            locator: self.locator,
        }
    }
}

/// A supplier or recipient as printed on the document.
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Party {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vat_number: Option<String>,
}

/// One line as printed. `line_total` is what the document says; `validate`
/// recomputes it from quantity and unit price.
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractedLine {
    pub description: String,
    /// Quantity in thousandths (1000 = 1 unit).
    pub quantity_milli: i64,
    /// Unit price excluding VAT.
    pub unit_price: Money,
    /// The line total excluding VAT, as printed.
    pub line_total: Money,
    pub tax: TaxCategory,
}

/// A due date and, if printed, the payment terms.
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DueTerms {
    #[cfg_attr(feature = "schemars", schemars(with = "String"))]
    pub due: Date,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terms: Option<String>,
}

/// A credit note's reference back to the invoice it adjusts.
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreditNoteRef {
    pub original_invoice_number: String,
    pub amount: Money,
}

/// Everything an extractor read from one document. Only the document, the
/// extractor and the currency are required; anything it could not find is
/// simply absent, and `validate` reports what that means.
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Extraction {
    pub document: DocumentId,
    /// Who produced this extraction (rule, model or human).
    pub extractor: Extractor,
    pub currency: Currency,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invoice_number: Option<Sourced<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schemars", schemars(with = "Option<Sourced<String>>"))]
    pub invoice_date: Option<Sourced<Date>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supplier: Option<Sourced<Party>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipient: Option<Sourced<Party>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<Sourced<ExtractedLine>>,
    /// The net, VAT and gross totals as printed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stated_totals: Option<Sourced<InvoiceTotals>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<Sourced<DueTerms>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_note: Option<Sourced<CreditNoteRef>>,
}

#[cfg(test)]
mod tests;
