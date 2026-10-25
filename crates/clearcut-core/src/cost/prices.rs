// Written by Kone Tshivhinda & Claude
//! The price table (research 06 section 1) and the cost of a call.

use super::{Cost, ExchangeRate, TokenUsage};
use crate::error::{Error, Result};
use crate::escalation::{plain_model_id, RouterPolicy};
use serde::{Deserialize, Serialize};

/// Dividing by this turns `tokens x (millionths of a dollar per million tokens) x (rand cents to
/// the dollar)` into nano-rand: 10^6 (per million tokens) x 10^6 (millionths) x 100 (cents to
/// rand) / 10^9 (nano-rand to the rand) = 10^5.
const DIVISOR: u128 = 100_000;

fn invalid(why: &'static str) -> Error {
    Error::InvalidPricing(why)
}

/// What one million tokens of each kind cost, in **millionths of a US dollar**.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rates {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
}

impl Rates {
    /// From the two base prices and the cache-read multiple in thousandths (100 is 0.1x, 25 is
    /// 0.025x). Cache writes are 1.25x (five minutes) and 2x (an hour) the input price on every
    /// model (research 06 section 4). Every product is exact in millionths of a dollar.
    pub fn from_base(input: u64, output: u64, read_permille: u64) -> Rates {
        Rates {
            input,
            output,
            cache_read: input.saturating_mul(read_permille) / 1_000,
            cache_write_5m: input.saturating_mul(5) / 4,
            cache_write_1h: input.saturating_mul(2),
        }
    }
}

/// A higher price for prompts over a length (Haiku 5.5 above 100 000 tokens).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LongPrompt {
    /// The input side of the call (input and both kinds of cache tokens) must be **more** than
    /// this for the long rates to apply, to every token of the call.
    pub above_tokens: u64,
    pub rates: Rates,
}

/// One model's price, and the day it was read from Anthropic's page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelPrice {
    pub model: String,
    /// `YYYY-MM-DD`.
    pub read_on: String,
    pub standard: Rates,
    pub long_prompt: Option<LongPrompt>,
}

/// What the next call could cost at most, worked out before it is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallEstimate {
    /// The page, its text and the document context: everything after the cache breakpoint.
    pub page_input_tokens: u64,
    /// The cached prefix (the rules and examples). It may have to be **written** for an hour.
    pub prefix_tokens: u64,
    /// The largest answer the call may give, thinking included.
    pub max_output_tokens: u32,
}

/// Prices by model id. **Read back from text, it is checked** like one built.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "UncheckedPrices")]
pub struct PriceTable {
    prices: Vec<ModelPrice>,
}

#[derive(Deserialize)]
struct UncheckedPrices {
    prices: Vec<ModelPrice>,
}

impl TryFrom<UncheckedPrices> for PriceTable {
    type Error = Error;

    fn try_from(raw: UncheckedPrices) -> Result<Self> {
        PriceTable::new(raw.prices)
    }
}

fn is_date(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 10
        && bytes.iter().enumerate().all(|(i, b)| match i {
            4 | 7 => *b == b'-',
            _ => b.is_ascii_digit(),
        })
}

impl PriceTable {
    /// Checks and keeps a table. A price of zero is refused: it would make a budget say a call
    /// is free.
    pub fn new(prices: Vec<ModelPrice>) -> Result<PriceTable> {
        if prices.is_empty() {
            return Err(invalid("a price table needs at least one model"));
        }
        for (i, price) in prices.iter().enumerate() {
            if !plain_model_id(&price.model) {
                return Err(invalid("a model id is not a plain identifier"));
            }
            if prices[..i].iter().any(|earlier| earlier.model == price.model) {
                return Err(invalid("a model has two prices"));
            }
            if !is_date(&price.read_on) {
                return Err(invalid("the date a price was read is not YYYY-MM-DD"));
            }
            if price.standard.input == 0 || price.standard.output == 0 {
                return Err(invalid("a price of zero would make a budget meaningless"));
            }
            if let Some(long) = &price.long_prompt {
                if long.above_tokens == 0 {
                    return Err(invalid("a long-prompt threshold must be above zero"));
                }
                if long.rates.input < price.standard.input || long.rates.output < price.standard.output {
                    return Err(invalid("a long-prompt rate is below the standard one"));
                }
            }
        }
        Ok(PriceTable { prices })
    }

