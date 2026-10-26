// Written by Kone Tshivhinda & Claude
//! Deterministic checks on an [`Extraction`] (research 05 §3). The extractor is
//! never trusted: ClearCut recomputes every figure it can and reports each
//! disagreement. A finding lowers confidence and routes the document to
//! review. **Nothing is silently corrected**; the stated value and the
//! recomputed value are both kept in the finding.
//!
//! Totals are compared exactly, to the cent. Some real invoices differ by a
//! cent because VAT is rounded per line rather than per tax category (as
//! ClearCut does it); those will be reported. That is the safe direction, and
//! the gold set will show whether a tolerance is worth allowing.

use super::Extraction;
use crate::compliance_za::{self, InvoiceHeader, Issue};
use crate::event::TotalField;
use crate::{Currency, Invoice, InvoiceTotals, LineItem, Money};
use serde::Serialize;

/// One disagreement or gap found in an extraction. Line numbers are 1-based.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Finding {
    /// No line items were read.
    NoLines,
    /// A line's amounts are not in the document's currency.
    CurrencyMismatch { line: u32 },
    /// A line's quantity is zero or negative.
    NonPositiveQuantity { line: u32 },
    /// A line's amounts are too large to calculate with.
    AmountOutOfRange { line: u32 },
    /// Quantity x unit price is not the printed line total.
    LineTotalMismatch {
        line: u32,
        stated: Money,
        recomputed: Money,
    },
    /// A printed total differs from the total recomputed from the lines.
    TotalsMismatch {
        field: TotalField,
        stated: Money,
        recomputed: Money,
    },
    /// The lines are individually fine but their sum is too large to calculate.
    TotalsOutOfRange,
    /// The due date is earlier than the invoice date.
    DueBeforeIssue,
    /// A South African tax-invoice field rule failed (`compliance_za`).
    Compliance(Issue),
}

impl Finding {
    /// Whether this finding stops a document from being called done. A wrong sum, an impossible
    /// quantity or no lines at all are the extraction being wrong or the document being unusable;
    /// a due date before the invoice date and a South African tax-invoice rule the document
    /// breaks are facts about the document, shown to the reader, not reasons to refuse it.
    pub fn is_blocking(&self) -> bool {
        match self {
            Finding::NoLines
            | Finding::CurrencyMismatch { .. }
            | Finding::NonPositiveQuantity { .. }
            | Finding::AmountOutOfRange { .. }
            | Finding::LineTotalMismatch { .. }
            | Finding::TotalsMismatch { .. }
            | Finding::TotalsOutOfRange => true,
            Finding::DueBeforeIssue | Finding::Compliance(_) => false,
        }
    }
}

/// The result of [`validate`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Validation {
    /// Totals rebuilt from quantity x unit price. `None` when the lines could
    /// not be trusted enough to total them.
    pub recomputed: Option<InvoiceTotals>,
    /// Everything that was wrong, in a fixed order: line problems in line
    /// order, then totals (net, VAT, gross), then dates, then SA field rules.
    pub findings: Vec<Finding>,
}

impl Validation {
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty()
    }
}

/// Checks `extraction` and reports every problem. Never fails and never edits
/// the extraction.
pub fn validate(extraction: &Extraction) -> Validation {
    let mut findings = Vec::new();

    if extraction.lines.is_empty() {
        findings.push(Finding::NoLines);
    }

    // Line by line: only lines that pass the basic checks are totalled.
    let mut items = Vec::with_capacity(extraction.lines.len());
    let mut lines_trusted = !extraction.lines.is_empty();
    for (index, sourced) in extraction.lines.iter().enumerate() {
        let line = line_number(index);
        let l = &sourced.value;
        if l.unit_price.currency != extraction.currency || l.line_total.currency != extraction.currency {
            findings.push(Finding::CurrencyMismatch { line });
            lines_trusted = false;
            continue;
        }
        if l.quantity_milli <= 0 {
            findings.push(Finding::NonPositiveQuantity { line });
            lines_trusted = false;
            continue;
        }
        match l.unit_price.mul_milli(l.quantity_milli) {
            Err(_) => {
                findings.push(Finding::AmountOutOfRange { line });
                lines_trusted = false;
            }
            Ok(recomputed) => {
                if recomputed != l.line_total {
                    findings.push(Finding::LineTotalMismatch {
                        line,
                        stated: l.line_total,
                        recomputed,
                    });
                }
                items.push(LineItem {
                    description: l.description.clone(),
                    quantity_milli: l.quantity_milli,
                    unit_price: l.unit_price,
                    tax: l.tax,
                });
            }
        }
    }

    // Totals are rebuilt from quantity x price (not from the printed line
    // totals) and compared with the printed totals.
    let recomputed = if lines_trusted {
        match Invoice::new("", extraction.currency, items).totals() {
            Ok(totals) => Some(totals),
            Err(_) => {
                findings.push(Finding::TotalsOutOfRange);
                None
            }
        }
    } else {
        None
    };
    if let (Some(stated), Some(recomputed)) = (&extraction.stated_totals, recomputed) {
        let stated = stated.value;
        for (field, printed, rebuilt) in [
            (TotalField::Net, stated.net, recomputed.net),
            (TotalField::Vat, stated.vat, recomputed.vat),
            (TotalField::Gross, stated.gross, recomputed.gross),
        ] {
            if printed != rebuilt {
                findings.push(Finding::TotalsMismatch {
                    field,
                    stated: printed,
                    recomputed: rebuilt,
                });
            }
        }
    }

    if let (Some(due), Some(issued)) = (&extraction.due, &extraction.invoice_date) {
        if due.value.due < issued.value {
            findings.push(Finding::DueBeforeIssue);
        }
    }

    // SA field rules depend on the VAT-inclusive amount: the printed gross if
    // there is one, otherwise the recomputed one. The thresholds are in rand,
    // so they are not applied to other currencies, and if no amount is known
    // the rules cannot be judged at all.
    let gross = extraction
        .stated_totals
        .as_ref()
        .map(|t| t.value.gross)
        .or(recomputed.map(|t| t.gross));
    if let Some(gross) = gross {
        if gross.currency == Currency::Zar {
            let header = header_of(extraction);
            findings.extend(
                compliance_za::validate(&header, gross)
                    .into_iter()
                    .map(Finding::Compliance),
            );
        }
    }

    Validation { recomputed, findings }
}

/// 1-based line number for a 0-based index. Saturates instead of panicking.
fn line_number(index: usize) -> u32 {
    u32::try_from(index + 1).unwrap_or(u32::MAX)
}

/// What `compliance_za` needs to know about the document's fields.
fn header_of(extraction: &Extraction) -> InvoiceHeader {
    let supplier = extraction.supplier.as_ref().map(|s| &s.value);
    let recipient = extraction.recipient.as_ref().map(|r| &r.value);
    InvoiceHeader {
        supplier_name: supplier.map(|p| p.name.clone()),
        supplier_address: supplier.and_then(|p| p.address.clone()),
        supplier_vat_number: supplier.and_then(|p| p.vat_number.clone()),
        recipient_name: recipient.map(|p| p.name.clone()),
        recipient_address: recipient.and_then(|p| p.address.clone()),
        recipient_vat_number: recipient.and_then(|p| p.vat_number.clone()),
        serial_number: extraction.invoice_number.as_ref().map(|n| n.value.clone()),
        has_date_of_issue: extraction.invoice_date.is_some(),
        has_description: extraction
            .lines
            .iter()
            .any(|l| !l.value.description.trim().is_empty()),
    }
}
