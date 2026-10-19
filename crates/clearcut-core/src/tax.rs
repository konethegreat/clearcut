// Written by Kone Tshivhinda & Claude
use serde::{Deserialize, Serialize};

/// South African VAT categories (VAT Act 89 of 1991). Rate is in basis points.
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaxCategory {
    /// Standard-rated, 15%.
    Standard,
    /// Zero-rated supply (e.g. exports): VAT charged at 0% but still reportable.
    ZeroRated,
    /// Exempt supply: no VAT, not claimable as input tax.
    Exempt,
    /// Outside the scope of VAT.
    OutOfScope,
}

/// Standard VAT rate in basis points. Source: SARS; unchanged at 15% for 2026.
pub const ZA_STANDARD_VAT_BPS: u32 = 1500;

impl TaxCategory {
    pub fn rate_bps(self) -> u32 {
        match self {
            TaxCategory::Standard => ZA_STANDARD_VAT_BPS,
            _ => 0,
        }
    }
}