    /// The prices of research 06 section 1, read on 22 October 2026. **Check them against
    /// Anthropic's pricing page before relying on them**; they change.
    pub fn default_table() -> PriceTable {
        let price = |model: &str, input, output, read_permille, long: Option<(u64, u64, u64)>| ModelPrice {
            model: model.to_string(),
            read_on: "2026-10-22".to_string(),
            standard: Rates::from_base(input, output, read_permille),
            long_prompt: long.map(|(above_tokens, input, output)| LongPrompt {
                above_tokens,
                rates: Rates::from_base(input, output, read_permille),
            }),
        };
        PriceTable {
            prices: vec![
                // $0.10 and $0.50 per million; $0.50 and $2.50 above 100 000 tokens.
                price(
                    "claude-haiku-5-5",
                    100_000,
                    500_000,
                    100,
                    Some((100_000, 500_000, 2_500_000)),
                ),
                price("claude-sonnet-5-5", 2_000_000, 10_000_000, 50, None),
                price("claude-opus-5-5", 4_000_000, 20_000_000, 50, None),
                price("claude-fable-5-1", 10_000_000, 50_000_000, 25, None),
            ],
        }
    }

    pub fn get(&self, model: &str) -> Option<&ModelPrice> {
        self.prices.iter().find(|p| p.model == model)
    }

    fn unknown(model: &str) -> Error {
        Error::UnknownModelPrice(model.to_string())
    }

    /// The rates a call is billed at: the long ones if its input side is over the threshold.
    fn rates_for(price: &ModelPrice, input_side: u64) -> Rates {
        match &price.long_prompt {
            Some(long) if input_side > long.above_tokens => long.rates,
            _ => price.standard,
        }
    }

    /// Nano-rand for `sum`, rounded up, saturating at the largest cost.
    fn to_cost(sum: u128, rate: ExchangeRate) -> Cost {
        let nano = sum
            .saturating_mul(u128::from(rate.rand_cents_per_usd()))
            .div_ceil(DIVISOR);
        Cost::from_nano_rand(u64::try_from(nano).unwrap_or(u64::MAX))
    }

    /// What a call really cost, from the token counts the provider reported.
    pub fn cost(&self, model: &str, usage: &TokenUsage, rate: ExchangeRate) -> Result<Cost> {
        let price = self.get(model).ok_or_else(|| Self::unknown(model))?;
        let rates = Self::rates_for(price, usage.input_side());
        let writes_1h = usage.cache_write_1h_tokens.min(usage.cache_write_tokens);
        let sum: u128 = [
            (usage.input_tokens, rates.input),
            (usage.output_tokens, rates.output),
            (usage.cache_read_tokens, rates.cache_read),
            (usage.cache_write_5m_tokens(), rates.cache_write_5m),
            (writes_1h, rates.cache_write_1h),
        ]
        .iter()
        .map(|(tokens, price)| u128::from(*tokens) * u128::from(*price))
        .sum();
        Ok(Self::to_cost(sum, rate))
    }

    /// The most a call of this size could cost: the page at the input price, the cached prefix
    /// at whichever is dearer of the input price and the one-hour write price (it may have to be
    /// written), and the largest answer. A real call of that size never costs more.
    pub fn worst_case(&self, model: &str, estimate: &CallEstimate, rate: ExchangeRate) -> Result<Cost> {
        let price = self.get(model).ok_or_else(|| Self::unknown(model))?;
        let input_side = estimate.page_input_tokens.saturating_add(estimate.prefix_tokens);
        let rates = Self::rates_for(price, input_side);
        let prefix_rate = rates.cache_write_1h.max(rates.input);
        let sum: u128 = u128::from(estimate.page_input_tokens) * u128::from(rates.input)
            + u128::from(estimate.prefix_tokens) * u128::from(prefix_rate)
            + u128::from(estimate.max_output_tokens) * u128::from(rates.output);
        Ok(Self::to_cost(sum, rate))
    }

    /// The models of the rungs that are on whose worst-case call does **not** fit `cap`, in
    /// ladder order. A model with no price never fits. The router takes these out of the ladder
    /// for the document (`RouterPolicy::without_models`).
    pub fn unaffordable_models(
        &self,
        policy: &RouterPolicy,
        page_input_tokens: u64,
        prefix_tokens: u64,
        rate: ExchangeRate,
        cap: Cost,
    ) -> Vec<String> {
        policy
            .effective()
            .into_iter()
            .filter(|rung| {
                let estimate = CallEstimate {
                    page_input_tokens,
                    prefix_tokens,
                    max_output_tokens: rung.max_output_tokens,
                };
                !self
                    .worst_case(&rung.model, &estimate, rate)
                    .is_ok_and(|worst| worst <= cap)
            })
            .map(|rung| rung.model.clone())
            .collect()
    }
}
