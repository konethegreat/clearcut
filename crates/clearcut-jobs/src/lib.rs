// Written by Kone Tshivhinda & Claude
//! The job runner (ADR-0008, step 3): the loop that turns queued jobs into
//! finished work.
//!
//! The queue itself (`clearcut-store`) only stores jobs and enforces the rules.
//! Something has to ask it for work, keep each job's lease alive while the work
//! runs, and report how it went. That is this crate. It knows nothing about
//! invoices: the actual work is a [`Handler`] you give it (the real handlers
//! arrive with the document pipeline, P2-04 onwards).
//!
//! What the runner guarantees, for every job it claims:
//!
//! - **The lease stays alive.** A background task sends a heartbeat at a steady
//!   rhythm, with the latest reported progress, for as long as the handler
//!   runs, however long that is.
//! - **A worker that loses its lease stops.** If a heartbeat finds the job has
//!   been taken over (or erased), the handler's work is abandoned at once and
//!   nothing is reported, so a stale worker can never overwrite the new
//!   holder's result.
//! - **Cancellation is cooperative.** The heartbeat learns when someone asked
//!   for the job to stop; the handler sees it through
//!   [`JobContext::is_cancelled`] and finishes with [`Finished::Cancelled`].
//! - **A failure is never lost.** Whatever the handler returns, including a
//!   panic, becomes a report to the queue, which applies the retry policy
//!   (`clearcut_core::jobs::RetryPolicy`). Every job ends `succeeded`,
//!   `needs_review` or `cancelled`, never silently stuck.
//! - **Progress is durable.** [`JobContext::report`] saves progress and a
//!   checkpoint immediately, and a retry starts with the checkpoint the last
//!   run saved ([`JobContext::checkpoint`]), so long documents resume instead of
//!   starting over.
//! - **Shutdown is graceful.** On the shutdown signal the loop stops claiming
//!   and waits for the jobs in flight to finish.

use clearcut_core::jobs::{FailureKind, JobKind, LeasePolicy, Outcome, Progress, RetryPolicy};
use clearcut_store::{ClaimedJob, FailureResult, Job, Store, TenantId};
use serde_json::Value;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, Notify, Semaphore};
use tokio::task::JoinSet;
use uuid::Uuid;

// ------------------------------------------------------------ configuration ----

