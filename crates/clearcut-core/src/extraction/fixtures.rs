// Written by Kone Tshivhinda & Claude
//! Test builders shared by the validation and derivation tests.

use super::{DueTerms, ExtractedLine, Extraction, Party, Sourced};
use crate::event::{Confidence, DocumentId, Extractor};
use crate::{Currency, InvoiceTotals, Money, TaxCategory};
use std::num::NonZeroU32;
use time::macros::date as d;

pub(crate) fn zar(minor: i64) -> Money {
    Money::zar(minor)
}

/// A value read from page 1 at 95% confidence.
pub(crate) fn at<T>(value: T) -> Sourced<T> {
    Sourced {
        value,
        page: NonZeroU32::new(1).unwrap(),
        locator: None,
        confidence: Confidence::new(950).unwrap(),
    }
}

pub(crate) fn line(
    description: &str,
    qty: i64,
    price: i64,
    total: i64,
    tax: TaxCategory,
) -> Sourced<ExtractedLine> {
    at(ExtractedLine {
        description: description.into(),
        quantity_milli: qty,
        unit_price: zar(price),
        line_total: zar(total),
        tax,
    })
}

pub(crate) fn party(name: &str, vat: &str) -> Sourced<Party> {
    at(Party {
        name: name.into(),
        address: Some("1 Main Rd, Rosebank".into()),
        vat_number: Some(vat.into()),
    })
}

/// A complete, correct invoice worth R18 250.00 (so a *full* tax invoice):
/// 10 x R1 500.00 standard-rated + 1 x R1 000.00 zero-rated.
pub(crate) fn clean() -> Extraction {
    Extraction {
        document: DocumentId::new("doc_1"),
        extractor: Extractor::System,
        currency: Currency::Zar,
        invoice_number: Some(at("INV-000001".to_string())),
        invoice_date: Some(at(d!(2026 - 03 - 31))),
        supplier: Some(party("Acme (Pty) Ltd", "4123456789")),
        recipient: Some(party("Client Co", "4987654321")),
        lines: vec![
            line("Consulting", 10_000, 150_000, 1_500_000, TaxCategory::Standard),
            line("Export licence", 1_000, 100_000, 100_000, TaxCategory::ZeroRated),
        ],
        stated_totals: Some(at(InvoiceTotals {
            net: zar(1_600_000),
            vat: zar(225_000),
            gross: zar(1_825_000),
        })),
        carried: vec![],
        due: Some(at(DueTerms {
            due: d!(2026 - 04 - 30),
            terms: Some("30 days".into()),
        })),
        credit_note: None,
    }
}

pub(crate) fn correct_totals() -> InvoiceTotals {
    InvoiceTotals {
        net: zar(1_600_000),
        vat: zar(225_000),
        gross: zar(1_825_000),
    }
}
