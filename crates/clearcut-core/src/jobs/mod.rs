// Written by Kone Tshivhinda & Claude
//! The rules of the background job queue (ADR-0008).
//!
//! This module is pure: no database, no clock, no randomness. It says what the
//! states are and which moves between them are legal, how a finished job
//! changes its document, how long to wait before a retry, how long a lease
//! lasts, and how to make text safe for the database. `clearcut-store` applies
//! these rules to Postgres, and has a test that the SQL functions agree with
//! the tables here.
//!
//! The shape of a job's life:
//!
//! ```text
//! queued ──claim──▶ running ──▶ succeeded
//!   │  ▲              │ │ └───▶ needs_review   (a person must look)
//!   │  └──retry/defer─┘ └─────▶ cancelled
//!   ├──cancel──────────────────▶ cancelled
//!   └──deadline passed─────────▶ needs_review
//! ```
//!
//! There is no `failed` state. A job that cannot be completed ends
//! `needs_review`, and so does its document (ADR-0005).

use crate::document::DocumentStatus;
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// What a job does. The names are stored in the database. New kinds are added
/// here, with no migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    /// Read, validate and turn one document into events (page progress is
    /// tracked inside the one job).
    ProcessDocument,
}

impl JobKind {
    pub const ALL: [JobKind; 1] = [JobKind::ProcessDocument];

    pub fn as_str(self) -> &'static str {
        match self {
            JobKind::ProcessDocument => "process_document",
        }
    }

    /// Parses exactly one kind: no trimming, no case folding.
    pub fn parse(text: &str) -> Result<JobKind> {
        JobKind::ALL
            .into_iter()
            .find(|kind| kind.as_str() == text)
            .ok_or_else(|| Error::UnknownJobKind(text.to_string()))
    }
}

/// Where a job is. The names are stored in the database.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    /// Waiting for a worker (or for its retry time).
    Queued,
    /// A worker holds a lease on it.
    Running,
    /// Finished: the document is done.
    Succeeded,
    /// Finished: a person must look at the document.
    NeedsReview,
    /// Finished: someone asked for it to stop.
    Cancelled,
}

impl JobState {
    pub const ALL: [JobState; 5] = [
        JobState::Queued,
        JobState::Running,
        JobState::Succeeded,
        JobState::NeedsReview,
        JobState::Cancelled,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            JobState::Queued => "queued",
            JobState::Running => "running",
            JobState::Succeeded => "succeeded",
            JobState::NeedsReview => "needs_review",
            JobState::Cancelled => "cancelled",
        }
    }

    /// Parses exactly one state: no trimming, no case folding.
    pub fn parse(text: &str) -> Result<JobState> {
        JobState::ALL
            .into_iter()
            .find(|state| state.as_str() == text)
            .ok_or_else(|| Error::UnknownJobState(text.to_string()))
    }

    /// A finished job never moves again.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            JobState::Succeeded | JobState::NeedsReview | JobState::Cancelled
        )
    }

    /// The legal moves: a claim, a retry or deferral, a fresh claim after a
    /// lost lease, finishing, cancelling before it runs, and the deadline
    /// sweep. Nothing moves out of a terminal state.
    pub fn can_move_to(self, next: JobState) -> bool {
        use JobState::*;
        matches!(
            (self, next),
            (Queued, Running)
                | (Queued, Cancelled)
                | (Queued, NeedsReview)
                | (Running, Running)
                | (Running, Queued)
                | (Running, Succeeded)
                | (Running, NeedsReview)
                | (Running, Cancelled)
        )
    }

    /// The move, or an error if it is not legal.
    pub fn transition(self, next: JobState) -> Result<JobState> {
        if self.can_move_to(next) {
            Ok(next)
        } else {
            Err(Error::InvalidJobTransition { from: self, to: next })
        }
    }

    /// What the document's status becomes when its job enters this state.
    /// `None` for `Queued`: waiting changes nothing.
    ///
    /// A cancelled job returns its document to `Received`, so it can be
    /// processed again later.
    pub fn document_status(self) -> Option<DocumentStatus> {
        match self {
            JobState::Queued => None,
            JobState::Running => Some(DocumentStatus::Processing),
            JobState::Succeeded => Some(DocumentStatus::Done),
            JobState::NeedsReview => Some(DocumentStatus::NeedsReview),
            JobState::Cancelled => Some(DocumentStatus::Received),
        }
    }
}

/// How a worker finishes a job it holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Succeeded,
    NeedsReview,
    /// Acknowledging a cancellation request.
    Cancelled,
}

