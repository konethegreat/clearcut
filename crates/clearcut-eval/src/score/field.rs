// Written by Kone Tshivhinda & Claude
//! The fields that are scored, which of them are critical, and how they are
//! named in reports (ADR-0010).
//!
//! **Critical** fields are the ones where a silent mistake does real harm:
//! invoice number and date, currency, the supplier's name and VAT number, the
//! three stated totals, each line's quantity, unit price and total, and a
//! credit note's amount. Everything else (descriptions, addresses, terms,
//! recipient details, tax category, a due date) is non-critical.
//!
//! Lines are numbered from 0 in report names (`lines[0].quantity`), as in label
//! problems. The validator's own finding keys count lines from 1.

use clearcut_core::event::TotalField;
use serde::{Serialize, Serializer};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PartyPart {
    Name,
    VatNumber,
    Address,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LinePart {
    Quantity,
    UnitPrice,
    LineTotal,
    Description,
    Tax,
}

/// Which stated total. (Core's `TotalField` has no ordering, which reports need.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TotalPart {
    Net,
    Vat,
    Gross,
}

impl From<TotalField> for TotalPart {
    fn from(field: TotalField) -> Self {
        match field {
            TotalField::Net => TotalPart::Net,
            TotalField::Vat => TotalPart::Vat,
            TotalField::Gross => TotalPart::Gross,
        }
    }
}

/// One scored field of one document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Field {
    Currency,
    InvoiceNumber,
    InvoiceDate,
    Supplier(PartyPart),
    Recipient(PartyPart),
    /// A line (0-based) and which part of it.
    Line(usize, LinePart),
    Total(TotalPart),
    DueDate,
    DueTerms,
    CreditNoteOriginal,
    CreditNoteAmount,
}

impl Field {
    pub fn is_critical(self) -> bool {
        match self {
            Field::Currency
            | Field::InvoiceNumber
            | Field::InvoiceDate
            | Field::Total(_)
            | Field::CreditNoteAmount => true,
            Field::Supplier(part) => matches!(part, PartyPart::Name | PartyPart::VatNumber),
            Field::Line(_, part) => matches!(
                part,
                LinePart::Quantity | LinePart::UnitPrice | LinePart::LineTotal
            ),
            Field::Recipient(_) | Field::DueDate | Field::DueTerms | Field::CreditNoteOriginal => false,
        }
    }

    /// The line amounts a totals problem could be hiding in.
    pub fn is_line_amount(self) -> bool {
        matches!(
            self,
            Field::Line(_, LinePart::Quantity | LinePart::UnitPrice | LinePart::LineTotal)
        )
    }
}

impl fmt::Display for PartyPart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PartyPart::Name => "name",
            PartyPart::VatNumber => "vat_number",
            PartyPart::Address => "address",
        })
    }
}

impl fmt::Display for LinePart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            LinePart::Quantity => "quantity",
            LinePart::UnitPrice => "unit_price",
            LinePart::LineTotal => "line_total",
            LinePart::Description => "description",
            LinePart::Tax => "tax",
        })
    }
}

impl fmt::Display for Field {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Field::Currency => f.write_str("currency"),
            Field::InvoiceNumber => f.write_str("invoice_number"),
            Field::InvoiceDate => f.write_str("invoice_date"),
            Field::Supplier(part) => write!(f, "supplier.{part}"),
            Field::Recipient(part) => write!(f, "recipient.{part}"),
            Field::Line(index, part) => write!(f, "lines[{index}].{part}"),
            Field::Total(TotalPart::Net) => f.write_str("totals.net"),
            Field::Total(TotalPart::Vat) => f.write_str("totals.vat"),
            Field::Total(TotalPart::Gross) => f.write_str("totals.gross"),
            Field::DueDate => f.write_str("due.date"),
            Field::DueTerms => f.write_str("due.terms"),
            Field::CreditNoteOriginal => f.write_str("credit_note.original_invoice_number"),
            Field::CreditNoteAmount => f.write_str("credit_note.amount"),
        }
    }
}

