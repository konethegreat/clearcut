// Written by Kone Tshivhinda & Claude
use super::*;
use crate::extraction::fixtures::{at, clean, line, party, zar};
use crate::extraction::Extraction;
use crate::{Currency, InvoiceTotals, TaxCategory};
use std::num::NonZeroU32;
use time::macros::date as d;

/// Moves every value of `extraction` to `page`.
fn on_page(mut extraction: Extraction, page: u32) -> Extraction {
    let page = NonZeroU32::new(page).unwrap();
    macro_rules! move_to {
        ($($field:ident),*) => {$(
            if let Some(v) = extraction.$field.as_mut() { v.page = page; }
        )*};
    }
    move_to!(
        invoice_number,
        invoice_date,
        supplier,
        recipient,
        stated_totals,
        due,
        credit_note
    );
    for l in &mut extraction.lines {
        l.page = page;
    }
    extraction
}

/// A page that prints only lines.
fn lines_only(descriptions: &[&str], page: u32) -> Extraction {
    let mut x = clean();
    x.invoice_number = None;
    x.invoice_date = None;
    x.supplier = None;
    x.recipient = None;
    x.stated_totals = None;
    x.due = None;
    x.lines = descriptions
        .iter()
        .map(|d| line(d, 1_000, 100_000, 100_000, TaxCategory::Standard))
        .collect();
    on_page(x, page)
}

#[test]
fn pages_are_put_together_lines_in_page_order_and_the_rest_from_the_first_page_that_has_it() {
    let mut first = lines_only(&["A", "B"], 1);
    first.invoice_number = Some(at("INV-1".to_string()));
    first.supplier = Some(party("Acme (Pty) Ltd", "4123456789"));
    let second = lines_only(&["C"], 2);
    let mut third = lines_only(&[], 3);
    third.stated_totals = Some(at(InvoiceTotals {
        net: zar(300_000),
        vat: zar(45_000),
        gross: zar(345_000),
    }));
    let third = on_page(third, 3);

    let assembled = assemble(&[first, second, third]).unwrap();
    assert!(assembled.conflicts.is_empty());
    let x = assembled.extraction;
    let lines: Vec<(&str, u32)> = x
        .lines
        .iter()
        .map(|l| (l.value.description.as_str(), l.page.get()))
        .collect();
    assert_eq!(lines, [("A", 1), ("B", 1), ("C", 2)]);
    assert_eq!(x.invoice_number.unwrap().page.get(), 1);
    assert_eq!(x.supplier.unwrap().value.name, "Acme (Pty) Ltd");
    assert_eq!(
        x.stated_totals.unwrap().page.get(),
        3,
        "provenance keeps the page the totals are on"
    );
}

#[test]
fn a_running_header_that_agrees_is_not_a_conflict() {
    let first = clean();
    let second = on_page(clean(), 2);
    let assembled = assemble(&[first, second]).unwrap();
    assert!(assembled.conflicts.is_empty(), "{:?}", assembled.conflicts);
    assert_eq!(assembled.extraction.lines.len(), 4);
    assert_eq!(assembled.extraction.invoice_number.unwrap().page.get(), 1);
}

#[test]
fn pages_that_disagree_keep_the_first_value_and_name_each_conflict_once() {
    let first = clean();
    let mut second = on_page(clean(), 2);
    second.invoice_number = Some(at("INV-000002".to_string()));
    second.invoice_date = Some(at(d!(2026 - 04 - 01)));
    second.supplier = Some(party("Other Co", "4123456789"));
    second.stated_totals = Some(at(InvoiceTotals {
        net: zar(1),
        vat: zar(1),
        gross: zar(2),
    }));
    let mut third = on_page(clean(), 3);
    third.invoice_number = Some(at("INV-000003".to_string()));

    let assembled = assemble(&[first.clone(), second, third]).unwrap();
    assert_eq!(
        assembled.conflicts,
        [
            Conflict::InvoiceNumber,
            Conflict::InvoiceDate,
            Conflict::Supplier,
            Conflict::Totals
        ]
    );
    assert_eq!(assembled.extraction.invoice_number, first.invoice_number);
    assert_eq!(assembled.extraction.stated_totals, first.stated_totals);
}

#[test]
fn an_address_printed_differently_is_the_same_party() {
    let first = clean();
    let mut second = on_page(clean(), 2);
    second.supplier.as_mut().unwrap().value.address = Some("PO Box 5".into());
    assert!(assemble(&[first, second]).unwrap().conflicts.is_empty());
}

#[test]
fn nothing_to_put_together_other_documents_and_other_currencies_are_refused() {
    assert!(assemble(&[]).is_err());
    let mut other_document = clean();
    other_document.document = crate::event::DocumentId::new("doc_2");
    assert!(assemble(&[clean(), other_document]).is_err());
    let mut dollars = clean();
    dollars.currency = Currency::Usd;
    assert!(matches!(
        assemble(&[clean(), dollars]),
        Err(Error::CurrencyMismatch {
            expected: "ZAR",
            got: "USD"
        })
    ));
}

#[test]
fn the_names_of_the_conflicts_are_fixed() {
    let names: Vec<&str> = [
        Conflict::InvoiceNumber,
        Conflict::InvoiceDate,
        Conflict::Supplier,
        Conflict::Recipient,
        Conflict::Totals,
        Conflict::DueDate,
        Conflict::CreditNote,
    ]
    .iter()
    .map(|c| c.as_str())
    .collect();
    assert_eq!(
        names,
        [
            "invoice_number",
            "invoice_date",
            "supplier",
            "recipient",
            "totals",
            "due_date",
            "credit_note"
        ]
    );
}
