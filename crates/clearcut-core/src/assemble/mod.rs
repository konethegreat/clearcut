// Written by Kone Tshivhinda & Claude
//! Putting the pages of one document together (the first half of P2-07).
//!
//! Each page of a document is read on its own and gives an [`Extraction`] of what is printed
//! **on that page**. The document is the pages put together:
//!
//! - **lines** are the pages' lines in page order;
//! - **everything that is printed once per document** (the invoice number, the date, the
//!   supplier and the recipient, the due date, the credit note's reference, the printed totals)
//!   is taken from the first page that has it. A page that repeats it (a running header) agrees
//!   and changes nothing; a page that **disagrees** is a [`Conflict`]: the first value is kept,
//!   the conflict is reported by its fixed name, and the caller sends the document to a person.
//!   Two invoice numbers in one file, or totals that differ page by page (a balance carried
//!   forward), are exactly the cases a person must look at, so nothing is merged quietly.
//!
//! Nothing here believes anything: the result is still an untrusted [`Extraction`] that
//! [`validate`](crate::extraction::validate) checks as a whole, which is where "the lines add up to
//! the printed total" is finally judged (a page cannot judge it, ADR-0018 section 3).
//! Carried balances, statement totals and page-spanning tables are the second half of P2-07.

use crate::extraction::Extraction;
use crate::{Error, Result};

/// A value printed once per document that two pages print differently. Fixed names only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Conflict {
    InvoiceNumber,
    InvoiceDate,
    Supplier,
    Recipient,
    Totals,
    DueDate,
    CreditNote,
}

impl Conflict {
    pub fn as_str(self) -> &'static str {
        match self {
            Conflict::InvoiceNumber => "invoice_number",
            Conflict::InvoiceDate => "invoice_date",
            Conflict::Supplier => "supplier",
            Conflict::Recipient => "recipient",
            Conflict::Totals => "totals",
            Conflict::DueDate => "due_date",
            Conflict::CreditNote => "credit_note",
        }
    }
}

/// The pages put together, and where they disagreed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assembled {
    pub extraction: Extraction,
    /// In a fixed order, each at most once.
    pub conflicts: Vec<Conflict>,
}

/// Takes the first page's value, and reports whether any later page has a different one.
fn first_of<T: Clone, K: PartialEq>(
    pages: &[Extraction],
    field: impl Fn(&Extraction) -> &Option<T>,
    same: impl Fn(&T) -> K,
) -> (Option<T>, bool) {
    let mut chosen: Option<T> = None;
    let mut disagree = false;
    for page in pages {
        let Some(value) = field(page).as_ref() else {
            continue;
        };
        match &chosen {
            None => chosen = Some(value.clone()),
            Some(first) => disagree |= same(first) != same(value),
        }
    }
    (chosen, disagree)
}

/// Puts the extractions of a document's pages, in page order, together.
///
/// Fails if there are none, if they are not of one document, or if they are not all in one
/// currency (each page was read with the currency already established, so that is a bug).
pub fn assemble(pages: &[Extraction]) -> Result<Assembled> {
    let Some(first) = pages.first() else {
        return Err(Error::InvalidExtraction("a document has at least one page"));
    };
    if pages.iter().any(|page| page.document != first.document) {
        return Err(Error::InvalidExtraction("the pages are not of one document"));
    }
    if let Some(other) = pages.iter().find(|page| page.currency != first.currency) {
        return Err(Error::CurrencyMismatch {
            expected: first.currency.code(),
            got: other.currency.code(),
        });
    }

    let mut conflicts = Vec::new();
    let mut note = |disagree: bool, conflict: Conflict| {
        if disagree {
            conflicts.push(conflict);
        }
    };
    let (invoice_number, d) = first_of(pages, |p| &p.invoice_number, |v| v.value.clone());
    note(d, Conflict::InvoiceNumber);
    let (invoice_date, d) = first_of(pages, |p| &p.invoice_date, |v| v.value);
    note(d, Conflict::InvoiceDate);
    // A party is the same party if its name and VAT number agree; an address may be printed
    // differently (or only on one page).
    let (supplier, d) = first_of(
        pages,
        |p| &p.supplier,
        |v| (v.value.name.clone(), v.value.vat_number.clone()),
    );
    note(d, Conflict::Supplier);
    let (recipient, d) = first_of(
        pages,
        |p| &p.recipient,
        |v| (v.value.name.clone(), v.value.vat_number.clone()),
    );
    note(d, Conflict::Recipient);
    let (stated_totals, d) = first_of(pages, |p| &p.stated_totals, |v| v.value);
    note(d, Conflict::Totals);
    let (due, d) = first_of(pages, |p| &p.due, |v| v.value.due);
    note(d, Conflict::DueDate);
    let (credit_note, d) = first_of(pages, |p| &p.credit_note, |v| v.value.clone());
    note(d, Conflict::CreditNote);

    Ok(Assembled {
        extraction: Extraction {
            document: first.document.clone(),
            extractor: first.extractor.clone(),
            currency: first.currency,
            invoice_number,
            invoice_date,
            supplier,
            recipient,
            lines: pages.iter().flat_map(|page| page.lines.iter().cloned()).collect(),
            stated_totals,
            carried: pages
                .iter()
                .flat_map(|page| page.carried.iter().cloned())
                .collect(),
            due,
            credit_note,
        },
        conflicts,
    })
}

#[cfg(test)]
mod tests;
