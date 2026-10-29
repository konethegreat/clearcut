// Written by Kone Tshivhinda & Claude
//! The structured facts an event carries. Each kind of event has its own typed
//! fields; amounts are always `Money` (integer cents), never floats.

use super::{DocumentId, EventType};
use crate::{Money, TaxCategory};
use serde::{Deserialize, Serialize};
use time::Date;

/// Which of the three invoice totals a mismatch is about.
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TotalField {
    Net,
    Vat,
    Gross,
}

/// Why two documents look like the same invoice (research 05 §3).
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DuplicateReason {
    /// Byte-identical content (same hash).
    ContentHash,
    /// Same supplier, invoice number, date and total.
    SupplierNumberDateTotal,
}

/// Which page-continuation figure a [`EventPayload::BalanceStated`] event is about (ADR-0022).
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BalanceRole {
    /// The amount the page brought forward at its head.
    BroughtForward,
    /// The amount the page carried forward at its foot.
    CarriedForward,
}

/// The data of an event. In Rust an `enum` can carry different fields in each
/// variant, so the compiler checks every place that handles an event kind.
///
/// JSON form: `{"type": "totals_mismatch", "field": "vat", ...}`.
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventPayload {
    /// The invoice's own number as printed on the document.
    InvoiceNumberStated { number: String },
    /// A supplier was identified on the document.
    SupplierIdentified {
        name: String,
        vat_number: Option<String>,
    },
    /// The recipient (the party the invoice is addressed to) was identified.
    RecipientIdentified {
        name: String,
        vat_number: Option<String>,
    },
    /// A line item was billed. `quantity_milli` is in thousandths (1000 = 1.000).
    LineItemBilled {
        description: String,
        quantity_milli: i64,
        unit_price: Money,
        line_total: Money,
        tax_category: TaxCategory,
    },
    /// A line's stated total differs from quantity x unit price. `line` is
    /// the 1-based line number; both amounts are kept.
    LineTotalMismatch {
        line: u32,
        stated: Money,
        recomputed: Money,
    },
    /// VAT for one tax category, as stated or calculated.
    VatCalculated {
        category: TaxCategory,
        taxable: Money,
        rate_bps: u32,
        vat: Money,
    },
    /// The totals printed on the document.
    TotalsStated { net: Money, vat: Money, gross: Money },
    /// A stated total differs from what ClearCut recomputed. Both values are
    /// kept; the stated one is never silently "fixed" (research 05 §3).
    TotalsMismatch {
        field: TotalField,
        stated: Money,
        recomputed: Money,
    },
    /// A page-continuation figure printed at the top or foot of a long invoice's page.
    BalanceStated { role: BalanceRole, amount: Money },
    /// This page's carried-forward figure differs from the next page's brought-forward figure.
    BalanceMismatch {
        /// The page whose carried-forward figure was compared.
        after_page: u32,
        carried_forward: Money,
        brought_forward: Money,
    },
    /// A due date or payment terms printed on the document.
    DueDateStated {
        #[cfg_attr(feature = "schemars", schemars(with = "String"))]
        due: Date,
        terms: Option<String>,
    },
    /// A credit note refers back to an earlier invoice.
    CreditNoteReferenced {
        original_invoice_number: String,
        amount: Money,
    },
    /// A compliance check failed (e.g. a required field is missing).
    ComplianceIssueFound { field: String, message: String },
    /// Another document looks like a duplicate of this one.
    PossibleDuplicate {
        matching_document: DocumentId,
        reason: DuplicateReason,
    },
}

