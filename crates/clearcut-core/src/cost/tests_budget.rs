// Written by Kone Tshivhinda & Claude
use super::*;
use crate::Error;

fn rand_cents(cents: u64) -> Cost {
    Cost::from_rand_cents(cents)
}

fn default_check(pages: u32, month_spent: u64) -> BudgetCheck {
    BudgetCheck::new(&BudgetPolicy::default(), pages, rand_cents(month_spent))
}

// ---------- the defaults (ADR-0011, with ADR-0018's cap that grows with the pages) ----------

#[test]
fn the_defaults_are_five_rand_a_document_fifty_cents_a_page_and_a_hundred_a_month() {
    let policy = BudgetPolicy::default();
    assert_eq!(policy.per_document(), rand_cents(500));
    assert_eq!(policy.per_page(), rand_cents(50));
    assert_eq!(policy.per_month(), rand_cents(10_000));
}

#[test]
fn the_document_cap_is_the_larger_of_the_flat_amount_and_so_much_a_page() {
    let policy = BudgetPolicy::default();
    assert_eq!(policy.document_cap(0), rand_cents(500));
    assert_eq!(policy.document_cap(1), rand_cents(500));
    assert_eq!(
        policy.document_cap(10),
        rand_cents(500),
        "ten pages is exactly R5.00"
    );
    assert_eq!(policy.document_cap(11), rand_cents(550));
    assert_eq!(policy.document_cap(300), rand_cents(15_000));
}

#[test]
fn a_huge_page_count_saturates_the_cap_instead_of_wrapping() {
    let policy = BudgetPolicy::new(
        rand_cents(500),
        Cost::from_nano_rand(u64::MAX),
        rand_cents(10_000),
    )
    .unwrap();
    assert_eq!(policy.document_cap(3), Cost::from_nano_rand(u64::MAX));
}

// ---------- building a policy ----------

#[test]
fn every_amount_must_be_above_zero() {
    assert!(BudgetPolicy::new(rand_cents(1), rand_cents(1), rand_cents(1)).is_ok());
    for (document, page, month) in [(0, 1, 1), (1, 0, 1), (1, 1, 0)] {
        assert_eq!(
            BudgetPolicy::new(rand_cents(document), rand_cents(page), rand_cents(month)),
            Err(Error::InvalidPricing("a budget amount must be above zero")),
            "{document} {page} {month}"
        );
    }
}

#[test]
fn a_policy_read_from_text_is_checked_like_one_built() {
    let json = serde_json::to_string(&BudgetPolicy::default()).unwrap();
    let back: BudgetPolicy = serde_json::from_str(&json).unwrap();
    assert_eq!(back, BudgetPolicy::default());
    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    value["per_month"] = serde_json::json!(0);
    assert!(serde_json::from_value::<BudgetPolicy>(value).is_err());
}

// ---------- spending within the document's cap ----------

#[test]
fn a_call_that_fits_the_document_is_allowed_and_one_that_does_not_is_refused() {
    let mut check = default_check(1, 0);
    assert_eq!(check.document_remaining(), rand_cents(500));
    assert_eq!(check.check(rand_cents(490)), Ok(()));
    check.record(rand_cents(490));
    assert_eq!(check.document_spent(), rand_cents(490));
    assert_eq!(check.document_remaining(), rand_cents(10));
    // Exactly what is left is allowed; one nano-rand more is not.
    assert_eq!(check.check(rand_cents(10)), Ok(()));
    assert_eq!(
        check.check(Cost::from_nano_rand(rand_cents(10).nano_rand() + 1)),
        Err(BudgetScope::Document)
    );
}

#[test]
fn what_was_really_spent_is_what_counts_not_the_estimate() {
    let mut check = default_check(1, 0);
    // The worst case of a call was R4.90; it really cost R0.02.
    assert_eq!(check.check(rand_cents(490)), Ok(()));
    check.record(rand_cents(2));
    assert_eq!(check.document_remaining(), rand_cents(498));
    assert_eq!(check.check(rand_cents(490)), Ok(()));
}

#[test]
fn a_document_of_many_pages_has_the_larger_cap() {
    let check = default_check(300, 0);
    // The document's own allowance; the month is a separate limit (below).
    assert_eq!(check.document_remaining(), rand_cents(15_000));
    assert_eq!(default_check(11, 0).check(rand_cents(550)), Ok(()));
    assert_eq!(
        default_check(11, 0).check(rand_cents(551)),
        Err(BudgetScope::Document)
    );
}

// ---------- the month ----------

#[test]
fn the_month_is_checked_after_the_document() {
    // R99.95 already spent this month: a call of R0.05 is the last that fits.
    let check = default_check(1, 9_995);
    assert_eq!(check.month_remaining(), rand_cents(5));
    assert_eq!(check.check(rand_cents(5)), Ok(()));
    assert_eq!(check.check(rand_cents(6)), Err(BudgetScope::Month));
}

#[test]
fn the_month_includes_what_this_document_has_spent() {
    let mut check = default_check(300, 9_000);
    check.record(rand_cents(900));
    assert_eq!(check.month_remaining(), rand_cents(100));
    assert_eq!(check.check(rand_cents(100)), Ok(()));
    assert_eq!(check.check(rand_cents(101)), Err(BudgetScope::Month));
}

#[test]
fn a_cap_larger_than_the_month_never_lets_a_document_past_the_month() {
    // 300 pages allow R150 on paper, but the month has R100.
    let check = default_check(300, 0);
    assert_eq!(check.check(rand_cents(10_000)), Ok(()));
    assert_eq!(check.check(rand_cents(10_001)), Err(BudgetScope::Month));
}

#[test]
fn when_both_are_over_the_document_is_named() {
    let check = default_check(1, 9_999);
    assert_eq!(check.check(rand_cents(501)), Err(BudgetScope::Document));
}

#[test]
fn a_month_already_overspent_allows_nothing() {
    let check = default_check(1, 20_000);
    assert_eq!(check.month_remaining(), Cost::ZERO);
    assert_eq!(check.check(Cost::from_nano_rand(1)), Err(BudgetScope::Month));
}

#[test]
fn spending_saturates_instead_of_wrapping() {
    let mut check = default_check(1, 0);
    check.record(Cost::from_nano_rand(u64::MAX));
    check.record(Cost::from_nano_rand(u64::MAX));
    assert_eq!(check.document_spent(), Cost::from_nano_rand(u64::MAX));
    assert_eq!(check.check(Cost::from_nano_rand(1)), Err(BudgetScope::Document));
    assert_eq!(check.document_remaining(), Cost::ZERO);
}

#[test]
fn the_scopes_have_fixed_names() {
    assert_eq!(BudgetScope::Document.as_str(), "document");
    assert_eq!(BudgetScope::Month.as_str(), "month");
}

#[test]
fn a_check_survives_being_stored_between_two_calls() {
    let mut check = default_check(20, 1_234);
    check.record(rand_cents(77));
    let back: BudgetCheck = serde_json::from_str(&serde_json::to_string(&check).unwrap()).unwrap();
    assert_eq!(back, check);
}

#[test]
fn each_recorded_call_adds_to_what_the_document_has_spent() {
    let mut check = default_check(1, 0);
    check.record(rand_cents(100));
    check.record(rand_cents(200));
    assert_eq!(check.document_spent(), rand_cents(300));
    check.record(rand_cents(50));
    assert_eq!(check.document_spent(), rand_cents(350));
    assert_eq!(check.document_remaining(), rand_cents(150));
}
