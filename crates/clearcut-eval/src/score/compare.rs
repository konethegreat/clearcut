// Written by Kone Tshivhinda & Claude
//! Comparing what a run extracted with the truth, field by field.
//!
//! Both sides are core [`Extraction`]s: the truth is converted from the label
//! (a person's reading of the document), the other is what the system read.
//! Each field of the truth, and each field the system invented, becomes one
//! [`Compared`]. This module only says **what differs**; whether a difference
//! was *flagged* is decided in [`super`], where the disposition, findings and
//! confidence are known.
//!
//! How values are compared:
//! - names, addresses, descriptions and payment terms: whitespace collapsed
//!   and case ignored (`"ACME  Ltd"` is `"Acme Ltd"`);
//! - ids (invoice number, VAT numbers, a credit note's original invoice
//!   number): surrounding whitespace ignored, otherwise **exact**;
//! - amounts, quantities, dates, currencies and tax categories: exact;
//! - a blank text counts as absent (a model that writes `""` for "not found"
//!   has not found it).
//!
//! Lines are matched by position. If the system drops or invents a line in the
//! middle, every line after it will differ: that over-reports, which is the safe
//! direction for a scorer.

use super::field::{Field, LinePart, PartyPart, TotalPart};
use clearcut_core::event::Confidence;
use clearcut_core::extraction::{Extraction, Sourced};
use std::num::NonZeroU32;

/// How a field of the result relates to the truth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Correct,
    /// Both have a value and they differ.
    Wrong,
    /// The truth has a value; the result has none.
    Missing,
    /// The result has a value; the truth has none.
    Spurious,
}

/// One field of one case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Compared {
    pub field: Field,
    pub verdict: Verdict,
    /// The result's confidence in the value, if it gave one.
    pub confidence: Option<Confidence>,
    /// For a correct field that cites a page: whether it cites the right one.
    pub page_correct: Option<bool>,
}

/// Compares `result` (`None` when nothing was extracted) with `truth`. The
/// fields come back in document order.
pub fn compare(truth: &Extraction, result: Option<&Extraction>) -> Vec<Compared> {
    let mut out = Vec::new();

    out.push(Compared {
        field: Field::Currency,
        verdict: match result {
            None => Verdict::Missing,
            Some(r) if r.currency == truth.currency => Verdict::Correct,
            Some(_) => Verdict::Wrong,
        },
        confidence: None,
        page_correct: None,
    });

    let mut add = |compared: Option<Compared>| out.extend(compared);

    // Scalars with a page.
    add(sourced(
        Field::InvoiceNumber,
        truth.invoice_number.as_ref(),
        result.and_then(|r| r.invoice_number.as_ref()),
        |s| nonblank(&s.value),
        id_eq,
    ));
    add(sourced(
        Field::InvoiceDate,
        truth.invoice_date.as_ref(),
        result.and_then(|r| r.invoice_date.as_ref()),
        |s| Some(&s.value),
        |a, b| a == b,
    ));

    // Parties: each part is its own field, all sharing the party's page and confidence.
    for (make, t, r) in [
        (
            Field::Supplier as fn(PartyPart) -> Field,
            truth.supplier.as_ref(),
            result.and_then(|r| r.supplier.as_ref()),
        ),
        (
            Field::Recipient as fn(PartyPart) -> Field,
            truth.recipient.as_ref(),
            result.and_then(|r| r.recipient.as_ref()),
        ),
    ] {
        add(sourced(
            make(PartyPart::Name),
            t,
            r,
            |s| nonblank(&s.value.name),
            text_eq,
        ));
        add(sourced(
            make(PartyPart::VatNumber),
            t,
            r,
            |s| opt_nonblank(&s.value.vat_number),
            id_eq,
        ));
        add(sourced(
            make(PartyPart::Address),
            t,
            r,
            |s| opt_nonblank(&s.value.address),
            text_eq,
        ));
    }

    // Lines, by position.
    let empty = Vec::new();
    let result_lines = result.map_or(&empty, |r| &r.lines);
    for index in 0..truth.lines.len().max(result_lines.len()) {
        let (t, r) = (truth.lines.get(index), result_lines.get(index));
        add(sourced(
            Field::Line(index, LinePart::Quantity),
            t,
            r,
            |s| Some(&s.value.quantity_milli),
            |a, b| a == b,
        ));
        add(sourced(
            Field::Line(index, LinePart::UnitPrice),
            t,
            r,
            |s| Some(&s.value.unit_price),
            |a, b| a == b,
        ));
        add(sourced(
            Field::Line(index, LinePart::LineTotal),
            t,
            r,
            |s| Some(&s.value.line_total),
            |a, b| a == b,
        ));
        add(sourced(
            Field::Line(index, LinePart::Description),
            t,
            r,
            |s| nonblank(&s.value.description),
            text_eq,
        ));
        add(sourced(
            Field::Line(index, LinePart::Tax),
            t,
            r,
            |s| Some(&s.value.tax),
            |a, b| a == b,
        ));
    }

    // The printed totals.
    let (t, r) = (
        truth.stated_totals.as_ref(),
        result.and_then(|r| r.stated_totals.as_ref()),
    );
    add(sourced(
        Field::Total(TotalPart::Net),
        t,
        r,
        |s| Some(&s.value.net),
        |a, b| a == b,
    ));
    add(sourced(
        Field::Total(TotalPart::Vat),
        t,
        r,
        |s| Some(&s.value.vat),
        |a, b| a == b,
    ));
    add(sourced(
        Field::Total(TotalPart::Gross),
        t,
        r,
        |s| Some(&s.value.gross),
        |a, b| a == b,
    ));

    // Payment terms.
    let (t, r) = (truth.due.as_ref(), result.and_then(|r| r.due.as_ref()));
    add(sourced(
        Field::DueDate,
        t,
        r,
        |s| Some(&s.value.due),
        |a, b| a == b,
    ));
    add(sourced(
        Field::DueTerms,
        t,
        r,
        |s| opt_nonblank(&s.value.terms),
        text_eq,
    ));

    // A credit note's reference.
    let (t, r) = (
        truth.credit_note.as_ref(),
        result.and_then(|r| r.credit_note.as_ref()),
    );
    add(sourced(
        Field::CreditNoteOriginal,
        t,
        r,
        |s| nonblank(&s.value.original_invoice_number),
        id_eq,
    ));
    add(sourced(
        Field::CreditNoteAmount,
        t,
        r,
        |s| Some(&s.value.amount),
        |a, b| a == b,
    ));

    out
}

