// Written by Kone Tshivhinda & Claude
//! The reading shape (ADR-0013) against the synthetic gold-set cases: write each
//! case's truth out as the text a model would return, one page at a time,
//! convert it back, and require exactly the extraction the label describes.
//! That proves the text formats and the strict parser agree on a realistic
//! spread, including the multi-page cases, without a model or a key.

use clearcut_core::decimal::{format_money, format_quantity};
use clearcut_core::event::{Confidence, DocumentId};
use clearcut_core::extraction::{Extraction, Party, Sourced};
use clearcut_core::reading::*;
use clearcut_core::{Currency, TaxCategory};
use clearcut_eval::label::Label;
use std::num::NonZeroU32;
use std::path::PathBuf;

fn sample_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("sample")
}

fn expected_extractions() -> Vec<(String, Extraction)> {
    let mut cases = Vec::new();
    for entry in std::fs::read_dir(sample_dir()).unwrap() {
        let path = entry.unwrap().path().join("label.json");
        if !path.exists() {
            continue;
        }
        let label = Label::parse(&std::fs::read(&path).unwrap()).unwrap();
        let extraction = label
            .truth
            .to_extraction(&label.case, &label.provenance.labelled_by)
            .unwrap();
        cases.push((label.case, extraction));
    }
    cases.sort_by(|a, b| a.0.cmp(&b.0));
    cases
}

fn text_party(party: Option<&Sourced<Party>>, page: u32) -> ReadParty {
    match party.filter(|p| p.page.get() == page) {
        Some(p) => ReadParty {
            name: p.value.name.clone(),
            address: p.value.address.clone().unwrap_or_default(),
            vat_number: p.value.vat_number.clone().unwrap_or_default(),
        },
        None => ReadParty::default(),
    }
}

/// What a model would return for page `page` of the document `e` describes.
fn reading_of_page(e: &Extraction, page: u32) -> PageReading {
    let on_page = |p: NonZeroU32| p.get() == page;
    let currency = if page == 1 {
        match e.currency {
            Currency::Zar => ReadCurrency::Zar,
            Currency::Usd => ReadCurrency::Usd,
            Currency::Eur => ReadCurrency::Eur,
            Currency::Gbp => ReadCurrency::Gbp,
        }
    } else {
        ReadCurrency::NotShown
    };
    PageReading {
        currency,
        amounts_include_vat: ReadVatBasis::No,
        invoice_number: e
            .invoice_number
            .as_ref()
            .filter(|v| on_page(v.page))
            .map(|v| v.value.clone())
            .unwrap_or_default(),
        invoice_date: e
            .invoice_date
            .as_ref()
            .filter(|v| on_page(v.page))
            .map(|v| v.value.to_string())
            .unwrap_or_default(),
        supplier: text_party(e.supplier.as_ref(), page),
        recipient: text_party(e.recipient.as_ref(), page),
        lines: e
            .lines
            .iter()
            .filter(|l| on_page(l.page))
            .map(|l| ReadLine {
                description: l.value.description.clone(),
                quantity: format_quantity(l.value.quantity_milli),
                unit_price: format_money(l.value.unit_price.minor),
                line_total: format_money(l.value.line_total.minor),
                tax: match l.value.tax {
                    TaxCategory::Standard => ReadTax::Standard,
                    TaxCategory::ZeroRated => ReadTax::ZeroRated,
                    TaxCategory::Exempt => ReadTax::Exempt,
                    TaxCategory::OutOfScope => ReadTax::OutOfScope,
                },
            })
            .collect(),
        totals: e
            .stated_totals
            .as_ref()
            .filter(|t| on_page(t.page))
            .map(|t| ReadTotals {
                net: format_money(t.value.net.minor),
                vat: format_money(t.value.vat.minor),
                gross: format_money(t.value.gross.minor),
            })
            .unwrap_or_default(),
        due: e
            .due
            .as_ref()
            .filter(|d| on_page(d.page))
            .map(|d| ReadDue {
                date: d.value.due.to_string(),
                terms: d.value.terms.clone().unwrap_or_default(),
            })
            .unwrap_or_default(),
        credit_note: e
            .credit_note
            .as_ref()
            .filter(|c| on_page(c.page))
            .map(|c| ReadCreditNote {
                original_invoice_number: c.value.original_invoice_number.clone(),
                amount: format_money(c.value.amount.minor),
            })
            .unwrap_or_default(),
    }
}

fn last_page(e: &Extraction) -> u32 {
    let pages = [
        e.invoice_number.as_ref().map(|v| v.page),
        e.invoice_date.as_ref().map(|v| v.page),
        e.supplier.as_ref().map(|v| v.page),
        e.recipient.as_ref().map(|v| v.page),
        e.stated_totals.as_ref().map(|v| v.page),
        e.due.as_ref().map(|v| v.page),
        e.credit_note.as_ref().map(|v| v.page),
    ];
    pages
        .into_iter()
        .flatten()
        .chain(e.lines.iter().map(|l| l.page))
        .map(NonZeroU32::get)
        .max()
        .unwrap_or(1)
}

/// Puts the pages' conversions back together: one value per field (it must be on
/// exactly one page) and the lines in page order.
fn merge(parts: Vec<Extraction>) -> Extraction {
    fn one<T>(values: Vec<Option<T>>) -> Option<T> {
        let mut found = values.into_iter().flatten();
        let first = found.next();
        assert!(
            found.next().is_none(),
            "a single-valued field appeared on two pages"
        );
        first
    }
    let mut parts = parts;
    let lines = parts
        .iter_mut()
        .flat_map(|p| std::mem::take(&mut p.lines))
        .collect();
    Extraction {
        document: parts[0].document.clone(),
        extractor: parts[0].extractor.clone(),
        currency: parts[0].currency,
        invoice_number: one(parts.iter().map(|p| p.invoice_number.clone()).collect()),
        invoice_date: one(parts.iter().map(|p| p.invoice_date.clone()).collect()),
        supplier: one(parts.iter().map(|p| p.supplier.clone()).collect()),
        recipient: one(parts.iter().map(|p| p.recipient.clone()).collect()),
        lines,
        stated_totals: one(parts.iter().map(|p| p.stated_totals.clone()).collect()),
        due: one(parts.iter().map(|p| p.due.clone()).collect()),
        credit_note: one(parts.iter().map(|p| p.credit_note.clone()).collect()),
    }
}

#[test]
fn every_synthetic_case_survives_the_trip_through_the_reading_shape() {
    let cases = expected_extractions();
    assert!(cases.len() >= 15, "the sample set has {} cases", cases.len());
    let mut multi_page = 0;
    for (case, expected) in cases {
        let pages = last_page(&expected);
        multi_page += usize::from(pages > 1);
        let mut parts = Vec::new();
        for page in 1..=pages {
            let reading = reading_of_page(&expected, page);
            // Through JSON, as it arrives from the model.
            let json = serde_json::to_string(&reading).unwrap();
            let reading: PageReading = serde_json::from_str(&json).unwrap();
            let context = ConversionContext {
                document: DocumentId::new(expected.document.as_str()),
                page: NonZeroU32::new(page).unwrap(),
                extractor: expected.extractor.clone(),
                confidence: Confidence::CERTAIN,
                currency_known: (page > 1).then_some(expected.currency),
            };
            let conversion = reading.into_extraction(&context).unwrap();
            assert_eq!(conversion.problems, vec![], "{case} page {page}");
            parts.push(conversion.extraction);
        }
        assert_eq!(merge(parts), expected, "{case}");
    }
    assert!(multi_page >= 2, "the sample set should include multi-page cases");
}
