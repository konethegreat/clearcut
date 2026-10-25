// Written by Kone Tshivhinda & Claude
use super::*;
use crate::Error;

// ---------- Cost: nano-rand, an integer, rounded up when shown ----------

#[test]
fn a_rand_is_a_billion_nano_rand_and_a_rand_cent_ten_million() {
    assert_eq!(Cost::from_rand_cents(100).nano_rand(), 1_000_000_000);
    assert_eq!(Cost::from_rand_cents(1).nano_rand(), 10_000_000);
    assert_eq!(Cost::ZERO.nano_rand(), 0);
    assert_eq!(Cost::from_nano_rand(7).nano_rand(), 7);
}

#[test]
fn rand_cents_are_rounded_up_never_down() {
    let cents = |nano| Cost::from_nano_rand(nano).rand_cents_ceil();
    assert_eq!(cents(0), 0);
    assert_eq!(cents(1), 1, "a fraction of a cent is a cent");
    assert_eq!(cents(10_000_000), 1);
    assert_eq!(cents(10_000_001), 2);
    assert_eq!(cents(19_800_000), 2);
    assert_eq!(cents(5_000_000_000), 500);
    assert_eq!(cents(5_000_000_001), 501);
    assert_eq!(cents(u64::MAX), u64::MAX.div_ceil(10_000_000));
}

#[test]
fn huge_rand_cents_do_not_wrap() {
    assert_eq!(Cost::from_rand_cents(u64::MAX), Cost::from_nano_rand(u64::MAX));
}

#[test]
fn costs_add_and_subtract_without_wrapping() {
    let a = Cost::from_nano_rand(u64::MAX - 1);
    assert_eq!(
        a.saturating_add(Cost::from_nano_rand(5)),
        Cost::from_nano_rand(u64::MAX)
    );
    assert_eq!(a.checked_add(Cost::from_nano_rand(5)), None);
    assert_eq!(
        Cost::from_nano_rand(3).checked_add(Cost::from_nano_rand(4)),
        Some(Cost::from_nano_rand(7))
    );
    assert_eq!(
        Cost::from_nano_rand(3).saturating_sub(Cost::from_nano_rand(5)),
        Cost::ZERO
    );
    assert_eq!(
        Cost::from_nano_rand(9).saturating_sub(Cost::from_nano_rand(5)),
        Cost::from_nano_rand(4)
    );
    assert_eq!(
        Cost::from_nano_rand(u64::MAX).saturating_mul(2),
        Cost::from_nano_rand(u64::MAX)
    );
    assert_eq!(
        Cost::from_nano_rand(21).saturating_mul(2),
        Cost::from_nano_rand(42)
    );
}

#[test]
fn a_cost_is_shown_in_rand_and_cents_rounded_up() {
    assert_eq!(Cost::ZERO.to_string(), "R0.00");
    assert_eq!(Cost::from_nano_rand(19_800_000).to_string(), "R0.02");
    assert_eq!(Cost::from_rand_cents(500).to_string(), "R5.00");
    assert_eq!(Cost::from_nano_rand(5_000_000_001).to_string(), "R5.01");
    assert_eq!(Cost::from_rand_cents(123_456).to_string(), "R1234.56");
    assert_eq!(Cost::from_rand_cents(9).to_string(), "R0.09");
}

#[test]
fn costs_compare_by_size() {
    assert!(Cost::from_rand_cents(5) < Cost::from_rand_cents(6));
    assert!(Cost::from_rand_cents(6) > Cost::ZERO);
}

// ---------- the exchange rate ----------

#[test]
fn the_default_rate_is_eighteen_rand_to_the_dollar() {
    assert_eq!(ExchangeRate::default().rand_cents_per_usd(), 1_800);
}

#[test]
fn a_rate_must_be_a_sensible_one() {
    for (cents, ok) in [
        (0, false),
        (1, true),
        (1_800, true),
        (100_000, true),
        (100_001, false),
    ] {
        assert_eq!(ExchangeRate::new(cents).is_ok(), ok, "{cents}");
    }
    assert_eq!(
        ExchangeRate::new(0),
        Err(Error::InvalidPricing(
            "the exchange rate is outside R0.01 to R1000 a dollar"
        ))
    );
}

#[test]
fn a_rate_read_from_text_is_checked_like_one_built() {
    let ok: ExchangeRate = serde_json::from_str("1850").unwrap();
    assert_eq!(ok.rand_cents_per_usd(), 1_850);
    assert!(serde_json::from_str::<ExchangeRate>("0").is_err());
    assert!(serde_json::from_str::<ExchangeRate>("100001").is_err());
    assert_eq!(serde_json::to_string(&ExchangeRate::default()).unwrap(), "1800");
}

// ---------- token counts ----------

#[test]
fn the_input_side_of_a_call_is_input_plus_both_cache_kinds() {
    let usage = TokenUsage {
        input_tokens: 10,
        output_tokens: 99,
        cache_read_tokens: 200,
        cache_write_tokens: 3_000,
        cache_write_1h_tokens: 1_000,
    };
    assert_eq!(usage.input_side(), 3_210);
    assert_eq!(usage.cache_write_5m_tokens(), 2_000);
    // The one-hour part can never be more than the writes, whatever is reported.
    let odd = TokenUsage {
        cache_write_tokens: 5,
        cache_write_1h_tokens: 9,
        ..TokenUsage::default()
    };
    assert_eq!(odd.cache_write_5m_tokens(), 0);
    assert_eq!(
        TokenUsage {
            input_tokens: u64::MAX,
            cache_read_tokens: 1,
            ..TokenUsage::default()
        }
        .input_side(),
        u64::MAX
    );
}