impl EventPayload {
    /// The kind of this payload.
    pub fn event_type(&self) -> EventType {
        match self {
            EventPayload::InvoiceNumberStated { .. } => EventType::InvoiceNumberStated,
            EventPayload::SupplierIdentified { .. } => EventType::SupplierIdentified,
            EventPayload::RecipientIdentified { .. } => EventType::RecipientIdentified,
            EventPayload::LineItemBilled { .. } => EventType::LineItemBilled,
            EventPayload::LineTotalMismatch { .. } => EventType::LineTotalMismatch,
            EventPayload::VatCalculated { .. } => EventType::VatCalculated,
            EventPayload::TotalsStated { .. } => EventType::TotalsStated,
            EventPayload::TotalsMismatch { .. } => EventType::TotalsMismatch,
            EventPayload::BalanceStated { .. } => EventType::BalanceStated,
            EventPayload::BalanceMismatch { .. } => EventType::BalanceMismatch,
            EventPayload::DueDateStated { .. } => EventType::DueDateStated,
            EventPayload::CreditNoteReferenced { .. } => EventType::CreditNoteReferenced,
            EventPayload::ComplianceIssueFound { .. } => EventType::ComplianceIssueFound,
            EventPayload::PossibleDuplicate { .. } => EventType::PossibleDuplicate,
        }
    }

    /// Every amount in this payload, in field order. The number guard
    /// (P3-06) uses this to check narrated figures against cited events.
    pub fn money_fields(&self) -> Vec<Money> {
        match self {
            EventPayload::LineItemBilled {
                unit_price,
                line_total,
                ..
            } => vec![*unit_price, *line_total],
            EventPayload::LineTotalMismatch {
                stated, recomputed, ..
            } => vec![*stated, *recomputed],
            EventPayload::VatCalculated { taxable, vat, .. } => vec![*taxable, *vat],
            EventPayload::TotalsStated { net, vat, gross } => vec![*net, *vat, *gross],
            EventPayload::TotalsMismatch {
                stated, recomputed, ..
            } => vec![*stated, *recomputed],
            EventPayload::BalanceStated { amount, .. } => vec![*amount],
            EventPayload::BalanceMismatch {
                carried_forward,
                brought_forward,
                ..
            } => vec![*carried_forward, *brought_forward],
            EventPayload::CreditNoteReferenced { amount, .. } => vec![*amount],
            EventPayload::SupplierIdentified { .. }
            | EventPayload::RecipientIdentified { .. }
            | EventPayload::InvoiceNumberStated { .. }
            | EventPayload::DueDateStated { .. }
            | EventPayload::ComplianceIssueFound { .. }
            | EventPayload::PossibleDuplicate { .. } => Vec::new(),
        }
    }

    /// The text in this payload a person may search for: what the document said, never a kind
    /// name or an amount. The store's search matches the same fields.
    pub fn searchable_text(&self) -> Vec<&str> {
        match self {
            EventPayload::InvoiceNumberStated { number } => vec![number],
            EventPayload::SupplierIdentified { name, vat_number }
            | EventPayload::RecipientIdentified { name, vat_number } => match vat_number {
                Some(vat) => vec![name.as_str(), vat.as_str()],
                None => vec![name.as_str()],
            },
            EventPayload::LineItemBilled { description, .. } => vec![description],
            EventPayload::DueDateStated { terms, .. } => terms.as_deref().into_iter().collect(),
            EventPayload::CreditNoteReferenced {
                original_invoice_number,
                ..
            } => vec![original_invoice_number],
            EventPayload::ComplianceIssueFound { message, .. } => vec![message],
            EventPayload::LineTotalMismatch { .. }
            | EventPayload::VatCalculated { .. }
            | EventPayload::TotalsStated { .. }
            | EventPayload::TotalsMismatch { .. }
            | EventPayload::BalanceStated { .. }
            | EventPayload::BalanceMismatch { .. }
            | EventPayload::PossibleDuplicate { .. } => Vec::new(),
        }
    }

    /// True when every amount in the payload uses the same currency.
    pub(crate) fn has_single_currency(&self) -> bool {
        let amounts = self.money_fields();
        amounts
            .first()
            .is_none_or(|first| amounts.iter().all(|m| m.currency == first.currency))
    }
}
