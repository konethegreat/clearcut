// Written by Kone Tshivhinda & Claude
//! VAT categories and the jurisdiction that prices them.
//!
//! A [`TaxProfile`] is the one place a jurisdiction's standard rate lives. ZA is the only
//! profile built today; adding a second jurisdiction means adding a profile and choosing it
//! where a document is read, not hunting for a rate in the calculator, the derivation, the
//! narrator and the agent tools. Thresholds and VAT-number rules stay in
//! [`crate::compliance_za`], because they are facts about South African law rather than rates.

use serde::{Deserialize, Serialize};

/// South African VAT categories (VAT Act 89 of 1991). Rate is in basis points.
// The variants are declared in the order they are grouped and reported in: Standard,
// ZeroRated, Exempt, OutOfScope. `Ord` follows declaration order.
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaxCategory {
    /// Standard-rated, 15% under ZA VAT.
    Standard,
    /// Zero-rated supply (e.g. exports): VAT charged at 0% but still reportable.
    ZeroRated,
    /// Exempt supply: no VAT, not claimable as input tax.
    Exempt,
    /// Outside the scope of VAT.
    OutOfScope,
}

/// The VAT rules of one jurisdiction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaxProfile {
    /// The jurisdiction's short code, as printed in reports and errors.
    pub jurisdiction: &'static str,
    /// The standard rate in basis points (1500 = 15.00%).
    pub standard_rate_bps: u32,
}

impl TaxProfile {
    /// The rate this profile charges a category at. Only the standard category is charged;
    /// zero-rated, exempt and out-of-scope supplies carry no VAT under any profile.
    pub fn rate_bps(self, category: TaxCategory) -> u32 {
        match category {
            TaxCategory::Standard => self.standard_rate_bps,
            _ => 0,
        }
    }
}

/// South Africa: standard rate 15%. Source: SARS; unchanged for 2026.
pub const ZA_VAT: TaxProfile = TaxProfile {
    jurisdiction: "ZA",
    standard_rate_bps: 1_500,
};

impl TaxCategory {
    /// The rate this category is charged at under `profile`.
    pub fn rate_bps_in(self, profile: TaxProfile) -> u32 {
        profile.rate_bps(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn south_africa_charges_fifteen_percent_on_the_standard_category_only() {
        assert_eq!(ZA_VAT.jurisdiction, "ZA");
        assert_eq!(ZA_VAT.standard_rate_bps, 1_500);
        assert_eq!(ZA_VAT.rate_bps(TaxCategory::Standard), 1_500);
        for category in [
            TaxCategory::ZeroRated,
            TaxCategory::Exempt,
            TaxCategory::OutOfScope,
        ] {
            assert_eq!(ZA_VAT.rate_bps(category), 0, "{category:?}");
        }
    }

    #[test]
    fn a_second_profile_prices_the_standard_category_its_own_way() {
        let elsewhere = TaxProfile {
            jurisdiction: "XX",
            standard_rate_bps: 2_000,
        };
        assert_eq!(TaxCategory::Standard.rate_bps_in(elsewhere), 2_000);
        assert_eq!(TaxCategory::ZeroRated.rate_bps_in(elsewhere), 0);
    }
}
