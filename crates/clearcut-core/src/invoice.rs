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
    /// The line amount **as the caller has already worked it out** (excluding VAT, in minor units).
    /// When given it is used as it is, and `quantity_milli` and `unit_price` are only informational:
    /// a caller that prices its own lines (for example a time entry) is not rounded a second time.
    /// When absent the line is `quantity_milli` times `unit_price`, rounded half away from zero.
    /// Must be in the same currency as `unit_price`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_total: Option<Money>,
}

impl LineItem {
    pub fn net(&self) -> Result<Money> {
        if self.quantity_milli <= 0 {
            return Err(Error::InvalidQuantity);
        }
        match self.line_total {
            // Adding it to zero in the line's currency is the currency check.
            Some(given) => Money::zero(self.unit_price.currency).checked_add(given),
            None => self.unit_price.mul_milli(self.quantity_milli),
        }
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
            line_total: None,
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

    /// The pair TimeLex asked for (docs/integration/timelex.md): 1.5 hours at R333.33 is R499.995,
    /// which is worked out as R500.00; a caller that has priced the line at R499.99 is not rounded
    /// again. VAT (15%) is then R75.00 either way (R74.9985 rounds up), the totals differ by a cent.
    #[test]
    fn a_line_total_given_by_the_caller_is_used_as_it_is() {
        let computed = Invoice::new(
            "T-1",
            Currency::Zar,
            vec![line("Advice", 1_500, 33_333, TaxCategory::Standard)],
        );
        let t = computed.totals().unwrap();
        assert_eq!(
            (t.net, t.vat, t.gross),
            (Money::zar(50_000), Money::zar(7_500), Money::zar(57_500))
        );

        let mut given = line("Advice", 1_500, 33_333, TaxCategory::Standard);
        given.line_total = Some(Money::zar(49_999));
        let t = Invoice::new("T-2", Currency::Zar, vec![given]).totals().unwrap();
        assert_eq!(
            (t.net, t.vat, t.gross),
            (Money::zar(49_999), Money::zar(7_500), Money::zar(57_499))
        );
    }

    #[test]
    fn a_given_line_total_in_another_currency_is_refused_and_absent_means_computed() {
        let mut odd = line("a", 1_000, 100, TaxCategory::Standard);
        odd.line_total = Some(Money::new(100, Currency::Usd));
        assert!(matches!(
            Invoice::new("X", Currency::Zar, vec![odd]).totals(),
            Err(Error::CurrencyMismatch { .. })
        ));
        // Old callers send no line_total and get the same answer as before.
        let json = r#"{"description":"a","quantity_milli":1000,"unit_price":{"minor":100,"currency":"ZAR"},"tax":"standard"}"#;
        let parsed: LineItem = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.line_total, None);
        assert_eq!(parsed.net().unwrap(), Money::zar(100));
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
