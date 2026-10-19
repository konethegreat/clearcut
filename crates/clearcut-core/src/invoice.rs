// Written by Kone Tshivhinda & Claude
use crate::error::{Error, Result};
use crate::money::{Currency, Money};
use crate::status::InvoiceStatus;
use crate::tax::TaxCategory;
use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineItem {
    pub description: String,
    /// Quantity in thousandths (1000 = 1 unit, 2500 = 2.5 units).
    pub quantity_milli: i64,
    /// Unit price excluding VAT.
    pub unit_price: Money,
    pub tax: TaxCategory,
}

impl LineItem {
    pub fn net(&self) -> Result<Money> {
        if self.quantity_milli <= 0 {
            return Err(Error::InvalidQuantity);
        }
        self.unit_price.mul_milli(self.quantity_milli)
    }
}

#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoiceTotals {
    pub net: Money,
    pub vat: Money,
    pub gross: Money,
}

#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invoice {
    pub number: String,
    pub currency: Currency,
    pub status: InvoiceStatus,
    pub lines: Vec<LineItem>,
}

impl Invoice {
    pub fn new(number: impl Into<String>, currency: Currency, lines: Vec<LineItem>) -> Self {
        Self {
            number: number.into(),
            currency,
            status: InvoiceStatus::Draft,
            lines,
        }
    }

    /// VAT is calculated per tax category on the category subtotal (not per
    /// line) to avoid accumulating rounding differences.
    pub fn totals(&self) -> Result<InvoiceTotals> {
        if self.lines.is_empty() {
            return Err(Error::EmptyInvoice);
        }
        let mut net = Money::zero(self.currency);
        let mut vat = Money::zero(self.currency);
        for cat in [
            TaxCategory::Standard,
            TaxCategory::ZeroRated,
            TaxCategory::Exempt,
            TaxCategory::OutOfScope,
        ] {
            let mut subtotal = Money::zero(self.currency);
            for line in self.lines.iter().filter(|l| l.tax == cat) {
                subtotal = subtotal.checked_add(line.net()?)?;
            }
            net = net.checked_add(subtotal)?;
            vat = vat.checked_add(subtotal.percent_bps(cat.rate_bps())?)?;
        }
        Ok(InvoiceTotals {
            net,
            vat,
            gross: net.checked_add(vat)?,
        })
    }

    pub fn transition(&mut self, to: InvoiceStatus) -> Result<()> {
        self.status = self.status.transition(to)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(desc: &str, qty: i64, price: i64, tax: TaxCategory) -> LineItem {
        LineItem {
            description: desc.into(),
            quantity_milli: qty,
            unit_price: Money::zar(price),
            tax,
        }
    }

    #[test]
    fn totals_mixed_rates() {
        let inv = Invoice::new(
            "INV-000001",
            Currency::Zar,
            vec![
                line("Consulting", 10_000, 150_000, TaxCategory::Standard), // 10 x R1 500
                line("Export licence", 1_000, 100_000, TaxCategory::ZeroRated),
            ],
        );
        let t = inv.totals().unwrap();
        assert_eq!(t.net, Money::zar(1_600_000));
        assert_eq!(t.vat, Money::zar(225_000));
        assert_eq!(t.gross, Money::zar(1_825_000));
    }

    #[test]
    fn empty_invoice_rejected() {
        let inv = Invoice::new("X", Currency::Zar, vec![]);
        assert_eq!(inv.totals(), Err(Error::EmptyInvoice));
    }

    #[test]
    fn zero_quantity_rejected() {
        let inv = Invoice::new("X", Currency::Zar, vec![line("a", 0, 100, TaxCategory::Standard)]);
        assert_eq!(inv.totals(), Err(Error::InvalidQuantity));
    }

    #[test]
    fn serde_roundtrip() {
        let inv = Invoice::new(
            "INV-1",
            Currency::Zar,
            vec![line("a", 1000, 100, TaxCategory::Standard)],
        );
        let json = serde_json::to_string(&inv).unwrap();
        assert_eq!(serde_json::from_str::<Invoice>(&json).unwrap(), inv);
    }
}
