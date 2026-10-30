// Written by Kone Tshivhinda & Claude
//! Money is stored as integer minor units (cents). Floating point is never
//! used for amounts. See ADR-0002.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::fmt;

#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Currency {
    Zar,
    Usd,
    Eur,
    Gbp,
}

impl Currency {
    pub fn code(self) -> &'static str {
        match self {
            Currency::Zar => "ZAR",
            Currency::Usd => "USD",
            Currency::Eur => "EUR",
            Currency::Gbp => "GBP",
        }
    }
}

#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Money {
    /// Amount in minor units (e.g. cents). 1 050 = R10.50.
    pub minor: i64,
    pub currency: Currency,
}

impl Money {
    pub const fn new(minor: i64, currency: Currency) -> Self {
        Self { minor, currency }
    }

    pub const fn zar(minor: i64) -> Self {
        Self::new(minor, Currency::Zar)
    }

    pub const fn zero(currency: Currency) -> Self {
        Self::new(0, currency)
    }

    pub fn checked_add(self, rhs: Money) -> Result<Money> {
        self.same_currency(rhs)?;
        let minor = self.minor.checked_add(rhs.minor).ok_or(Error::Overflow)?;
        Ok(Money::new(minor, self.currency))
    }

    pub fn checked_sub(self, rhs: Money) -> Result<Money> {
        self.same_currency(rhs)?;
        let minor = self.minor.checked_sub(rhs.minor).ok_or(Error::Overflow)?;
        Ok(Money::new(minor, self.currency))
    }

    /// Multiply by a quantity expressed in thousandths (1000 = 1.000 units),
    /// rounding half away from zero to the nearest minor unit.
    pub fn mul_milli(self, qty_milli: i64) -> Result<Money> {
        let wide = (self.minor as i128) * (qty_milli as i128);
        Ok(Money::new(div_round_half_away(wide, 1_000)?, self.currency))
    }

    /// Apply a rate in basis points (1500 = 15.00%), rounding half away from zero.
    pub fn percent_bps(self, bps: u32) -> Result<Money> {
        let wide = (self.minor as i128) * (bps as i128);
        Ok(Money::new(div_round_half_away(wide, 10_000)?, self.currency))
    }

    /// The amount exclusive of a rate already included in it (1500 = the price includes
    /// 15.00%), rounding half away from zero. R1.15 at 15% becomes R1.00. This is the
    /// inverse of [`Money::percent_bps`] only where the division is exact.
    pub fn excluding_bps(self, bps: u32) -> Result<Money> {
        let wide = (self.minor as i128) * 10_000;
        let denominator = 10_000 + bps as i128;
        Ok(Money::new(div_round_half_away(wide, denominator)?, self.currency))
    }

    fn same_currency(self, rhs: Money) -> Result<()> {
        if self.currency != rhs.currency {
            return Err(Error::CurrencyMismatch {
                expected: self.currency.code(),
                got: rhs.currency.code(),
            });
        }
        Ok(())
    }
}

fn div_round_half_away(n: i128, d: i128) -> Result<i64> {
    let half = d / 2;
    let q = if n >= 0 { (n + half) / d } else { (n - half) / d };
    i64::try_from(q).map_err(|_| Error::Overflow)
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.minor < 0 { "-" } else { "" };
        let abs = self.minor.unsigned_abs();
        let symbol = match self.currency {
            Currency::Zar => "R",
            Currency::Usd => "$",
            Currency::Eur => "€",
            Currency::Gbp => "£",
        };
        write!(f, "{sign}{symbol}{}.{:02}", abs / 100, abs % 100)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displays_rand() {
        assert_eq!(Money::zar(1050).to_string(), "R10.50");
        assert_eq!(Money::zar(-5).to_string(), "-R0.05");
    }

    #[test]
    fn vat_rounds_half_up() {
        // 15% of R0.10 = R0.015 -> R0.02
        assert_eq!(Money::zar(10).percent_bps(1500).unwrap(), Money::zar(2));
        // 15% of R1.00 = R0.15 exactly
        assert_eq!(Money::zar(100).percent_bps(1500).unwrap(), Money::zar(15));
    }

    #[test]
    fn fractional_quantity() {
        // 2.5 x R10.01 = R25.025 -> R25.03
        assert_eq!(Money::zar(1001).mul_milli(2500).unwrap(), Money::zar(2503));
    }

    #[test]
    fn a_vat_inclusive_amount_is_worked_back_to_its_net() {
        // R1.15 at 15% is R1.00 exactly.
        assert_eq!(Money::zar(115).excluding_bps(1500).unwrap(), Money::zar(100));
        // R29.98 at 15% is R26.069... -> R26.07.
        assert_eq!(Money::zar(2998).excluding_bps(1500).unwrap(), Money::zar(2607));
        // R0.10 at 15% is R0.0869... -> R0.09, and the VAT comes back as R0.01.
        assert_eq!(Money::zar(10).excluding_bps(1500).unwrap(), Money::zar(9));
        // Negative amounts round away from zero too.
        assert_eq!(Money::zar(-115).excluding_bps(1500).unwrap(), Money::zar(-100));
    }

    #[test]
    fn rejects_mixed_currency() {
        let a = Money::zar(1);
        let b = Money::new(1, Currency::Usd);
        assert!(a.checked_add(b).is_err());
    }
}