/// Compares one value of the truth with the same value of the result. `get`
/// pulls the value out of the wrapper (it may be absent inside it, as an
/// optional address is), and `same` decides equality. Returns `None` when
/// neither side has the value, so there is nothing to score.
fn sourced<S, V: ?Sized>(
    field: Field,
    truth: Option<&Sourced<S>>,
    result: Option<&Sourced<S>>,
    get: impl Fn(&Sourced<S>) -> Option<&V>,
    same: impl Fn(&V, &V) -> bool,
) -> Option<Compared> {
    let wanted = truth.and_then(&get);
    let found = result.and_then(&get);
    let (verdict, page_correct) = match (wanted, found) {
        (None, None) => return None,
        (Some(_), None) => (Verdict::Missing, None),
        (None, Some(_)) => (Verdict::Spurious, None),
        (Some(w), Some(f)) if same(w, f) => (
            Verdict::Correct,
            page_of(truth).zip(page_of(result)).map(|(want, got)| want == got),
        ),
        (Some(_), Some(_)) => (Verdict::Wrong, None),
    };
    let confidence = found.and(result).map(|r| r.confidence);
    Some(Compared {
        field,
        verdict,
        confidence,
        page_correct,
    })
}

fn page_of<S>(sourced: Option<&Sourced<S>>) -> Option<NonZeroU32> {
    sourced.map(|s| s.page)
}

fn nonblank(text: &str) -> Option<&str> {
    (!text.trim().is_empty()).then_some(text)
}

fn opt_nonblank(text: &Option<String>) -> Option<&str> {
    text.as_deref().and_then(nonblank)
}

/// Whitespace collapsed, case ignored.
fn text_eq(a: &str, b: &str) -> bool {
    fn norm(text: &str) -> String {
        text.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    }
    norm(a) == norm(b)
}

/// Surrounding whitespace ignored; otherwise exact.
fn id_eq(a: &str, b: &str) -> bool {
    a.trim() == b.trim()
}

#[cfg(test)]
mod tests;