/// A setting that cannot work.
#[derive(Debug, thiserror::Error)]
#[error("invalid runner setting: {0}")]
pub struct ConfigError(&'static str);

/// How a runner behaves. Start from [`RunnerConfig::new`] and change what you
/// need; the defaults are the ones agreed in ADR-0008.
#[derive(Debug, Clone)]
pub struct RunnerConfig {
    worker: String,
    lease: LeasePolicy,
    retry: RetryPolicy,
    heartbeat_every: Option<Duration>,
    poll_interval: Duration,
    concurrency: usize,
    per_tenant_cap: Option<u32>,
    kinds: Vec<JobKind>,
}

impl RunnerConfig {
    /// `worker` names this runner in the queue (and in audit entries), so make
    /// it identify the process, for example the host name and a counter.
    pub fn new(worker: impl Into<String>) -> Result<Self, ConfigError> {
        let worker = worker.into();
        if worker.trim().is_empty() {
            return Err(ConfigError("a worker needs a name"));
        }
        Ok(RunnerConfig {
            worker,
            lease: LeasePolicy::default(),
            retry: RetryPolicy::default(),
            heartbeat_every: None,
            poll_interval: Duration::from_secs(1),
            concurrency: 4,
            per_tenant_cap: None,
            kinds: JobKind::ALL.to_vec(),
        })
    }

    pub fn worker(&self) -> &str {
        &self.worker
    }

    pub fn lease(&self) -> LeasePolicy {
        self.lease
    }

    pub fn retry(&self) -> RetryPolicy {
        self.retry
    }

    /// How often a running job sends its heartbeat: a third of the lease unless
    /// set explicitly.
    pub fn heartbeat_every(&self) -> Duration {
        self.heartbeat_every
            .unwrap_or_else(|| self.lease.heartbeat_interval())
    }

    /// How long an idle runner waits before asking for work again.
    pub fn poll_interval(&self) -> Duration {
        self.poll_interval
    }

    /// The most jobs this runner works on at once.
    pub fn concurrency(&self) -> usize {
        self.concurrency
    }

    pub fn per_tenant_cap(&self) -> Option<u32> {
        self.per_tenant_cap
    }

    pub fn kinds(&self) -> &[JobKind] {
        &self.kinds
    }

    /// A new lease length. An explicit heartbeat rhythm must still fit inside it.
    pub fn with_lease(mut self, lease: LeasePolicy) -> Result<Self, ConfigError> {
        if self
            .heartbeat_every
            .is_some_and(|every| every >= lease.duration())
        {
            return Err(ConfigError(
                "the heartbeat must come sooner than the lease runs out",
            ));
        }
        self.lease = lease;
        Ok(self)
    }

    pub fn with_retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    /// Overrides the heartbeat rhythm. It must be above zero and shorter than the lease.
    pub fn with_heartbeat_every(mut self, every: Duration) -> Result<Self, ConfigError> {
        if every.is_zero() {
            return Err(ConfigError("the heartbeat interval must be more than zero"));
        }
        if every >= self.lease.duration() {
            return Err(ConfigError(
                "the heartbeat must come sooner than the lease runs out",
            ));
        }
        self.heartbeat_every = Some(every);
        Ok(self)
    }

    pub fn with_poll_interval(mut self, interval: Duration) -> Result<Self, ConfigError> {
        if interval.is_zero() {
            return Err(ConfigError("the poll interval must be more than zero"));
        }
        self.poll_interval = interval;
        Ok(self)
    }

    pub fn with_concurrency(mut self, concurrency: usize) -> Result<Self, ConfigError> {
        if concurrency == 0 {
            return Err(ConfigError("a runner must be able to run at least one job"));
        }
        self.concurrency = concurrency;
        Ok(self)
    }

    /// At most this many jobs of one tenant run at once, across all runners
    /// (a soft limit; see `Store::claim_job`).
    pub fn with_per_tenant_cap(mut self, cap: Option<u32>) -> Self {
        self.per_tenant_cap = cap;
        self
    }

    /// The kinds of job this runner can handle.
    pub fn with_kinds(mut self, kinds: Vec<JobKind>) -> Self {
        self.kinds = kinds;
        self
    }
}

// ------------------------------------------------------------------ handler ----

/// How a handler's work ended, when it did not fail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finished {
    /// The document was read, validated and its events stored.
    Succeeded,
    /// The work cannot be completed automatically; a person must look. The text
    /// says why (no invoice content).
    NeedsReview(String),
    /// The handler saw that cancellation was requested and stopped.
    Cancelled,
}

/// A failed run. The kind decides what the queue does next
/// (`clearcut_core::jobs::RetryPolicy::decide`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub kind: FailureKind,
    /// A short machine-readable message, kept on the job. It must not contain
    /// invoice content. (Longer text is trimmed to fit.)
    pub message: String,
}

impl Failure {
    /// A hiccup worth retrying with backoff; counts against the attempts.
    pub fn transient(message: impl Into<String>) -> Self {
        Failure {
            kind: FailureKind::Transient,
            message: message.into(),
        }
    }

    /// The model provider is down or throttling: retry later without using up
    /// an attempt.
    pub fn provider_unavailable(message: impl Into<String>) -> Self {
        Failure {
            kind: FailureKind::ProviderUnavailable,
            message: message.into(),
        }
    }

    /// Even the top model tier could not produce valid output: stop retrying
    /// and hand the document to a person.
    pub fn needs_review(message: impl Into<String>) -> Self {
        Failure {
            kind: FailureKind::NeedsReview,
            message: message.into(),
        }
    }
}

pub type HandlerResult = Result<Finished, Failure>;

/// The work for a job. Implemented by whatever processes documents.
///
/// A handler gets the [`Job`] and a [`JobContext`] for reporting progress and
/// noticing cancellation. It must be safe to run twice for the same job (a
/// retry, or a worker that lost its lease), because the queue guarantees "at
/// least once", not "exactly once". (The events it writes are idempotent for
/// exactly this reason.)
pub trait Handler: Send + Sync + 'static {
    fn handle(&self, job: Job, ctx: JobContext) -> impl Future<Output = HandlerResult> + Send;
}

// ------------------------------------------------------------------ context ----

