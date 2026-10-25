// Written by Kone Tshivhinda & Claude
use super::*;
use crate::escalation::RouterPolicy;
use crate::Error;

const HAIKU: &str = "claude-haiku-5-5";
const SONNET: &str = "claude-sonnet-5-5";
const OPUS: &str = "claude-opus-5-5";
const FABLE: &str = "claude-fable-5-1";

fn table() -> PriceTable {
    PriceTable::default_table()
}

fn rate() -> ExchangeRate {
    ExchangeRate::default()
}

fn usage(input: u64, output: u64) -> TokenUsage {
    TokenUsage {
        input_tokens: input,
        output_tokens: output,
        ..TokenUsage::default()
    }
}

fn nano(cost: crate::error::Result<Cost>) -> u64 {
    cost.unwrap().nano_rand()
}

// ---------- the table of 22 October 2026 (research 06 section 1) ----------

#[test]
fn the_default_table_holds_the_four_models_with_the_date_it_was_read() {
    let t = table();
    for model in [HAIKU, SONNET, OPUS, FABLE] {
        assert_eq!(t.get(model).unwrap().read_on, "2026-10-22", "{model}");
    }
    assert!(t.get("claude-nothing").is_none());
}

#[test]
fn the_rates_are_the_published_prices_in_millionths_of_a_dollar_per_million_tokens() {
    let t = table();
    let r = |m: &str| t.get(m).unwrap().standard;
    assert_eq!((r(HAIKU).input, r(HAIKU).output), (100_000, 500_000));
    assert_eq!((r(SONNET).input, r(SONNET).output), (2_000_000, 10_000_000));
    assert_eq!((r(OPUS).input, r(OPUS).output), (4_000_000, 20_000_000));
    assert_eq!((r(FABLE).input, r(FABLE).output), (10_000_000, 50_000_000));
}

#[test]
fn cache_reads_and_writes_are_fixed_multiples_of_the_input_price() {
    let t = table();
    let r = |m: &str| t.get(m).unwrap().standard;
    // Reads: 0.1x on Haiku, 0.05x on Sonnet and Opus, 0.025x on Fable.
    assert_eq!(r(HAIKU).cache_read, 10_000);
    assert_eq!(r(SONNET).cache_read, 100_000);
    assert_eq!(r(OPUS).cache_read, 200_000);
    assert_eq!(r(FABLE).cache_read, 250_000);
    // Writes: 1.25x for five minutes, 2x for an hour, on all four.
    for m in [HAIKU, SONNET, OPUS, FABLE] {
        assert_eq!(r(m).cache_write_5m, r(m).input * 5 / 4, "{m}");
        assert_eq!(r(m).cache_write_1h, r(m).input * 2, "{m}");
    }
}

#[test]
fn only_haiku_is_dearer_above_a_hundred_thousand_tokens() {
    let t = table();
    let long = t.get(HAIKU).unwrap().long_prompt.unwrap();
    assert_eq!(long.above_tokens, 100_000);
    assert_eq!((long.rates.input, long.rates.output), (500_000, 2_500_000));
    assert_eq!(long.rates.cache_read, 50_000);
    assert_eq!(long.rates.cache_write_5m, 625_000);
    assert_eq!(long.rates.cache_write_1h, 1_000_000);
    for m in [SONNET, OPUS, FABLE] {
        assert!(t.get(m).unwrap().long_prompt.is_none(), "{m}");
    }
}

// ---------- the cost of a call, to the nano-rand ----------

#[test]
fn a_page_costs_what_research_06_says_at_eighteen_rand_to_the_dollar() {
    // 6 000 tokens in, 1 000 out, nothing cached.
    let page = usage(6_000, 1_000);
    let t = table();
    assert_eq!(nano(t.cost(HAIKU, &page, rate())), 19_800_000); // R0.0198, shown as R0.02
    assert_eq!(nano(t.cost(SONNET, &page, rate())), 396_000_000); // R0.396, "about R0.40"
    assert_eq!(nano(t.cost(OPUS, &page, rate())), 792_000_000); // R0.792, "about R0.79"
    assert_eq!(nano(t.cost(FABLE, &page, rate())), 1_980_000_000); // R1.98
    assert_eq!(t.cost(HAIKU, &page, rate()).unwrap().to_string(), "R0.02");
    assert_eq!(t.cost(FABLE, &page, rate()).unwrap().to_string(), "R1.98");
}

