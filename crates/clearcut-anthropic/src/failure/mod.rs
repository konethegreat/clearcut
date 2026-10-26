// Written by Kone Tshivhinda & Claude
//! What went wrong, in `ProviderError` terms (ADR-0012 §3).
//!
//! Every non-200 answer and every transport failure becomes one exact
//! [`ProviderError`]. The mapping reads the error body only to **classify** it
//! (which of several 400s or 429s it is); nothing from the body is ever copied
//! into the error, because Anthropic's message may quote our request, and our
//! request holds an invoice. The only free-form facts that appear are the status
//! number, the error type if it is one we know, and the sanitised request id.

use crate::http::TransportError;
use clearcut_provider::{KeyProblem, ProviderError};
use serde_json::Value;
use std::time::Duration;

/// A `retry-after` longer than a day is not believed.
const MAX_RETRY_AFTER_SECS: u64 = 86_400;
/// A rate limit this short is waited out inside the call; longer goes back to the job queue.
const MAX_INLINE_WAIT: Duration = Duration::from_secs(60);

/// The error types Anthropic documents (research 06). Anything else is "unknown".
const KNOWN_ERROR_TYPES: [&str; 10] = [
    "invalid_request_error",
    "authentication_error",
    "billing_error",
    "permission_error",
    "not_found_error",
    "request_too_large",
    "rate_limit_error",
    "api_error",
    "timeout_error",
    "overloaded_error",
];

/// Maps a non-200 answer. `now_unix` is the current time in seconds since 1970,
/// used only to say when a monthly cap resets.
pub fn map_error(
    status: u16,
    retry_after: Option<u64>,
    body: &[u8],
    request_id: Option<&str>,
    now_unix: u64,
) -> ProviderError {
    let error = parse_error_body(body);
    let wait = retry_after.map(|s| Duration::from_secs(s.min(MAX_RETRY_AFTER_SECS)));

    match status {
        429 if error.code.as_deref() == Some("enforced_spend_limit_reached") => ProviderError::KeyProblem {
            problem: KeyProblem::SpendCap,
            retry_after: Some(Duration::from_secs(seconds_until_next_month(now_unix))),
        },
        429 => ProviderError::Unavailable { retry_after: wait },
        400 if error.message_has("specified api usage limits") => key_problem(KeyProblem::UserSpendLimit),
        400 if error.message_has("covered model") || error.message_has("data retention") => {
            ProviderError::ModelUnavailable
        }
        401 => key_problem(KeyProblem::Rejected),
        402 => key_problem(KeyProblem::Billing),
        403 if error.message_has("model") => ProviderError::ModelUnavailable,
        403 => key_problem(KeyProblem::NoPermission),
        404 => ProviderError::ModelUnavailable,
        500..=599 => ProviderError::Unavailable { retry_after: wait },
        _ => ProviderError::Rejected(rejection(status, error.kind(), request_id)),
    }
}

/// What a transport failure means.
pub fn from_transport(failure: TransportError) -> ProviderError {
    match failure {
        // No answer after the request was sent: a deadline passed, or the connection broke
        // part-way. Either way it may have been processed and billed.
        TransportError::Timeout | TransportError::Interrupted => ProviderError::Timeout,
        // Nothing reached the provider.
        TransportError::Unreachable => ProviderError::Unavailable { retry_after: None },
        TransportError::TooLarge { .. } => ProviderError::InvalidOutput {
            usage: Default::default(),
            reason: "the answer was larger than the size that is read".to_string(),
        },
        TransportError::Client => ProviderError::Rejected("the HTTP client could not be used".to_string()),
    }
}

fn key_problem(problem: KeyProblem) -> ProviderError {
    ProviderError::KeyProblem {
        problem,
        retry_after: None,
    }
}

fn rejection(status: u16, kind: &str, request_id: Option<&str>) -> String {
    let mut reason = format!("the API refused the request (status {status}, {kind})");
    if let Some(id) = request_id {
        reason.push_str(&format!(", request id {id}"));
    }
    reason
}

/// What was read from an error body, for classification only.
struct ErrorBody {
    kind: Option<String>,
    code: Option<String>,
    message: String,
}

impl ErrorBody {
    /// The error type, if it is one the documentation lists; otherwise `unknown`.
    fn kind(&self) -> &str {
        match &self.kind {
            Some(kind) if KNOWN_ERROR_TYPES.contains(&kind.as_str()) => kind,
            _ => "unknown",
        }
    }

    fn message_has(&self, words: &str) -> bool {
        self.message.contains(words)
    }
}

fn parse_error_body(body: &[u8]) -> ErrorBody {
    let error = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| value.get("error").cloned());
    let text = |value: Option<&Value>| value.and_then(Value::as_str).map(str::to_string);
    ErrorBody {
        kind: text(error.as_ref().and_then(|e| e.get("type"))),
        code: text(
            error
                .as_ref()
                .and_then(|e| e.get("details"))
                .and_then(|d| d.get("error_code")),
        ),
        // Lower-cased so wording changes in capitalisation do not matter.
        message: text(error.as_ref().and_then(|e| e.get("message")))
            .unwrap_or_default()
            .to_lowercase(),
    }
}

/// How many seconds from `now_unix` until 00:00 UTC on the first day of the next
/// month, when a monthly spend cap resets. Always at least one second.
pub fn seconds_until_next_month(now_unix: u64) -> u64 {
    let days = (now_unix / 86_400) as i64;
    let (year, month, _) = civil_from_days(days);
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let target = days_from_civil(next_year, next_month, 1) as u64 * 86_400;
    target - now_unix
}

// Howard Hinnant's civil-calendar algorithms (public domain): days since
// 1970-01-01 to and from a (year, month, day) in the proleptic Gregorian calendar.

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let year_of_era = y - era * 400;
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// What to do about a failed attempt without going back to the job queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retry {
    /// Give up here; the queue's own policy (ADR-0008) takes over.
    No,
    /// Wait this long (the provider said how long), then try again.
    After(Duration),
    /// Wait a short, growing, randomised time, then try again.
    Backoff,
}

/// Only an outage (5xx, overload, connection trouble) or a rate limit that says
/// it will clear within a minute is worth a quick retry. Everything else, above
/// all a key problem, is not: retrying cannot fix it, and the job queue already
/// has a policy, so retries are not multiplied (ADR-0012 §5).
pub fn inline_retry(status: Option<u16>, error: &ProviderError) -> Retry {
    match error {
        ProviderError::Unavailable {
            retry_after: Some(wait),
        } if *wait <= MAX_INLINE_WAIT => Retry::After(*wait),
        ProviderError::Unavailable { retry_after: Some(_) } => Retry::No,
        ProviderError::Unavailable { retry_after: None } => match status {
            Some(status) if status < 500 => Retry::No,
            _ => Retry::Backoff,
        },
        _ => Retry::No,
    }
}

#[cfg(test)]
mod tests;