/// Why [`JobContext::report`] did not save.
#[derive(Debug, thiserror::Error)]
pub enum ReportError {
    /// The job was taken over or erased; stop working on it.
    #[error("the lease on this job is no longer held")]
    LeaseLost,
    #[error("could not save progress: {0}")]
    Store(#[from] clearcut_store::Error),
}

struct ContextInner {
    store: Store,
    claimed: ClaimedJob,
    lease: LeasePolicy,
    /// The latest progress. The lock is held across each heartbeat so a
    /// background beat can never write an older progress over a newer one that
    /// a handler has just reported.
    progress: Mutex<Progress>,
    /// The latest saved checkpoint (a plain lock: it is never held across an await).
    checkpoint: std::sync::Mutex<Option<Value>>,
    cancel_requested: AtomicBool,
    lost: AtomicBool,
    lost_signal: Notify,
}

/// What a handler can do while it works. Cheap to clone.
#[derive(Clone)]
pub struct JobContext {
    inner: Arc<ContextInner>,
}

impl JobContext {
    fn new(store: Store, claimed: ClaimedJob, lease: LeasePolicy) -> Self {
        let progress = claimed.job.progress;
        let checkpoint = claimed.job.checkpoint.clone();
        JobContext {
            inner: Arc::new(ContextInner {
                store,
                claimed,
                lease,
                progress: Mutex::new(progress),
                checkpoint: std::sync::Mutex::new(checkpoint),
                cancel_requested: AtomicBool::new(false),
                lost: AtomicBool::new(false),
                lost_signal: Notify::new(),
            }),
        }
    }

    /// The job as it was when claimed.
    pub fn job(&self) -> &Job {
        &self.inner.claimed.job
    }

    /// The checkpoint the previous run last saved (or this run, after a
    /// [`report`](Self::report)), so a retry can resume. `None` on a first run.
    pub fn checkpoint(&self) -> Option<Value> {
        self.inner
            .checkpoint
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Someone asked for this job to stop. Check it between units of work and
    /// finish with [`Finished::Cancelled`].
    pub fn is_cancelled(&self) -> bool {
        self.inner.cancel_requested.load(Ordering::SeqCst)
    }

    /// The job was taken over or erased. The runner abandons the handler soon
    /// after this becomes true, so most handlers never need to look.
    pub fn lease_lost(&self) -> bool {
        self.inner.lost.load(Ordering::SeqCst)
    }

    /// Saves progress now, with an optional note and checkpoint (`None` keeps
    /// what is stored), and picks up any cancellation request. Do this after
    /// each meaningful step (for example each page) so a retry resumes from
    /// there.
    pub async fn report(
        &self,
        progress: Progress,
        note: Option<&str>,
        checkpoint: Option<Value>,
    ) -> Result<(), ReportError> {
        if self.lease_lost() {
            return Err(ReportError::LeaseLost);
        }
        let mut progress_lock = self.inner.progress.lock().await;
        let beat = self
            .inner
            .store
            .heartbeat_job(
                &self.inner.claimed,
                self.inner.lease,
                progress,
                note,
                checkpoint.as_ref(),
            )
            .await;
        match beat {
            Ok(beat) => {
                *progress_lock = progress;
                if checkpoint.is_some() {
                    *self.inner.checkpoint.lock().unwrap_or_else(|e| e.into_inner()) = checkpoint;
                }
                if beat.cancel_requested {
                    self.inner.cancel_requested.store(true, Ordering::SeqCst);
                }
                Ok(())
            }
            Err(error) if lease_is_gone(&error) => {
                self.mark_lost();
                Err(ReportError::LeaseLost)
            }
            Err(error) => Err(error.into()),
        }
    }

    fn mark_lost(&self) {
        self.inner.lost.store(true, Ordering::SeqCst);
        // `notify_one` keeps a permit if nobody is waiting yet, so the signal
        // cannot be missed.
        self.inner.lost_signal.notify_one();
    }

    async fn wait_lost(&self) {
        loop {
            if self.lease_lost() {
                return;
            }
            self.inner.lost_signal.notified().await;
        }
    }

    /// One background heartbeat with the latest progress.
    async fn beat(&self) {
        let progress = self.inner.progress.lock().await;
        let result = self
            .inner
            .store
            .heartbeat_job(&self.inner.claimed, self.inner.lease, *progress, None, None)
            .await;
        match result {
            Ok(beat) => {
                if beat.cancel_requested {
                    self.inner.cancel_requested.store(true, Ordering::SeqCst);
                }
            }
            Err(error) if lease_is_gone(&error) => self.mark_lost(),
            // A passing database problem: the next beat may get through. If the
            // lease runs out meanwhile, the queue hands the job to someone else.
            Err(error) => tracing::warn!(job = %self.inner.claimed.job.id, %error, "a heartbeat failed"),
        }
    }
}

fn lease_is_gone(error: &clearcut_store::Error) -> bool {
    matches!(
        error,
        clearcut_store::Error::LeaseLost | clearcut_store::Error::JobNotFound
    )
}

// ------------------------------------------------------------------- runner ----

/// How a claimed job ended, from the runner's point of view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    Succeeded,
    NeedsReview,
    Cancelled,
    /// A failed run; the job is queued again after `after`.
    Retrying {
        after: Duration,
        counts: bool,
    },
    /// The job was taken over or erased while running. The handler was
    /// abandoned and nothing was reported.
    LeaseLost,
    /// The work finished but the report could not be saved (the database was
    /// unreachable). The lease will run out and the job will be run again.
    ReportFailed,
}

/// What [`Runner::run_once`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunReport {
    pub job: Uuid,
    pub tenant: TenantId,
    pub ended: Ended,
}