impl Serialize for Field {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exactly_the_adr_listed_fields_are_critical() {
        let critical = [
            Field::Currency,
            Field::InvoiceNumber,
            Field::InvoiceDate,
            Field::Supplier(PartyPart::Name),
            Field::Supplier(PartyPart::VatNumber),
            Field::Total(TotalPart::Net),
            Field::Total(TotalPart::Vat),
            Field::Total(TotalPart::Gross),
            Field::Line(3, LinePart::Quantity),
            Field::Line(0, LinePart::UnitPrice),
            Field::Line(7, LinePart::LineTotal),
            Field::CreditNoteAmount,
        ];
        let non_critical = [
            Field::Supplier(PartyPart::Address),
            Field::Recipient(PartyPart::Name),
            Field::Recipient(PartyPart::VatNumber),
            Field::Recipient(PartyPart::Address),
            Field::Line(0, LinePart::Description),
            Field::Line(0, LinePart::Tax),
            Field::DueDate,
            Field::DueTerms,
            Field::CreditNoteOriginal,
        ];
        for field in critical {
            assert!(field.is_critical(), "{field}");
        }
        for field in non_critical {
            assert!(!field.is_critical(), "{field}");
        }
    }

    #[test]
    fn fields_are_named_by_position_in_the_document() {
        let names = [
            (Field::Currency, "currency"),
            (Field::InvoiceNumber, "invoice_number"),
            (Field::InvoiceDate, "invoice_date"),
            (Field::Supplier(PartyPart::Name), "supplier.name"),
            (Field::Supplier(PartyPart::VatNumber), "supplier.vat_number"),
            (Field::Supplier(PartyPart::Address), "supplier.address"),
            (Field::Recipient(PartyPart::Name), "recipient.name"),
            (Field::Line(2, LinePart::Quantity), "lines[2].quantity"),
            (Field::Line(0, LinePart::UnitPrice), "lines[0].unit_price"),
            (Field::Line(10, LinePart::LineTotal), "lines[10].line_total"),
            (Field::Line(1, LinePart::Description), "lines[1].description"),
            (Field::Line(1, LinePart::Tax), "lines[1].tax"),
            (Field::Total(TotalPart::Net), "totals.net"),
            (Field::Total(TotalPart::Vat), "totals.vat"),
            (Field::Total(TotalPart::Gross), "totals.gross"),
            (Field::DueDate, "due.date"),
            (Field::DueTerms, "due.terms"),
            (Field::CreditNoteOriginal, "credit_note.original_invoice_number"),
            (Field::CreditNoteAmount, "credit_note.amount"),
        ];
        for (field, name) in names {
            assert_eq!(field.to_string(), name);
            assert_eq!(serde_json::to_string(&field).unwrap(), format!("\"{name}\""));
        }
    }

    #[test]
    fn line_amounts_are_the_three_critical_line_parts() {
        assert!(Field::Line(0, LinePart::Quantity).is_line_amount());
        assert!(Field::Line(0, LinePart::UnitPrice).is_line_amount());
        assert!(Field::Line(0, LinePart::LineTotal).is_line_amount());
        assert!(!Field::Line(0, LinePart::Description).is_line_amount());
        assert!(!Field::Line(0, LinePart::Tax).is_line_amount());
        assert!(!Field::Total(TotalPart::Net).is_line_amount());
    }

    #[test]
    fn fields_sort_in_document_order() {
        let mut fields = vec![
            Field::Total(TotalPart::Gross),
            Field::Line(1, LinePart::Quantity),
            Field::Line(0, LinePart::LineTotal),
            Field::InvoiceNumber,
            Field::Currency,
        ];
        fields.sort();
        assert_eq!(
            fields,
            [
                Field::Currency,
                Field::InvoiceNumber,
                Field::Line(0, LinePart::LineTotal),
                Field::Line(1, LinePart::Quantity),
                Field::Total(TotalPart::Gross),
            ]
        );
    }
}