impl Outcome {
    pub fn state(self) -> JobState {
        match self {
            Outcome::Succeeded => JobState::Succeeded,
            Outcome::NeedsReview => JobState::NeedsReview,
            Outcome::Cancelled => JobState::Cancelled,
        }
    }

    pub fn as_str(self) -> &'static str {
        self.state().as_str()
    }

    pub fn document_status(self) -> DocumentStatus {
        match self {
            Outcome::Succeeded => DocumentStatus::Done,
            Outcome::NeedsReview => DocumentStatus::NeedsReview,
            Outcome::Cancelled => DocumentStatus::Received,
        }
    }
}

/// Why a run failed, as far as the queue is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FailureKind {
    /// A hiccup (timeout, server error, bad output that a retry may fix).
    /// Retried with backoff; counts against the attempts.
    Transient,
    /// The model provider is down or throttling us. Retried later; does **not**
    /// count, so an outage cannot use up a job's attempts. Only the deadline
    /// ends such a job.
    ProviderUnavailable,
    /// Even the top model tier could not produce valid output. No retry will
    /// help; a person must look.
    NeedsReview,
}

/// What the queue does after a failed run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Run it again after this delay. `counts` says whether the run that just
    /// failed stays counted against the attempts.
    Retry { after: Duration, counts: bool },
    /// Stop and hand the document to a person.
    NeedsReview,
}

/// How many times to try, and how long to wait between tries (ADR-0008).
///
/// Randomness is not generated here: callers pass a sample in basis points of
/// the unit interval (`0..=10_000`, uniformly distributed), so the rules stay
/// deterministic and testable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    max_attempts: u32,
    base_delay: Duration,
    max_delay: Duration,
    provider_delay: Duration,
    jitter_bp: u32,
}

impl Default for RetryPolicy {
    /// 5 attempts; 30 s doubling to a 15-minute cap; a provider outage is
    /// retried after 5 minutes; delays vary by +/-20%.
    fn default() -> Self {
        RetryPolicy {
            max_attempts: 5,
            base_delay: Duration::from_secs(30),
            max_delay: Duration::from_secs(15 * 60),
            provider_delay: Duration::from_secs(5 * 60),
            jitter_bp: 2000,
        }
    }
}

/// One whole in basis points.
const UNIT_BP: u128 = 10_000;

impl RetryPolicy {
    /// `max_attempts` must be 1 to 20 (the database allows no more);
    /// `jitter_bp` is the spread in basis points (2000 = +/-20%).
    pub fn new(
        max_attempts: u32,
        base_delay: Duration,
        max_delay: Duration,
        provider_delay: Duration,
        jitter_bp: u32,
    ) -> Result<Self> {
        if !(1..=20).contains(&max_attempts) {
            return Err(Error::InvalidJobPolicy("max_attempts must be 1 to 20"));
        }
        if base_delay.is_zero() {
            return Err(Error::InvalidJobPolicy("base delay must be more than zero"));
        }
        if max_delay < base_delay {
            return Err(Error::InvalidJobPolicy(
                "max delay must not be below the base delay",
            ));
        }
        if provider_delay.is_zero() {
            return Err(Error::InvalidJobPolicy("provider delay must be more than zero"));
        }
        if u128::from(jitter_bp) > UNIT_BP {
            return Err(Error::InvalidJobPolicy("jitter cannot be more than 100%"));
        }
        Ok(RetryPolicy {
            max_attempts,
            base_delay,
            max_delay,
            provider_delay,
            jitter_bp,
        })
    }

    pub fn max_attempts(&self) -> u32 {
        self.max_attempts
    }
    pub fn base_delay(&self) -> Duration {
        self.base_delay
    }
    pub fn max_delay(&self) -> Duration {
        self.max_delay
    }
    pub fn provider_delay(&self) -> Duration {
        self.provider_delay
    }
    pub fn jitter_bp(&self) -> u32 {
        self.jitter_bp
    }

    /// True once `attempts` runs have been started and counted.
    pub fn is_exhausted(&self, attempts: u32) -> bool {
        attempts >= self.max_attempts
    }

    /// The delay after the `attempt`-th failed run, before jitter: the base,
    /// doubling each time, never more than the cap. (Attempt 0 is treated as
    /// the first. Cannot overflow.)
    pub fn backoff(&self, attempt: u32) -> Duration {
        let doublings = attempt.max(1) - 1;
        let factor = 1u32.checked_shl(doublings).unwrap_or(u32::MAX);
        self.base_delay
            .checked_mul(factor)
            .map_or(self.max_delay, |delay| delay.min(self.max_delay))
    }