#[test]
fn input_and_output_are_priced_apart() {
    let t = table();
    assert_eq!(nano(t.cost(HAIKU, &usage(6_000, 0), rate())), 10_800_000);
    assert_eq!(nano(t.cost(HAIKU, &usage(0, 1_000), rate())), 9_000_000);
    assert_eq!(nano(t.cost(HAIKU, &usage(0, 0), rate())), 0);
}

#[test]
fn cache_reads_are_cheap_and_writes_cost_a_premium() {
    let t = table();
    let reads = TokenUsage {
        cache_read_tokens: 10_000,
        ..TokenUsage::default()
    };
    assert_eq!(nano(t.cost(HAIKU, &reads, rate())), 1_800_000);
    let five = TokenUsage {
        cache_write_tokens: 1_000,
        ..TokenUsage::default()
    };
    assert_eq!(nano(t.cost(HAIKU, &five, rate())), 2_250_000);
    let hour = TokenUsage {
        cache_write_tokens: 1_000,
        cache_write_1h_tokens: 1_000,
        ..TokenUsage::default()
    };
    assert_eq!(nano(t.cost(HAIKU, &hour, rate())), 3_600_000);
    // 3 000 written, 1 000 of them for an hour: 2 000 at the five-minute price.
    let mixed = TokenUsage {
        cache_write_tokens: 3_000,
        cache_write_1h_tokens: 1_000,
        ..TokenUsage::default()
    };
    assert_eq!(nano(t.cost(HAIKU, &mixed, rate())), 4_500_000 + 3_600_000);
}

#[test]
fn every_kind_of_token_is_added_into_one_cost() {
    let all = TokenUsage {
        input_tokens: 6_000,
        output_tokens: 1_000,
        cache_read_tokens: 10_000,
        cache_write_tokens: 3_000,
        cache_write_1h_tokens: 1_000,
    };
    let expected = 10_800_000 + 9_000_000 + 1_800_000 + 4_500_000 + 3_600_000;
    assert_eq!(nano(table().cost(HAIKU, &all, rate())), expected);
}

#[test]
fn the_exchange_rate_scales_the_cost() {
    let page = usage(6_000, 1_000);
    let t = table();
    let dearer = ExchangeRate::new(1_900).unwrap();
    assert_eq!(nano(t.cost(HAIKU, &page, dearer)), 20_900_000);
    let cheaper = ExchangeRate::new(900).unwrap();
    assert_eq!(nano(t.cost(HAIKU, &page, cheaper)), 9_900_000);
}

#[test]
fn a_fraction_of_a_nano_rand_is_charged_as_a_whole_one() {
    // One cache-read token on Haiku at R18.01 is 180.1 nano-rand: charged as 181.
    let one = TokenUsage {
        cache_read_tokens: 1,
        ..TokenUsage::default()
    };
    let odd = ExchangeRate::new(1_801).unwrap();
    assert_eq!(nano(table().cost(HAIKU, &one, odd)), 181);
    // And exactly 180 at R18.00: nothing is added when nothing is left over.
    assert_eq!(nano(table().cost(HAIKU, &one, rate())), 180);
}

#[test]
fn haiku_above_a_hundred_thousand_tokens_is_priced_at_the_long_rates() {
    let t = table();
    // The input side is input + cache reads + cache writes.
    assert_eq!(nano(t.cost(HAIKU, &usage(100_000, 0), rate())), 100_000 * 1_800);
    assert_eq!(nano(t.cost(HAIKU, &usage(100_001, 0), rate())), 100_001 * 9_000);
    let split = TokenUsage {
        input_tokens: 40_000,
        cache_read_tokens: 40_000,
        cache_write_tokens: 20_001,
        ..TokenUsage::default()
    };
    // Over by one token in all: every token, including the cached ones, at the long rates.
    assert_eq!(
        nano(t.cost(HAIKU, &split, rate())),
        40_000 * 9_000 + 40_000 * 900 + 20_001 * 11_250
    );
    // Output is dearer too.
    assert_eq!(
        nano(t.cost(HAIKU, &usage(100_001, 1_000), rate())),
        100_001 * 9_000 + 1_000 * 45_000
    );
    // The others charge the same per token however long the prompt.
    assert_eq!(nano(t.cost(SONNET, &usage(900_000, 0), rate())), 900_000 * 36_000);
}