type Jitter = Arc<dyn Fn() -> u32 + Send + Sync>;

struct Inner<H> {
    store: Store,
    handler: Arc<H>,
    config: RunnerConfig,
    jitter: Jitter,
    /// How many times the runner has asked the queue for work.
    polls: AtomicU64,
}

/// Claims jobs, runs them through a [`Handler`], and reports the results.
/// Cheap to clone; clones share the same handler and settings.
pub struct Runner<H> {
    inner: Arc<Inner<H>>,
}

impl<H> Clone for Runner<H> {
    fn clone(&self) -> Self {
        Runner {
            inner: self.inner.clone(),
        }
    }
}

impl<H: Handler> Runner<H> {
    pub fn new(store: Store, handler: H, config: RunnerConfig) -> Self {
        Runner {
            inner: Arc::new(Inner {
                store,
                handler: Arc::new(handler),
                config,
                jitter: Arc::new(random_unit_bp),
                polls: AtomicU64::new(0),
            }),
        }
    }

    /// Replaces the source of the random sample (`0..=10_000`) that spreads
    /// retry delays. Tests use a fixed one.
    pub fn with_jitter(self, jitter: impl Fn() -> u32 + Send + Sync + 'static) -> Self {
        let old = &self.inner;
        Runner {
            inner: Arc::new(Inner {
                store: old.store.clone(),
                handler: old.handler.clone(),
                config: old.config.clone(),
                jitter: Arc::new(jitter),
                polls: AtomicU64::new(0),
            }),
        }
    }

    /// How many times this runner has asked the queue for work (including
    /// asks that found nothing). An idle runner should show about one a poll
    /// interval; a number racing upward means something is spinning.
    pub fn polls(&self) -> u64 {
        self.inner.polls.load(Ordering::Relaxed)
    }

    /// Claims at most one job, runs it to the end, and says how it went.
    /// `None` if nothing was due.
    pub async fn run_once(&self) -> Result<Option<RunReport>, clearcut_store::Error> {
        let config = &self.inner.config;
        self.inner.polls.fetch_add(1, Ordering::Relaxed);
        let claimed = self
            .inner
            .store
            .claim_job(
                config.worker(),
                config.lease(),
                config.kinds(),
                config.per_tenant_cap(),
            )
            .await?;
        match claimed {
            Some(claimed) => Ok(Some(self.process(claimed).await)),
            None => Ok(None),
        }
    }

    /// Runs until `shutdown` completes: claims jobs as slots free up, works on
    /// up to `concurrency` at once, and sleeps `poll_interval` when there is
    /// nothing to do or the database is unreachable. On shutdown it stops
    /// claiming and waits for the jobs in flight to finish.
    pub async fn run(&self, shutdown: impl Future<Output = ()> + Send) {
        let config = &self.inner.config;
        let slots = Arc::new(Semaphore::new(config.concurrency()));
        let mut in_flight = JoinSet::new();
        tokio::pin!(shutdown);
        loop {
            // Wait for a free slot (or the signal to stop).
            let slot = tokio::select! {
                _ = &mut shutdown => break,
                slot = slots.clone().acquire_owned() => match slot {
                    Ok(slot) => slot,
                    Err(_) => break, // the semaphore is never closed; defensive
                },
            };
            self.inner.polls.fetch_add(1, Ordering::Relaxed);
            let claimed = self
                .inner
                .store
                .claim_job(
                    config.worker(),
                    config.lease(),
                    config.kinds(),
                    config.per_tenant_cap(),
                )
                .await;
            match claimed {
                Ok(Some(claimed)) => {
                    let runner = self.clone();
                    in_flight.spawn(async move {
                        runner.process(claimed).await;
                        drop(slot); // frees the slot
                    });
                }
                other => {
                    drop(slot);
                    if let Err(error) = other {
                        tracing::error!(%error, "could not ask the queue for work; will try again");
                    }
                    tokio::select! {
                        _ = &mut shutdown => break,
                        _ = tokio::time::sleep(config.poll_interval()) => {}
                    }
                }
            }
            // Forget the jobs that have finished.
            while in_flight.try_join_next().is_some() {}
        }
        // Graceful: let the jobs already running finish.
        while in_flight.join_next().await.is_some() {}
    }

