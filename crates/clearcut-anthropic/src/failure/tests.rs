// Written by Kone Tshivhinda & Claude
use super::*;
use clearcut_core::jobs::FailureKind;
use clearcut_provider::{KeyProblem, ProviderError};
use serde_json::json;
use std::time::Duration;

const NOW: u64 = 1_792_670_400; // 2026-10-22 12:00:00 UTC
const SECRET: &str = "ACCOUNT 1234567890 SECRET";

fn body(kind: &str, message: &str) -> Vec<u8> {
    json!({
        "type": "error",
        "error": { "type": kind, "message": message },
        "request_id": "req_in_body"
    })
    .to_string()
    .into_bytes()
}

fn map(status: u16, retry_after: Option<u64>, body: &[u8]) -> ProviderError {
    map_error(status, retry_after, body, Some("req_abc"), NOW)
}

fn assert_clean(error: &ProviderError) {
    let shown = format!("{error:?} {error}");
    assert!(
        !shown.contains("SECRET") && !shown.contains("1234567890"),
        "{shown}"
    );
}

// ---------- the clock ----------

#[test]
fn seconds_until_the_first_of_next_month() {
    for (now, until) in [
        (1_792_670_400, 1_793_491_200), // 22 Oct 2026 -> 1 Nov 2026
        (1_798_761_599, 1_798_761_600), // 31 Dec 2026 23:59:59 -> 1 Jan 2027
        (1_834_207_200, 1_835_481_600), // 15 Feb 2028 (leap year) -> 1 Mar 2028
        (1_802_217_600, 1_803_859_200), // 10 Feb 2027 -> 1 Mar 2027
        (1_793_491_200, 1_796_083_200), // exactly 1 Nov 2026 00:00:00 -> 1 Dec 2026
        (0, 2_678_400),                 // the epoch -> 1 Feb 1970
    ] {
        assert_eq!(seconds_until_next_month(now), until - now, "now = {now}");
    }
}

#[test]
fn the_wait_is_always_between_one_second_and_thirty_one_days() {
    let mut now = 1_700_000_000;
    while now < 1_900_000_000 {
        let wait = seconds_until_next_month(now);
        assert!((1..=31 * 86_400).contains(&wait), "now = {now}, wait = {wait}");
        now += 86_399; // not a whole day, so every hour of the day is visited
    }
}

// ---------- the key and the account ----------

#[test]
fn a_rate_limit_with_a_retry_after_is_unavailable_for_that_long() {
    let error = map(429, Some(17), &body("rate_limit_error", SECRET));
    match &error {
        ProviderError::Unavailable { retry_after } => assert_eq!(*retry_after, Some(Duration::from_secs(17))),
        other => panic!("wrong error {other:?}"),
    }
    assert_eq!(error.failure_kind(), FailureKind::ProviderUnavailable);
    assert_clean(&error);
}

#[test]
fn a_rate_limit_without_a_retry_after_is_unavailable_without_a_time() {
    let error = map(429, None, &body("rate_limit_error", SECRET));
    assert!(
        matches!(error, ProviderError::Unavailable { retry_after: None }),
        "{error:?}"
    );
}

#[test]
fn an_absurd_retry_after_is_capped_at_a_day() {
    let error = map(429, Some(10_000_000), &body("rate_limit_error", ""));
    assert_eq!(error.retry_after(), Some(Duration::from_secs(86_400)));
}

#[test]
fn the_monthly_spend_cap_is_not_a_rate_limit() {
    let spend = json!({
        "type": "error",
        "error": {
            "type": "rate_limit_error",
            "message": SECRET,
            "details": { "error_code": "enforced_spend_limit_reached" }
        }
    })
    .to_string()
    .into_bytes();
    // With or without a retry-after, it is the cap, and it waits for the month to turn.
    for retry_after in [None, Some(30)] {
        let error = map(429, retry_after, &spend);
        match &error {
            ProviderError::KeyProblem { problem, retry_after } => {
                assert_eq!(*problem, KeyProblem::SpendCap);
                assert_eq!(*retry_after, Some(Duration::from_secs(1_793_491_200 - NOW)));
            }
            other => panic!("wrong error {other:?}"),
        }
        assert_eq!(error.failure_kind(), FailureKind::ProviderUnavailable);
        assert_clean(&error);
    }
}