#[test]
fn a_model_with_no_price_cannot_be_costed() {
    assert_eq!(
        table().cost("claude-nothing", &usage(1, 1), rate()),
        Err(Error::UnknownModelPrice("claude-nothing".to_string()))
    );
}

#[test]
fn absurd_counts_saturate_instead_of_wrapping() {
    let huge = TokenUsage {
        input_tokens: u64::MAX,
        output_tokens: u64::MAX,
        cache_read_tokens: u64::MAX,
        cache_write_tokens: u64::MAX,
        cache_write_1h_tokens: u64::MAX,
    };
    assert_eq!(nano(table().cost(FABLE, &huge, rate())), u64::MAX);
    assert_eq!(nano(table().cost(HAIKU, &huge, rate())), u64::MAX);
}

// ---------- the worst case of the next call ----------

#[test]
fn the_worst_case_is_the_page_the_prefix_written_for_an_hour_and_the_largest_answer() {
    // Haiku: 6 000 page tokens, no prefix, 16 000 out.
    let estimate = CallEstimate {
        page_input_tokens: 6_000,
        prefix_tokens: 0,
        max_output_tokens: 16_000,
    };
    assert_eq!(
        nano(table().worst_case(HAIKU, &estimate, rate())),
        10_800_000 + 144_000_000
    );
    // With a 4 000-token prefix that may have to be written for an hour.
    let with_prefix = CallEstimate {
        prefix_tokens: 4_000,
        ..estimate
    };
    assert_eq!(
        nano(table().worst_case(HAIKU, &with_prefix, rate())),
        10_800_000 + 144_000_000 + 4_000 * 3_600
    );
}

#[test]
fn the_worst_case_of_a_long_prompt_uses_the_long_rates() {
    let estimate = CallEstimate {
        page_input_tokens: 100_000,
        prefix_tokens: 1,
        max_output_tokens: 1_000,
    };
    let cost = nano(table().worst_case(HAIKU, &estimate, rate()));
    assert_eq!(cost, 100_000 * 9_000 + 18_000 + 1_000 * 45_000);
}

#[test]
fn the_worst_case_is_never_less_than_what_a_call_that_size_could_cost() {
    // Whatever the mix of cache kinds, a real call of that size costs no more than the estimate.
    let estimate = CallEstimate {
        page_input_tokens: 5_000,
        prefix_tokens: 4_000,
        max_output_tokens: 8_000,
    };
    let worst = table().worst_case(OPUS, &estimate, rate()).unwrap();
    for (input, writes_1h, writes_5m, reads) in [
        (9_000, 0, 0, 0),
        (5_000, 4_000, 0, 0),
        (5_000, 0, 4_000, 0),
        (5_000, 0, 0, 4_000),
    ] {
        let call = TokenUsage {
            input_tokens: input,
            output_tokens: 8_000,
            cache_read_tokens: reads,
            cache_write_tokens: writes_1h + writes_5m,
            cache_write_1h_tokens: writes_1h,
        };
        assert!(table().cost(OPUS, &call, rate()).unwrap() <= worst, "{call:?}");
    }
}

#[test]
fn an_unknown_model_has_no_worst_case_either() {
    let estimate = CallEstimate {
        page_input_tokens: 1,
        prefix_tokens: 1,
        max_output_tokens: 1,
    };
    assert_eq!(
        table().worst_case("claude-nothing", &estimate, rate()),
        Err(Error::UnknownModelPrice("claude-nothing".to_string()))
    );
}

// ---------- which rungs fit a budget ----------

fn typical(prefix: u64) -> (u64, u64) {
    (6_000, prefix)
}

