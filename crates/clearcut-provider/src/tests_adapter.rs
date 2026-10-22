// Written by Kone Tshivhinda & Claude
//! The additions the Anthropic adapter needs (ADR-0012 §6): a cache lifetime per
//! request, the one-hour part of cache writes, and two more reasons a call can fail.

use super::*;
use clearcut_core::jobs::FailureKind;
use serde_json::json;
use std::time::Duration;
use uuid::Uuid;

fn minimal() -> RequestBuilder {
    ModelRequest::builder(
        "claude-haiku-5-5",
        CallContext {
            tenant: Uuid::from_u128(1),
            job: None,
            document: None,
            page: None,
            purpose: Purpose::Extraction,
        },
    )
    .content(Block::text("Invoice INV-1"))
    .output_schema(json!({ "type": "object" }))
    .max_output_tokens(2000)
}

#[test]
fn the_cache_lasts_five_minutes_unless_the_request_asks_for_an_hour() {
    assert_eq!(minimal().build().unwrap().cache_ttl(), CacheTtl::FiveMinutes);
    let long = minimal().cache_ttl(CacheTtl::OneHour).build().unwrap();
    assert_eq!(long.cache_ttl(), CacheTtl::OneHour);
    assert_eq!(CacheTtl::FiveMinutes.as_str(), "5m");
    assert_eq!(CacheTtl::OneHour.as_str(), "1h");
    assert!(format!("{long:?}").contains("OneHour"));
}

#[test]
fn a_one_hour_cache_write_is_counted_apart_but_never_twice() {
    let usage = Usage {
        input_tokens: 10,
        output_tokens: 20,
        cache_read_tokens: 30,
        cache_write_tokens: 100,
        cache_write_1h_tokens: 60,
    };
    // The one-hour part is a part of the writes, so the total does not add it again.
    assert_eq!(usage.total(), 10 + 20 + 30 + 100);
    assert_eq!(usage.cache_write_5m_tokens(), 40);
    // A provider that reports more one-hour writes than writes does not make the rest wrap around.
    let odd = Usage {
        cache_write_tokens: 5,
        cache_write_1h_tokens: 9,
        ..Usage::default()
    };
    assert_eq!(odd.cache_write_5m_tokens(), 0);

    let two = usage + usage;
    assert_eq!(two.cache_write_1h_tokens, 120);
    assert_eq!(
        [usage, usage, usage]
            .into_iter()
            .sum::<Usage>()
            .cache_write_1h_tokens,
        180
    );
    let mut running = Usage::default();
    running += usage;
    assert_eq!(running.cache_write_1h_tokens, 60);
    let big = Usage {
        cache_write_1h_tokens: u64::MAX,
        ..Usage::default()
    };
    assert_eq!((big + big).cache_write_1h_tokens, u64::MAX, "saturates");
}

#[test]
fn a_problem_with_the_key_is_not_the_documents_fault_and_never_burns_attempts() {
    for problem in [
        KeyProblem::Rejected,
        KeyProblem::NoPermission,
        KeyProblem::Billing,
        KeyProblem::SpendCap,
        KeyProblem::UserSpendLimit,
    ] {
        let error = ProviderError::KeyProblem {
            problem,
            retry_after: None,
        };
        assert_eq!(
            error.failure_kind(),
            FailureKind::ProviderUnavailable,
            "{problem:?}"
        );
        assert_eq!(error.usage(), None);
        assert_eq!(error.retry_after(), None);
    }
    let wait = Duration::from_secs(86_400);
    let capped = ProviderError::KeyProblem {
        problem: KeyProblem::SpendCap,
        retry_after: Some(wait),
    };
    assert_eq!(capped.retry_after(), Some(wait));
}

#[test]
fn each_key_problem_says_what_is_wrong() {
    for (problem, words) in [
        (KeyProblem::Rejected, "rejected"),
        (KeyProblem::NoPermission, "permission"),
        (KeyProblem::Billing, "billing"),
        (KeyProblem::SpendCap, "monthly"),
        (KeyProblem::UserSpendLimit, "spend limit"),
    ] {
        let error = ProviderError::KeyProblem {
            problem,
            retry_after: None,
        };
        let shown = error.to_string();
        assert!(shown.contains(words), "{shown}");
        assert!(shown.contains("API key") || shown.contains("account"), "{shown}");
    }
}

#[test]
fn a_model_the_key_cannot_use_sends_the_router_to_the_next_rung() {
    let error = ProviderError::ModelUnavailable;
    assert_eq!(error.failure_kind(), FailureKind::NeedsReview);
    assert_eq!(error.usage(), None);
    assert_eq!(error.retry_after(), None);
    assert_eq!(error.to_string(), "the model is not available to this API key");
}