    /// [`backoff`](Self::backoff) with jitter applied, and never above the cap.
    pub fn retry_delay(&self, attempt: u32, unit_bp: u32) -> Duration {
        self.jittered(self.backoff(attempt), unit_bp).min(self.max_delay)
    }

    /// What to do after a failed run. `attempts` is the number of counted runs
    /// so far, including the one that just failed.
    pub fn decide(&self, kind: FailureKind, attempts: u32, unit_bp: u32) -> Decision {
        match kind {
            FailureKind::NeedsReview => Decision::NeedsReview,
            FailureKind::ProviderUnavailable => Decision::Retry {
                after: self.jittered(self.provider_delay, unit_bp),
                counts: false,
            },
            FailureKind::Transient if self.is_exhausted(attempts) => Decision::NeedsReview,
            FailureKind::Transient => Decision::Retry {
                after: self.retry_delay(attempts, unit_bp),
                counts: true,
            },
        }
    }

    /// Spreads `delay` over `1 - j .. 1 + j` of itself, where `unit_bp`
    /// (clamped to `0..=10_000`) picks the point in that range.
    fn jittered(&self, delay: Duration, unit_bp: u32) -> Duration {
        let unit = u128::from(unit_bp).min(UNIT_BP);
        let spread = u128::from(self.jitter_bp);
        let factor = UNIT_BP - spread + unit * 2 * spread / UNIT_BP;
        let millis = delay.as_millis().saturating_mul(factor) / UNIT_BP;
        Duration::from_millis(u64::try_from(millis).unwrap_or(u64::MAX))
    }
}

/// How long a worker may hold a job without a sign of life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeasePolicy {
    duration: Duration,
}

impl Default for LeasePolicy {
    /// Five minutes, with a heartbeat about every 100 seconds.
    fn default() -> Self {
        LeasePolicy {
            duration: Duration::from_secs(300),
        }
    }
}

impl LeasePolicy {
    /// At least 3 seconds, or there is no room to heartbeat in time.
    pub fn new(duration: Duration) -> Result<Self> {
        if duration < Duration::from_secs(3) {
            return Err(Error::InvalidJobPolicy("a lease must be at least 3 seconds"));
        }
        Ok(LeasePolicy { duration })
    }

    pub fn duration(&self) -> Duration {
        self.duration
    }

    /// A third of the lease: two heartbeats can be missed before it expires.
    pub fn heartbeat_interval(&self) -> Duration {
        self.duration / 3
    }
}

/// How long after being queued a job may take before it is handed to a person,
/// so that no job can hang forever even if no worker runs for days. A
/// parameter at enqueue; this is the default (ADR-0008).
pub const DEFAULT_DEADLINE: Duration = Duration::from_secs(48 * 3600);

/// How far along a job is. The total may not be known yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    done: u32,
    total: Option<u32>,
}

impl Progress {
    pub fn new(done: u32, total: Option<u32>) -> Result<Self> {
        if total.is_some_and(|total| done > total) {
            return Err(Error::InvalidJobProgress("done is more than total"));
        }
        Ok(Progress { done, total })
    }

    pub fn done(&self) -> u32 {
        self.done
    }

    pub fn total(&self) -> Option<u32> {
        self.total
    }

    /// Whole percent complete, rounded down; `None` when there is no total to
    /// measure against.
    pub fn percent(&self) -> Option<u8> {
        let total = u64::from(self.total.filter(|&total| total > 0)?);
        u8::try_from(u64::from(self.done) * 100 / total).ok()
    }
}

/// The longest stored `last_error`, in characters (the database column limit).
pub const MAX_ERROR_CHARS: usize = 500;
/// The longest stored progress note, in characters.
pub const MAX_NOTE_CHARS: usize = 200;

/// Makes a worker's error message fit its column: control characters removed,
/// whitespace collapsed, cut to [`MAX_ERROR_CHARS`] with an ellipsis. Without
/// this, a long error could break the very write that records the failure and
/// leave the job stuck. Error text must still never contain invoice content;
/// that is a rule for whoever writes the message.
pub fn short_error(text: &str) -> String {
    tidy(text, MAX_ERROR_CHARS)
}

/// The same for a progress note ([`MAX_NOTE_CHARS`]).
pub fn short_note(text: &str) -> String {
    tidy(text, MAX_NOTE_CHARS)
}

fn tidy(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    let mut last_was_space = true; // swallows leading whitespace
    for c in text.chars() {
        if c.is_whitespace() {
            if !last_was_space {
                out.push(' ');
            }
            last_was_space = true;
        } else if !c.is_control() {
            out.push(c);
            last_was_space = false;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    if out.chars().count() > max_chars {
        out = out.chars().take(max_chars - 1).collect();
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests;