#[test]
fn at_the_default_cap_every_rung_fits_a_typical_page_without_a_cache_write() {
    let policy = RouterPolicy::default_ladder().with_enabled(FABLE, true).unwrap();
    let cap = Cost::from_rand_cents(500);
    let (page, prefix) = typical(0);
    assert_eq!(
        table().unaffordable_models(&policy, page, prefix, rate(), cap),
        Vec::<String>::new()
    );
}

#[test]
fn fable_does_not_fit_r5_once_the_prefix_must_be_written() {
    // 6 000 page tokens at R180 per million is R1.08, the 4 000-token answer R3.60, and a
    // 4 000-token prefix written for an hour at R360 per million is R1.44: R6.12 in all.
    let policy = RouterPolicy::default_ladder().with_enabled(FABLE, true).unwrap();
    let cap = Cost::from_rand_cents(500);
    let (page, prefix) = typical(4_000);
    assert_eq!(
        table().unaffordable_models(&policy, page, prefix, rate(), cap),
        vec![FABLE.to_string()]
    );
    // Opus still fits: R0.43 + R2.88 + R0.58.
    let worst = table()
        .worst_case(
            OPUS,
            &CallEstimate {
                page_input_tokens: 6_000,
                prefix_tokens: 4_000,
                max_output_tokens: 8_000,
            },
            rate(),
        )
        .unwrap();
    assert!(worst < cap);
}

#[test]
fn a_small_cap_leaves_only_the_cheap_rungs() {
    let policy = RouterPolicy::default_ladder();
    let cap = Cost::from_rand_cents(100);
    let (page, prefix) = typical(0);
    assert_eq!(
        table().unaffordable_models(&policy, page, prefix, rate(), cap),
        vec![SONNET.to_string(), OPUS.to_string()]
    );
}

#[test]
fn a_rung_that_is_off_is_not_judged() {
    let policy = RouterPolicy::default_ladder();
    let cap = Cost::from_rand_cents(1);
    let (page, prefix) = typical(0);
    let all = table().unaffordable_models(&policy, page, prefix, rate(), cap);
    assert!(!all.contains(&FABLE.to_string()), "{all:?}");
}

#[test]
fn a_model_with_no_price_never_fits() {
    let table = PriceTable::new(vec![table().get(HAIKU).unwrap().clone()]).unwrap();
    let policy = RouterPolicy::default_ladder();
    let cap = Cost::from_rand_cents(100_000);
    let (page, prefix) = typical(0);
    assert_eq!(
        table.unaffordable_models(&policy, page, prefix, rate(), cap),
        vec![SONNET.to_string(), OPUS.to_string()]
    );
}

#[test]
fn a_rung_fits_when_its_worst_case_equals_the_cap() {
    let policy = RouterPolicy::default_ladder();
    let estimate = CallEstimate {
        page_input_tokens: 6_000,
        prefix_tokens: 0,
        max_output_tokens: 16_000,
    };
    let exact = table().worst_case(HAIKU, &estimate, rate()).unwrap();
    assert!(!table()
        .unaffordable_models(&policy, 6_000, 0, rate(), exact)
        .contains(&HAIKU.to_string()));
    let one_less = Cost::from_nano_rand(exact.nano_rand() - 1);
    assert!(table()
        .unaffordable_models(&policy, 6_000, 0, rate(), one_less)
        .contains(&HAIKU.to_string()));
}

// ---------- building and reading a table ----------

fn price(model: &str) -> ModelPrice {
    ModelPrice {
        model: model.to_string(),
        read_on: "2026-10-22".to_string(),
        standard: Rates::from_base(1_000_000, 5_000_000, 100),
        long_prompt: None,
    }
}

