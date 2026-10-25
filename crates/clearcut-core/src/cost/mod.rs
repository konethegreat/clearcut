// Written by Kone Tshivhinda & Claude
//! What a model call costs, in rand, and what a document may spend (ADR-0018 section 6, the
//! first half of P2-12).
//!
//! **Integers only.** One Haiku token costs under a millionth of a rand, so a cent is far too
//! coarse: a [`Cost`] is a whole number of **nano-rand** (a billionth of a rand), and prices are
//! kept in millionths of a US dollar per million tokens, so that every published price and
//! every cache multiple (1.25x, 2x, 0.1x, 0.05x, 0.025x) is exact. A cost is rounded **up**
//! to the next nano-rand when it is worked out, and up to the next cent when it is shown or
//! compared with a limit in cents: the error is always in the user's favour (we charge a
//! budget a little more, never less).
//!
//! The exchange rate is set by the operator (default R18.00 to the dollar, as everywhere in
//! this project). Prices are those of research 06 and carry the date they were read; **a
//! model without a price cannot be costed, so it cannot be afforded** (the check fails
//! closed). Nothing here knows about a provider: the router converts its own usage figures.

mod budget;
mod prices;

#[cfg(test)]
mod tests_budget;
#[cfg(test)]
mod tests_cost;
#[cfg(test)]
mod tests_prices;

pub use budget::{BudgetCheck, BudgetPolicy, BudgetScope};
pub use prices::{CallEstimate, LongPrompt, ModelPrice, PriceTable, Rates};

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Nano-rand in a rand cent.
const NANO_PER_RAND_CENT: u64 = 10_000_000;

/// An amount of rand, as a whole number of nano-rand (a billionth of a rand).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Cost(u64);

impl Cost {
    pub const ZERO: Cost = Cost(0);

    pub const fn from_nano_rand(nano: u64) -> Cost {
        Cost(nano)
    }

    /// An amount given in rand cents (R5.00 is 500). Saturates rather than wrapping.
    pub const fn from_rand_cents(cents: u64) -> Cost {
        Cost(cents.saturating_mul(NANO_PER_RAND_CENT))
    }

    pub const fn nano_rand(self) -> u64 {
        self.0
    }

    /// Whole rand cents, **rounded up**: a fraction of a cent is shown as a cent.
    pub const fn rand_cents_ceil(self) -> u64 {
        self.0.div_ceil(NANO_PER_RAND_CENT)
    }

    pub const fn checked_add(self, other: Cost) -> Option<Cost> {
        match self.0.checked_add(other.0) {
            Some(sum) => Some(Cost(sum)),
            None => None,
        }
    }

    pub const fn saturating_add(self, other: Cost) -> Cost {
        Cost(self.0.saturating_add(other.0))
    }

    pub const fn saturating_sub(self, other: Cost) -> Cost {
        Cost(self.0.saturating_sub(other.0))
    }

    pub const fn saturating_mul(self, times: u64) -> Cost {
        Cost(self.0.saturating_mul(times))
    }
}

impl fmt::Display for Cost {
    /// `R0.02`: rand and cents, rounded up to the cent.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let cents = self.rand_cents_ceil();
        write!(f, "R{}.{:02}", cents / 100, cents % 100)
    }
}

/// Rand cents to the US dollar, set by the operator. Default R18.00 (1 800).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct ExchangeRate(u32);

impl ExchangeRate {
    /// Between R0.01 and R1 000 to the dollar.
    pub fn new(rand_cents_per_usd: u32) -> Result<ExchangeRate> {
        if rand_cents_per_usd == 0 || rand_cents_per_usd > 100_000 {
            return Err(Error::InvalidPricing(
                "the exchange rate is outside R0.01 to R1000 a dollar",
            ));
        }
        Ok(ExchangeRate(rand_cents_per_usd))
    }

    pub fn rand_cents_per_usd(self) -> u32 {
        self.0
    }
}

impl Default for ExchangeRate {
    fn default() -> Self {
        ExchangeRate(1_800)
    }
}

impl TryFrom<u32> for ExchangeRate {
    type Error = Error;

    fn try_from(cents: u32) -> Result<Self> {
        ExchangeRate::new(cents)
    }
}

impl From<ExchangeRate> for u32 {
    fn from(rate: ExchangeRate) -> u32 {
        rate.0
    }
}

/// The token counts of one call, as the provider reports them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    /// Input after the last cache breakpoint.
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    /// Everything written to the cache, of either lifetime.
    pub cache_write_tokens: u64,
    /// The part of `cache_write_tokens` written with the one-hour lifetime.
    pub cache_write_1h_tokens: u64,
}

impl TokenUsage {
    /// Everything the model read: what Haiku's long-prompt price is decided on.
    pub fn input_side(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.cache_read_tokens)
            .saturating_add(self.cache_write_tokens)
    }

    /// Writes with the five-minute lifetime: the writes that were not one-hour ones.
    pub fn cache_write_5m_tokens(&self) -> u64 {
        self.cache_write_tokens.saturating_sub(self.cache_write_1h_tokens)
    }
}
