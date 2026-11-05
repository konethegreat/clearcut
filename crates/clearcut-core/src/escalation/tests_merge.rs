// Written by Kone Tshivhinda & Claude
//! Merging the parts of one page: lines in order, facts once, disagreement recorded.

use super::merge::{merge_parts, MergeError};
use crate::reading::{
    PageReading, ReadCarried, ReadCreditNote, ReadCurrency, ReadDue, ReadLine, ReadParty, ReadTax,
    ReadTotals, ReadVatBasis,
};

fn line(description: &str, total: &str) -> ReadLine {
    ReadLine {
        description: description.into(),
        quantity: "1".into(),
        unit_price: total.into(),
        line_total: total.into(),
        tax: ReadTax::Standard,
    }
}

fn page(lines: Vec<ReadLine>) -> PageReading {
    PageReading {
        currency: ReadCurrency::Zar,
        amounts_include_vat: ReadVatBasis::No,
        invoice_number: String::new(),
        invoice_date: String::new(),
        supplier: ReadParty::default(),
        recipient: ReadParty::default(),
        lines,
        totals: ReadTotals::default(),
        carried: ReadCarried::default(),
        due: ReadDue::default(),
        credit_note: ReadCreditNote::default(),
    }
}

#[test]
fn no_parts_is_an_error() {
    assert_eq!(merge_parts(&[]), Err(MergeError::NoParts));
}

#[test]
fn lines_are_joined_in_part_order() {
    let merged = merge_parts(&[
        page(vec![line("a", "1.00"), line("b", "2.00")]),
        page(vec![line("c", "3.00")]),
    ])
    .unwrap();
    let descriptions: Vec<_> = merged
        .reading
        .lines
        .iter()
        .map(|line| line.description.as_str())
        .collect();
    assert_eq!(descriptions, vec!["a", "b", "c"]);
    assert!(merged.conflicts.is_empty());
}

#[test]
fn a_fact_is_taken_from_the_first_part_that_shows_it() {
    let mut first = page(vec![]);
    first.invoice_number = "INV-1".into();
    let mut second = page(vec![]);
    second.invoice_number = "INV-1".into();
    let merged = merge_parts(&[first, second]).unwrap();
    assert_eq!(merged.reading.invoice_number, "INV-1");
    assert!(merged.conflicts.is_empty());
}

#[test]
fn a_later_different_fact_is_a_conflict_and_the_first_one_stays() {
    let mut first = page(vec![]);
    first.invoice_number = "INV-1".into();
    first.supplier.name = "Acme".into();
    first.totals.gross = "10.00".into();
    let mut second = page(vec![]);
    second.invoice_number = "INV-2".into();
    second.supplier.name = "Other".into();
    second.totals.gross = "11.00".into();
    let merged = merge_parts(&[first, second]).unwrap();
    assert_eq!(merged.reading.invoice_number, "INV-1");
    assert_eq!(merged.reading.supplier.name, "Acme");
    assert_eq!(merged.reading.totals.gross, "10.00");
    assert_eq!(
        merged.conflicts,
        vec!["invoice_number", "supplier/name", "totals/gross"]
    );
}

#[test]
fn an_empty_value_and_not_shown_are_not_disagreements() {
    let mut first = page(vec![]);
    first.amounts_include_vat = ReadVatBasis::NotShown;
    first.due.terms = "30 days".into();
    let mut second = page(vec![]);
    second.amounts_include_vat = ReadVatBasis::No;
    second.due.terms = String::new();
    let merged = merge_parts(&[first, second]).unwrap();
    assert_eq!(merged.reading.amounts_include_vat, ReadVatBasis::No);
    assert_eq!(merged.reading.due.terms, "30 days");
    assert!(merged.conflicts.is_empty());
}

#[test]
fn each_total_is_merged_on_its_own() {
    let mut first = page(vec![]);
    first.totals.net = "10.00".into();
    let mut second = page(vec![]);
    second.totals.gross = "11.50".into();
    let merged = merge_parts(&[first, second]).unwrap();
    assert_eq!(merged.reading.totals.net, "10.00");
    assert_eq!(merged.reading.totals.gross, "11.50");
    assert!(merged.conflicts.is_empty());
}

#[test]
fn a_conflict_is_named_once() {
    let mut first = page(vec![]);
    first.recipient.vat_number = "1".into();
    let mut second = page(vec![]);
    second.recipient.vat_number = "2".into();
    second.invoice_date = "2026-01-01".into();
    let mut third = page(vec![]);
    third.recipient.vat_number = "3".into();
    third.invoice_date = "2026-01-02".into();
    let merged = merge_parts(&[first, second, third]).unwrap();
    assert_eq!(merged.conflicts, vec!["recipient/vat_number", "invoice_date"]);
}