#[test]
fn a_table_is_checked_when_built() {
    assert!(PriceTable::new(vec![price("a")]).is_ok());
    let refuse = |prices: Vec<ModelPrice>, why: &'static str| {
        assert_eq!(PriceTable::new(prices), Err(Error::InvalidPricing(why)));
    };
    refuse(vec![], "a price table needs at least one model");
    refuse(vec![price("a"), price("a")], "a model has two prices");
    refuse(vec![price("")], "a model id is not a plain identifier");
    refuse(vec![price("has space")], "a model id is not a plain identifier");
    let mut free = price("a");
    free.standard.input = 0;
    refuse(vec![free], "a price of zero would make a budget meaningless");
    let mut free_out = price("a");
    free_out.standard.output = 0;
    refuse(vec![free_out], "a price of zero would make a budget meaningless");
    let mut date = price("a");
    date.read_on = "22 October".to_string();
    refuse(vec![date], "the date a price was read is not YYYY-MM-DD");
    let mut long = price("a");
    long.long_prompt = Some(LongPrompt {
        above_tokens: 0,
        rates: Rates::from_base(1, 1, 100),
    });
    refuse(vec![long], "a long-prompt threshold must be above zero");
    let mut cheap_long = price("a");
    cheap_long.long_prompt = Some(LongPrompt {
        above_tokens: 100,
        rates: Rates::from_base(1, 1, 100),
    });
    refuse(vec![cheap_long], "a long-prompt rate is below the standard one");
}

#[test]
fn rates_are_derived_from_the_base_exactly() {
    let r = Rates::from_base(100_000, 500_000, 100);
    assert_eq!(
        (
            r.input,
            r.output,
            r.cache_read,
            r.cache_write_5m,
            r.cache_write_1h
        ),
        (100_000, 500_000, 10_000, 125_000, 200_000)
    );
    let r = Rates::from_base(10_000_000, 50_000_000, 25);
    assert_eq!(
        (r.cache_read, r.cache_write_5m, r.cache_write_1h),
        (250_000, 12_500_000, 20_000_000)
    );
}

#[test]
fn a_table_read_from_text_cannot_skip_the_checks() {
    let json = serde_json::to_string(&table()).unwrap();
    let back: PriceTable = serde_json::from_str(&json).unwrap();
    assert_eq!(back, table());
    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    value["prices"][0]["standard"]["input"] = serde_json::json!(0);
    assert!(serde_json::from_value::<PriceTable>(value).is_err());
    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    value["prices"] = serde_json::json!([]);
    assert!(serde_json::from_value::<PriceTable>(value).is_err());
}

// ---------- the same checks, one defect at a time ----------

#[test]
fn a_date_must_be_exactly_year_month_day_in_digits_and_dashes() {
    for (date, ok) in [
        ("2026-10-22", true),
        ("2026/10/22", false), // separators wrong, digits right
        ("2026-10.22", false), // second separator wrong
        ("2026-1a-22", false), // one letter
        ("2026-10-2x", false),
        ("x026-10-22", false),
        ("2026-10-2", false),   // too short
        ("2026-10-222", false), // too long
        ("", false),
    ] {
        let mut p = price("a");
        p.read_on = date.to_string();
        assert_eq!(PriceTable::new(vec![p]).is_ok(), ok, "{date:?}");
    }
}

#[test]
fn a_long_prompt_may_not_be_cheaper_in_either_direction_but_may_be_equal() {
    let standard = Rates::from_base(1_000_000, 5_000_000, 100);
    let with = |input: u64, output: u64| {
        let mut p = price("a");
        p.long_prompt = Some(LongPrompt {
            above_tokens: 100,
            rates: Rates::from_base(input, output, 100),
        });
        PriceTable::new(vec![p])
    };
    // Equal to the standard rates is allowed; dearer is allowed.
    assert!(with(standard.input, standard.output).is_ok());
    assert!(with(standard.input + 1, standard.output + 1).is_ok());
    // Cheaper in the input only, or in the output only, is not.
    assert!(with(standard.input - 1, standard.output).is_err());
    assert!(with(standard.input, standard.output - 1).is_err());
}

#[test]
fn a_one_hour_write_count_larger_than_all_the_writes_is_cut_to_the_writes() {
    // Reported: 5 tokens written, 9 of them "for an hour". Only 5 can have been.
    let odd = TokenUsage {
        cache_write_tokens: 5,
        cache_write_1h_tokens: 9,
        ..TokenUsage::default()
    };
    // 5 tokens at the one-hour price: 5 x 3 600 nano-rand on Haiku.
    assert_eq!(nano(table().cost(HAIKU, &odd, rate())), 5 * 3_600);
}
