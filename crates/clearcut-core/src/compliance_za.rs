// Written by Kone Tshivhinda & Claude
//! South African VAT invoice rules (VAT Act s20, BGR 15). Thresholds are
//! VAT-inclusive and may change; verify against SARS before relying on them.

use crate::money::Money;
use serde::{Deserialize, Serialize};

/// Above this (VAT-inclusive, in cents) a full tax invoice is required: R5 000.
pub const FULL_TAX_INVOICE_ABOVE_MINOR: i64 = 500_000;
/// At or below this no formal tax invoice is required: R50.
pub const NO_INVOICE_AT_OR_BELOW_MINOR: i64 = 5_000;

#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaxInvoiceKind {
    /// Over R5 000: must include recipient name, address and VAT number.
    Full,
    /// R50.01 to R5 000: recipient details not required.
    Abridged,
    /// R50 or less: a till slip suffices.
    NotRequired,
}

pub fn required_kind(gross: Money) -> TaxInvoiceKind {
    if gross.minor > FULL_TAX_INVOICE_ABOVE_MINOR {
        TaxInvoiceKind::Full
    } else if gross.minor > NO_INVOICE_AT_OR_BELOW_MINOR {
        TaxInvoiceKind::Abridged
    } else {
        TaxInvoiceKind::NotRequired
    }
}

/// Parties and metadata needed to check a tax invoice.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InvoiceHeader {
    pub supplier_name: Option<String>,
    pub supplier_address: Option<String>,
    pub supplier_vat_number: Option<String>,
    pub recipient_name: Option<String>,
    pub recipient_address: Option<String>,
    pub recipient_vat_number: Option<String>,
    pub serial_number: Option<String>,
    pub has_date_of_issue: bool,
    pub has_description: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issue {
    pub field: &'static str,
    pub message: &'static str,
}

fn missing(v: &Option<String>) -> bool {
    v.as_deref().map(str::trim).unwrap_or("").is_empty()
}

/// SA VAT numbers are 10 digits starting with 4.
pub fn is_valid_vat_number_format(v: &str) -> bool {
    v.len() == 10 && v.starts_with('4') && v.bytes().all(|b| b.is_ascii_digit())
}

pub fn validate(header: &InvoiceHeader, gross: Money) -> Vec<Issue> {
    let kind = required_kind(gross);
    let mut out = Vec::new();
    if kind == TaxInvoiceKind::NotRequired {
        return out;
    }
    let mut need = |cond: bool, field: &'static str, message: &'static str| {
        if cond {
            out.push(Issue { field, message });
        }
    };
    need(
        missing(&header.supplier_name),
        "supplier_name",
        "supplier name is required",
    );
    need(
        missing(&header.supplier_address),
        "supplier_address",
        "supplier address is required",
    );
    need(
        header
            .supplier_vat_number
            .as_deref()
            .is_none_or(|v| !is_valid_vat_number_format(v)),
        "supplier_vat_number",
        "supplier VAT number must be 10 digits starting with 4",
    );
    need(
        missing(&header.serial_number),
        "serial_number",
        "unique sequential serial number is required",
    );
    need(
        !header.has_date_of_issue,
        "date_of_issue",
        "date of issue is required",
    );
    need(
        !header.has_description,
        "description",
        "description of goods or services is required",
    );
    if kind == TaxInvoiceKind::Full {
        need(
            missing(&header.recipient_name),
            "recipient_name",
            "recipient name is required above R5 000",
        );
        need(
            missing(&header.recipient_address),
            "recipient_address",
            "recipient address is required above R5 000",
        );
        need(
            header
                .recipient_vat_number
                .as_deref()
                .is_none_or(|v| !is_valid_vat_number_format(v)),
            "recipient_vat_number",
            "recipient VAT number is required above R5 000",
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thresholds() {
        assert_eq!(required_kind(Money::zar(5_000)), TaxInvoiceKind::NotRequired);
        assert_eq!(required_kind(Money::zar(5_001)), TaxInvoiceKind::Abridged);
        assert_eq!(required_kind(Money::zar(500_000)), TaxInvoiceKind::Abridged);
        assert_eq!(required_kind(Money::zar(500_001)), TaxInvoiceKind::Full);
    }

    #[test]
    fn vat_number_format() {
        assert!(is_valid_vat_number_format("4123456789"));
        assert!(!is_valid_vat_number_format("3123456789"));
        assert!(!is_valid_vat_number_format("412345678"));
    }

    fn complete() -> InvoiceHeader {
        InvoiceHeader {
            supplier_name: Some("Acme (Pty) Ltd".into()),
            supplier_address: Some("1 Main Rd, Houghton".into()),
            supplier_vat_number: Some("4123456789".into()),
            recipient_name: Some("Client Co".into()),
            recipient_address: Some("2 High St, Sandton".into()),
            recipient_vat_number: Some("4987654321".into()),
            serial_number: Some("INV-000001".into()),
            has_date_of_issue: true,
            has_description: true,
        }
    }

    #[test]
    fn complete_full_invoice_is_clean() {
        assert!(validate(&complete(), Money::zar(1_000_000)).is_empty());
    }

    #[test]
    fn full_invoice_needs_recipient() {
        let mut h = complete();
        h.recipient_vat_number = None;
        let issues = validate(&h, Money::zar(1_000_000));
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].field, "recipient_vat_number");
    }

    #[test]
    fn abridged_does_not_need_recipient() {
        let mut h = complete();
        h.recipient_name = None;
        h.recipient_address = None;
        h.recipient_vat_number = None;
        assert!(validate(&h, Money::zar(100_000)).is_empty());
    }
}
