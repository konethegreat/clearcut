// Written by Kone Tshivhinda & Claude
//! [`AnthropicProvider`]: the Anthropic adapter behind the `ModelProvider` trait
//! (ADR-0012).
//!
//! One call does this, and nothing more:
//! 1. build the request body ([`crate::request`]) and refuse early, in our own
//!    code, anything the API would reject;
//! 2. POST it with the user's key, giving the request the time its answer size
//!    deserves;
//! 3. read the answer ([`crate::response`]) or map the failure
//!    ([`crate::failure`]) to the exact `ProviderError`;
//! 4. retry **inline** only what is worth retrying at once: an outage, or a rate
//!    limit that says it clears within a minute. At most twice, and never more
//!    than two minutes of waiting in total, so these retries do not multiply
//!    with the job queue's own (ADR-0008).
//!
//! It does **not** choose models, count money, or limit concurrency: those are
//! the router (P2-14), the cost guard (P2-12) and the rate limiter (P2-24).
//!
//! Waiting, randomness and the clock sit behind [`Pacing`], so tests record the
//! waits instead of sleeping and every number in them is exact.

use crate::config::{AnthropicConfig, ConfigError};
use crate::failure::{self, Retry};
use crate::http::AnthropicHttp;
use crate::key::ApiKey;
use crate::{request, response};
use bytes::Bytes;
use clearcut_provider::{ModelProvider, ModelRequest, ModelResponse, ProviderError};
use std::collections::hash_map::RandomState;
use std::fmt;
use std::future::Future;
use std::hash::{BuildHasher, Hasher};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The most of an answer that is read (ADR-0012 §5).
pub const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
/// After the first try, at most this many more.
const MAX_RETRIES: u32 = 2;
/// The most time spent waiting between tries, all together.
const TOTAL_WAIT_BUDGET: Duration = Duration::from_secs(120);
/// The first backoff; it doubles each time.
const BACKOFF_BASE: Duration = Duration::from_secs(1);
/// A random extra wait of up to this much, so many callers do not retry in step.
const JITTER_WINDOW: Duration = Duration::from_millis(500);

/// Waiting, randomness and the clock.
pub trait Pacing: Send + Sync + 'static {
    fn pause(&self, wait: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
    /// A random duration from zero up to (not including) `up_to`.
    fn jitter(&self, up_to: Duration) -> Duration;
    /// Seconds since 1970-01-01 UTC.
    fn now_unix(&self) -> u64;
}

/// The real thing.
struct RealPacing;

impl Pacing for RealPacing {
    fn pause(&self, wait: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(tokio::time::sleep(wait))
    }

    fn jitter(&self, up_to: Duration) -> Duration {
        // `RandomState` is seeded randomly by the operating system, which is all
        // the randomness a retry delay needs (no extra dependency).
        let random = RandomState::new().build_hasher().finish();
        let window = u64::try_from(up_to.as_nanos()).unwrap_or(u64::MAX).max(1);
        Duration::from_nanos(random % window)
    }

    fn now_unix(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs())
    }
}

/// How long one request may take: a base plus time for the tokens it may produce.
/// The default is 30 seconds plus one second per 40 tokens (16 000 tokens: about
/// seven minutes). The connect timeout stays short (see [`crate::http`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeoutPolicy {
    pub base: Duration,
    pub tokens_per_second: u32,
}

impl Default for TimeoutPolicy {
    fn default() -> Self {
        TimeoutPolicy {
            base: Duration::from_secs(30),
            tokens_per_second: 40,
        }
    }
}

impl TimeoutPolicy {
    pub fn for_tokens(&self, max_output_tokens: u32) -> Duration {
        let seconds = u64::from(max_output_tokens) / u64::from(self.tokens_per_second.max(1));
        self.base.saturating_add(Duration::from_secs(seconds))
    }
}

/// The Anthropic adapter. Cheap to clone; clones share the connection pool.
#[derive(Clone)]
pub struct AnthropicProvider {
    http: AnthropicHttp,
    pacing: Arc<dyn Pacing>,
    timeouts: TimeoutPolicy,
}

impl fmt::Debug for AnthropicProvider {
    /// The key is never shown (the HTTP client prints it as `[redacted]`).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnthropicProvider")
            .field("http", &self.http)
            .field("timeouts", &self.timeouts)
            .finish()
    }
}

impl AnthropicProvider {
    pub fn new(config: &AnthropicConfig, key: &ApiKey) -> Result<AnthropicProvider, ConfigError> {
        Ok(AnthropicProvider {
            http: AnthropicHttp::new(config, key)?,
            pacing: Arc::new(RealPacing),
            timeouts: TimeoutPolicy::default(),
        })
    }

    /// Replaces how waiting, randomness and the clock work (for tests).
    pub fn with_pacing(mut self, pacing: Arc<dyn Pacing>) -> AnthropicProvider {
        self.pacing = pacing;
        self
    }

    pub fn with_timeouts(mut self, timeouts: TimeoutPolicy) -> AnthropicProvider {
        self.timeouts = timeouts;
        self
    }

    async fn call(&self, model_request: &ModelRequest) -> Result<ModelResponse, ProviderError> {
        let built = request::build(model_request)?;
        let body = serde_json::to_vec(&built.body)
            .map(Bytes::from)
            .map_err(|_| ProviderError::Rejected("the request could not be encoded".to_string()))?;
        let timeout = self.timeouts.for_tokens(model_request.max_output_tokens());

        let mut attempt: u32 = 0;
        let mut waited = Duration::ZERO;
        loop {
            let (status, error) = match self
                .http
                .post_json("/v1/messages", body.clone(), MAX_RESPONSE_BYTES, timeout)
                .await
            {
                Ok(reply) if reply.status == 200 => {
                    return response::parse(&reply.body, model_request.model(), reply.request_id);
                }
                Ok(reply) => (
                    Some(reply.status),
                    failure::map_error(
                        reply.status,
                        reply.retry_after,
                        &reply.body,
                        reply.request_id.as_deref(),
                        self.pacing.now_unix(),
                    ),
                ),
                Err(transport) => (None, failure::from_transport(transport)),
            };

            let wait = match failure::inline_retry(status, &error) {
                Retry::No => return Err(error),
                Retry::After(wait) => wait,
                Retry::Backoff => {
                    // 1 s, then 2 s, plus a little random extra.
                    BACKOFF_BASE * 2u32.pow(attempt) + self.pacing.jitter(JITTER_WINDOW)
                }
            };
            if attempt >= MAX_RETRIES || waited + wait > TOTAL_WAIT_BUDGET {
                return Err(error);
            }
            waited += wait;
            attempt += 1;
            self.pacing.pause(wait).await;
        }
    }
}

impl ModelProvider for AnthropicProvider {
    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        self.call(&request).await
    }
}

#[cfg(test)]
mod tests;
