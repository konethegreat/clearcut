// Written by Kone Tshivhinda & Claude
//! Joining the parts of one page (long-invoice completeness, B.1).
//!
//! Lines are concatenated in part order. Every other fact is taken from the first part that
//! shows it; a later part that shows a different value is a conflict, never a repair. Nothing
//! here reads text or guesses: the caller converts, validates and assesses the merged reading.

use crate::reading::{PageReading, ReadParty, ReadTotals};

/// Why no merge could be made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MergeError {
    #[error("there are no parts to merge")]
    NoParts,
}

/// The merged page, and the facts the parts disagreed on (fixed paths, in first-seen order).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Merged {
    pub reading: PageReading,
    pub conflicts: Vec<&'static str>,
}

fn take<T: Clone + Default + PartialEq>(
    slot: &mut T,
    later: &T,
    path: &'static str,
    conflicts: &mut Vec<&'static str>,
) {
    if slot == &T::default() {
        *slot = later.clone();
    } else if slot != later && later != &T::default() && !conflicts.contains(&path) {
        conflicts.push(path);
    }
}

fn take_text(slot: &mut String, later: &str, path: &'static str, conflicts: &mut Vec<&'static str>) {
    if slot.is_empty() {
        slot.push_str(later);
    } else if slot != later && !later.is_empty() && !conflicts.contains(&path) {
        conflicts.push(path);
    }
}

/// Merges the parts of one page, in the order they were read.
pub fn merge_parts(readings: &[PageReading]) -> Result<Merged, MergeError> {
    let Some((first, rest)) = readings.split_first() else {
        return Err(MergeError::NoParts);
    };
    let mut merged = first.clone();
    let mut conflicts = Vec::new();
    for part in rest {
        merged.lines.extend(part.lines.iter().cloned());
        take_text(
            &mut merged.invoice_number,
            &part.invoice_number,
            "invoice_number",
            &mut conflicts,
        );
        take_text(
            &mut merged.invoice_date,
            &part.invoice_date,
            "invoice_date",
            &mut conflicts,
        );
        take(&mut merged.currency, &part.currency, "currency", &mut conflicts);
        take(
            &mut merged.amounts_include_vat,
            &part.amounts_include_vat,
            "amounts_include_vat",
            &mut conflicts,
        );
        take_party(&mut merged.supplier, &part.supplier, "supplier", &mut conflicts);
        take_party(
            &mut merged.recipient,
            &part.recipient,
            "recipient",
            &mut conflicts,
        );
        take_totals(&mut merged.totals, &part.totals, &mut conflicts);
        take_text(
            &mut merged.carried.brought_forward,
            &part.carried.brought_forward,
            "carried/brought_forward",
            &mut conflicts,
        );
        take_text(
            &mut merged.carried.carried_forward,
            &part.carried.carried_forward,
            "carried/carried_forward",
            &mut conflicts,
        );
        take_text(&mut merged.due.date, &part.due.date, "due/date", &mut conflicts);
        take_text(
            &mut merged.due.terms,
            &part.due.terms,
            "due/terms",
            &mut conflicts,
        );
        take_text(
            &mut merged.credit_note.original_invoice_number,
            &part.credit_note.original_invoice_number,
            "credit_note/original_invoice_number",
            &mut conflicts,
        );
        take_text(
            &mut merged.credit_note.amount,
            &part.credit_note.amount,
            "credit_note/amount",
            &mut conflicts,
        );
    }
    Ok(Merged {
        reading: merged,
        conflicts,
    })
}

fn take_party(
    slot: &mut ReadParty,
    later: &ReadParty,
    prefix: &'static str,
    conflicts: &mut Vec<&'static str>,
) {
    let path = |field: &'static str| -> &'static str {
        match (prefix, field) {
            ("supplier", "name") => "supplier/name",
            ("supplier", "address") => "supplier/address",
            _ if prefix == "supplier" => "supplier/vat_number",
            ("recipient", "name") => "recipient/name",
            ("recipient", "address") => "recipient/address",
            _ => "recipient/vat_number",
        }
    };
    take_text(&mut slot.name, &later.name, path("name"), conflicts);
    take_text(&mut slot.address, &later.address, path("address"), conflicts);
    take_text(
        &mut slot.vat_number,
        &later.vat_number,
        path("vat_number"),
        conflicts,
    );
}

fn take_totals(slot: &mut ReadTotals, later: &ReadTotals, conflicts: &mut Vec<&'static str>) {
    take_text(&mut slot.net, &later.net, "totals/net", conflicts);
    take_text(&mut slot.vat, &later.vat, "totals/vat", conflicts);
    take_text(&mut slot.gross, &later.gross, "totals/gross", conflicts);
}