#[test]
fn the_owners_own_spend_limit_is_a_key_problem_with_no_time() {
    for message in [
        "You have reached your specified API usage limits. You will regain access on 2026-11-01.",
        "you have REACHED YOUR SPECIFIED API USAGE LIMITS",
    ] {
        let error = map(400, None, &body("invalid_request_error", message));
        assert!(
            matches!(
                error,
                ProviderError::KeyProblem {
                    problem: KeyProblem::UserSpendLimit,
                    retry_after: None
                }
            ),
            "{error:?}"
        );
    }
}

#[test]
fn a_key_that_is_rejected_is_never_retried() {
    let error = map(401, None, &body("authentication_error", SECRET));
    assert!(matches!(
        error,
        ProviderError::KeyProblem {
            problem: KeyProblem::Rejected,
            retry_after: None
        }
    ));
    assert_eq!(error.failure_kind(), FailureKind::ProviderUnavailable);
    assert_clean(&error);
}

#[test]
fn a_billing_problem_is_a_key_problem() {
    let error = map(402, None, &body("billing_error", SECRET));
    assert!(matches!(
        error,
        ProviderError::KeyProblem {
            problem: KeyProblem::Billing,
            retry_after: None
        }
    ));
    assert_clean(&error);
}

// ---------- the model ----------

#[test]
fn a_model_that_is_not_found_or_not_allowed_sends_the_router_on() {
    for (status, kind, message) in [
        (404, "not_found_error", "model: claude-nope"),
        (
            403,
            "permission_error",
            "Your API key does not have access to the model claude-opus-5-5",
        ),
        (
            400,
            "invalid_request_error",
            "This model is a Covered Model and needs 30-day data retention for your organization",
        ),
    ] {
        let error = map(status, None, &body(kind, message));
        assert!(
            matches!(error, ProviderError::ModelUnavailable),
            "{status}: {error:?}"
        );
        assert_eq!(error.failure_kind(), FailureKind::NeedsReview);
    }
}

#[test]
fn a_403_that_is_not_about_a_model_is_a_permission_problem() {
    let error = map(403, None, &body("permission_error", "This key may not do that."));
    assert!(matches!(
        error,
        ProviderError::KeyProblem {
            problem: KeyProblem::NoPermission,
            retry_after: None
        }
    ));
}

// ---------- our own mistakes ----------

#[test]
fn a_bad_request_is_our_bug_and_says_only_fixed_facts() {
    for (status, kind) in [
        (400, "invalid_request_error"),
        (413, "request_too_large"),
        (422, "invalid_request_error"),
        (405, "something_else"),
    ] {
        let error = map(status, None, &body(kind, SECRET));
        match &error {
            ProviderError::Rejected(reason) => {
                assert!(reason.contains(&status.to_string()), "{reason}");
                assert!(reason.contains("req_abc"), "{reason}");
            }
            other => panic!("{status}: wrong error {other:?}"),
        }
        assert_eq!(error.failure_kind(), FailureKind::NeedsReview);
        assert_clean(&error);
    }
}

#[test]
fn the_error_type_is_named_only_if_it_is_one_we_know() {
    let known = map(400, None, &body("invalid_request_error", ""));
    assert!(known.to_string().contains("invalid_request_error"), "{known}");
    let invented = map(400, None, &body("a type that quotes SECRET text", ""));
    assert_clean(&invented);
    assert!(invented.to_string().contains("unknown"), "{invented}");
}

#[test]
fn a_body_that_is_not_an_error_message_still_maps_by_status() {
    for junk in [&b""[..], b"<html>SECRET</html>", b"null", b"{\"error\": 5}"] {
        let error = map(400, None, junk);
        assert!(matches!(error, ProviderError::Rejected(_)), "{error:?}");
        assert_clean(&error);
        let unauthorised = map(401, None, junk);
        assert!(matches!(
            unauthorised,
            ProviderError::KeyProblem {
                problem: KeyProblem::Rejected,
                ..
            }
        ));
    }
}

#[test]
fn a_missing_request_id_is_simply_left_out() {
    let error = map_error(400, None, &body("invalid_request_error", ""), None, NOW);
    assert!(!error.to_string().contains("request id"), "{error}");
}

// ---------- the provider's own trouble ----------