    /// Runs one claimed job to the end.
    async fn process(&self, claimed: ClaimedJob) -> RunReport {
        let config = &self.inner.config;
        let job = claimed.job.id;
        let tenant = claimed.job.tenant;
        let ctx = JobContext::new(self.inner.store.clone(), claimed.clone(), config.lease());

        // The heartbeat runs on its own task so it keeps ticking while the
        // handler is busy inside one long call.
        let beating = {
            let ctx = ctx.clone();
            let every = config.heartbeat_every();
            tokio::spawn(async move {
                let mut ticker = tokio::time::interval_at(tokio::time::Instant::now() + every, every);
                ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                loop {
                    ticker.tick().await;
                    ctx.beat().await;
                    if ctx.lease_lost() {
                        return;
                    }
                }
            })
        };

        // The handler also runs on its own task. That is how a panic inside it
        // becomes an ordinary error here instead of killing the runner, and
        // how it can be abandoned (dropped) when the lease is lost.
        let mut work = {
            let handler = self.inner.handler.clone();
            let job = claimed.job.clone();
            let ctx = ctx.clone();
            tokio::spawn(async move { handler.handle(job, ctx).await })
        };

        let joined = tokio::select! {
            joined = &mut work => Some(joined),
            _ = ctx.wait_lost() => None,
        };
        beating.abort();

        let ended = match joined {
            None => {
                work.abort();
                let _ = work.await; // wait until the handler's future is really dropped
                tracing::warn!(%job, "lost the lease; abandoned the work");
                Ended::LeaseLost
            }
            Some(Ok(Ok(finished))) => self.finish(&claimed, finished).await,
            Some(Ok(Err(failure))) => self.fail(&claimed, failure).await,
            Some(Err(error)) => {
                let message = if error.is_panic() {
                    "handler panicked"
                } else {
                    "handler was cancelled"
                };
                tracing::error!(%job, message, "the handler did not finish");
                self.fail(&claimed, Failure::transient(message)).await
            }
        };
        RunReport { job, tenant, ended }
    }

    async fn finish(&self, claimed: &ClaimedJob, finished: Finished) -> Ended {
        let (outcome, reason, ended) = match finished {
            Finished::Succeeded => (Outcome::Succeeded, None, Ended::Succeeded),
            Finished::NeedsReview(reason) => (Outcome::NeedsReview, Some(reason), Ended::NeedsReview),
            Finished::Cancelled => (Outcome::Cancelled, None, Ended::Cancelled),
        };
        match self
            .inner
            .store
            .complete_job(claimed, outcome, reason.as_deref())
            .await
        {
            Ok(()) => ended,
            Err(error) => report_error(claimed, error),
        }
    }

    async fn fail(&self, claimed: &ClaimedJob, failure: Failure) -> Ended {
        let unit = (self.inner.jitter)();
        let result = self
            .inner
            .store
            .fail_job(
                claimed,
                failure.kind,
                &failure.message,
                &self.inner.config.retry(),
                unit,
            )
            .await;
        match result {
            Ok(FailureResult::Retrying { after, counts }) => Ended::Retrying { after, counts },
            Ok(FailureResult::NeedsReview) => Ended::NeedsReview,
            Ok(FailureResult::Cancelled) => Ended::Cancelled,
            Err(error) => report_error(claimed, error),
        }
    }
}

/// A report to the queue failed. If the job is gone or taken over there is
/// nothing to do; anything else is logged and the lease will run out.
fn report_error(claimed: &ClaimedJob, error: clearcut_store::Error) -> Ended {
    if lease_is_gone(&error) {
        tracing::warn!(job = %claimed.job.id, "the job was taken over or erased before it could be reported");
        Ended::LeaseLost
    } else {
        tracing::error!(job = %claimed.job.id, %error, "could not report the result of a job");
        Ended::ReportFailed
    }
}

/// A uniform random sample in `0..=10_000` for retry jitter.
fn random_unit_bp() -> u32 {
    let mut bytes = [0u8; 4];
    // If the operating system's random source fails, the middle of the range
    // gives the nominal delay, which is a safe fallback.
    if getrandom::fill(&mut bytes).is_err() {
        return 5000;
    }
    u32::from_le_bytes(bytes) % 10_001
}

#[cfg(test)]
mod tests;