#[test]
fn server_errors_and_overload_are_unavailable() {
    for status in [500, 502, 503, 504, 529, 599] {
        let error = map(status, None, &body("api_error", SECRET));
        assert!(
            matches!(error, ProviderError::Unavailable { retry_after: None }),
            "{status}: {error:?}"
        );
        assert_eq!(error.failure_kind(), FailureKind::ProviderUnavailable);
        assert_clean(&error);
    }
}

#[test]
fn a_server_error_passes_on_a_retry_after_if_it_sent_one() {
    let error = map(503, Some(12), &body("api_error", ""));
    assert_eq!(error.retry_after(), Some(Duration::from_secs(12)));
}

#[test]
fn a_redirect_or_a_surprise_status_is_rejected_not_followed() {
    for status in [301, 302, 307, 204, 100] {
        let error = map(status, None, b"");
        assert!(matches!(error, ProviderError::Rejected(_)), "{status}: {error:?}");
    }
}

// ---------- transport ----------

#[test]
fn transport_failures_map_to_fixed_errors() {
    use crate::http::TransportError;
    assert!(matches!(
        from_transport(TransportError::Timeout),
        ProviderError::Timeout
    ));
    // Nothing was sent: free, and worth a quick retry.
    assert!(matches!(
        from_transport(TransportError::Unreachable),
        ProviderError::Unavailable { retry_after: None }
    ));
    // The connection broke after the request was sent: it may have been processed and billed,
    // so it is a timeout (no answer, outcome unknown), never "unavailable" (refused, free), and
    // never retried inline where the budget cannot see it.
    assert!(matches!(
        from_transport(TransportError::Interrupted),
        ProviderError::Timeout
    ));
    assert!(matches!(
        from_transport(TransportError::TooLarge { max_bytes: 5 }),
        ProviderError::InvalidOutput { .. }
    ));
    assert!(matches!(
        from_transport(TransportError::Client),
        ProviderError::Rejected(_)
    ));
}

// ---------- what may be retried at once ----------

#[test]
fn only_outages_and_short_rate_limits_are_retried_inline() {
    let sec = Duration::from_secs;
    // 5xx, overload and connection trouble: wait a little and try again.
    for status in [Some(500), Some(502), Some(503), Some(504), Some(529), None] {
        let error = ProviderError::Unavailable { retry_after: None };
        assert_eq!(inline_retry(status, &error), Retry::Backoff, "{status:?}");
    }
    // A rate limit that says how long: wait exactly that, if it is a minute or less.
    let limited = |s| ProviderError::Unavailable {
        retry_after: Some(sec(s)),
    };
    assert_eq!(inline_retry(Some(429), &limited(1)), Retry::After(sec(1)));
    assert_eq!(inline_retry(Some(429), &limited(60)), Retry::After(sec(60)));
    assert_eq!(inline_retry(Some(429), &limited(61)), Retry::No);
    assert_eq!(inline_retry(Some(503), &limited(5)), Retry::After(sec(5)));
    // A 429 that says nothing is not guessed at: the queue's policy decides.
    assert_eq!(
        inline_retry(Some(429), &ProviderError::Unavailable { retry_after: None }),
        Retry::No
    );
    // Nothing else is ever retried here.
    for never in [
        ProviderError::Timeout,
        ProviderError::Auth,
        ProviderError::ModelUnavailable,
        ProviderError::Rejected(String::new()),
        ProviderError::KeyProblem {
            problem: KeyProblem::SpendCap,
            retry_after: Some(sec(5)),
        },
        ProviderError::KeyProblem {
            problem: KeyProblem::Rejected,
            retry_after: None,
        },
    ] {
        assert_eq!(inline_retry(Some(500), &never), Retry::No, "{never:?}");
    }
}

#[test]
fn either_wording_of_a_covered_model_refusal_is_recognised() {
    for message in [
        "Fable 5.1 is a Covered Model.",
        "This request needs 30-day data retention, which your organization does not allow.",
    ] {
        let error = map(400, None, &body("invalid_request_error", message));
        assert!(
            matches!(error, ProviderError::ModelUnavailable),
            "{message}: {error:?}"
        );
    }
}

#[test]
fn december_rolls_into_january_of_the_next_year() {
    // 15 Dec 2026 12:00 UTC -> 1 Jan 2027 00:00 UTC.
    assert_eq!(
        seconds_until_next_month(1_797_336_000),
        1_798_761_600 - 1_797_336_000
    );
}
