// Written by Kone Tshivhinda & Claude
//! The runner against a real Postgres, as the restricted `clearcut_app` role the
//! service runs as, with scripted handlers standing in for real work. `pool`
//! (from `sqlx::test`) is the owner connection: tests use it to look from
//! outside and to move time without waiting.

use super::*;
use clearcut_core::jobs::{FailureKind, JobKind, JobState, LeasePolicy, Progress, RetryPolicy};
use clearcut_store::{DocId, DocumentStatus, NewDocument, NewJob, TenantId};
use serde_json::json;
use sqlx::PgPool;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use uuid::Uuid;

// ---------- test doubles ----------

type HandlerFuture = Pin<Box<dyn Future<Output = HandlerResult> + Send>>;

/// A handler whose behaviour a test writes as a closure.
struct Script {
    run: Box<dyn Fn(Job, JobContext) -> HandlerFuture + Send + Sync>,
}

impl Handler for Script {
    fn handle(&self, job: Job, ctx: JobContext) -> impl Future<Output = HandlerResult> + Send {
        (self.run)(job, ctx)
    }
}

fn script<F, Fut>(f: F) -> Script
where
    F: Fn(Job, JobContext) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = HandlerResult> + Send + 'static,
{
    Script {
        run: Box::new(move |job, ctx| Box::pin(f(job, ctx))),
    }
}

/// Settings that make a test fast: heartbeats and polls every few milliseconds.
fn fast() -> RunnerConfig {
    RunnerConfig::new("test-worker")
        .unwrap()
        .with_heartbeat_every(Duration::from_millis(50))
        .unwrap()
        .with_poll_interval(Duration::from_millis(20))
        .unwrap()
}

fn runner(store: &Store, handler: Script, config: RunnerConfig) -> Runner<Script> {
    // A jitter sample of 50% means "exactly the nominal delay".
    Runner::new(store.clone(), handler, config).with_jitter(|| 5000)
}

async fn app_store(pool: &PgPool) -> Store {
    let options = pool.connect_options().as_ref().clone().username("clearcut_app");
    let app_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(8)
        .connect_with(options)
        .await
        .unwrap();
    Store::new(app_pool)
}

fn document(seed: u8) -> NewDocument {
    NewDocument {
        original_filename: format!("invoice-{seed}.pdf"),
        content_type: "application/pdf".into(),
        byte_size: 100,
        sha256: [seed; 32],
        storage_key: format!("tenants/t/docs/{seed}"),
        page_count: None,
    }
}

fn actor() -> serde_json::Value {
    json!({ "type": "system" })
}

struct World {
    pool: PgPool,
    store: Store,
    tenant: TenantId,
}

async fn world(pool: &PgPool) -> World {
    let tenant = Store::new(pool.clone()).create_tenant("Acme").await.unwrap();
    World {
        pool: pool.clone(),
        store: app_store(pool).await,
        tenant,
    }
}

impl World {
    /// Takes a document away from under a running job. `erase_document` now refuses that (a live
    /// job makes it fail), so this does it as the owner: what a worker must survive whatever the
    /// cause (an operator's hard removal, a tenant deleted).
    async fn remove_under_a_running_job(&self, doc: DocId) {
        sqlx::query("delete from jobs where document_id = $1")
            .bind(doc.0)
            .execute(&self.pool)
            .await
            .unwrap();
        sqlx::query("delete from documents where id = $1")
            .bind(doc.0)
            .execute(&self.pool)
            .await
            .unwrap();
    }

    /// A new document with a queued job.
    async fn job(&self, seed: u8) -> (DocId, Uuid) {
        let doc = self
            .store
            .add_document(self.tenant, &document(seed))
            .await
            .unwrap()
            .id;
        let id = self
            .store
            .enqueue_job(self.tenant, &NewJob::new(doc, JobKind::ProcessDocument), &actor())
            .await
            .unwrap()
            .id;
        (doc, id)
    }

    async fn state(&self, job: Uuid) -> String {
        sqlx::query_scalar("select state from jobs where id = $1")
            .bind(job)
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }

    async fn job_row(&self, job: Uuid) -> Job {
        self.store.get_job(self.tenant, job).await.unwrap()
    }

    async fn doc_status(&self, doc: DocId) -> DocumentStatus {
        self.store.document_status(self.tenant, doc).await.unwrap()
    }

    async fn make_due(&self, job: Uuid) {
        sqlx::query("update jobs set run_after = now() - interval '1 second' where id = $1")
            .bind(job)
            .execute(&self.pool)
            .await
            .unwrap();
    }

    async fn wait_for_state(&self, job: Uuid, state: &str) {
        for _ in 0..200 {
            if self.state(job).await == state {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("job never reached {state}");
    }
}

async fn run_once(runner: &Runner<Script>) -> RunReport {
    runner
        .run_once()
        .await
        .unwrap()
        .expect("a job should have been claimed")
}

/// Sets a flag when dropped: proves a handler future was cancelled.
struct DropFlag(Arc<AtomicBool>);

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

// ---------- configuration ----------

#[test]
fn the_default_settings_are_the_agreed_ones() {
    let config = RunnerConfig::new("worker-1").unwrap();
    assert_eq!(config.worker(), "worker-1");
    assert_eq!(config.lease(), LeasePolicy::default());
    assert_eq!(config.retry(), RetryPolicy::default());
    assert_eq!(
        config.heartbeat_every(),
        Duration::from_secs(100),
        "a third of the lease"
    );
    assert_eq!(config.poll_interval(), Duration::from_secs(1));
    assert_eq!(config.concurrency(), 4);
    assert_eq!(config.per_tenant_cap(), None);
    assert_eq!(config.kinds(), JobKind::ALL);
}

#[test]
fn settings_that_cannot_work_are_refused() {
    assert!(RunnerConfig::new("").is_err());
    assert!(RunnerConfig::new("   ").is_err());
    let config = RunnerConfig::new("w").unwrap();
    assert!(config.clone().with_concurrency(0).is_err());
    assert!(config.clone().with_concurrency(1).is_ok());
    assert!(config.clone().with_poll_interval(Duration::ZERO).is_err());
    assert!(config.clone().with_heartbeat_every(Duration::ZERO).is_err());
    // A heartbeat no sooner than the lease runs out is useless.
    let lease = config.lease().duration();
    assert!(config.clone().with_heartbeat_every(lease).is_err());
    assert!(config
        .clone()
        .with_heartbeat_every(lease - Duration::from_millis(1))
        .is_ok());
}

#[test]
fn a_new_lease_moves_the_default_heartbeat_with_it() {
    let config = RunnerConfig::new("w")
        .unwrap()
        .with_lease(LeasePolicy::new(Duration::from_secs(30)).unwrap())
        .unwrap();
    assert_eq!(config.heartbeat_every(), Duration::from_secs(10));
}

#[test]
fn a_lease_too_short_for_an_explicit_heartbeat_is_refused() {
    let config = RunnerConfig::new("w")
        .unwrap()
        .with_heartbeat_every(Duration::from_secs(60))
        .unwrap();
    let short = LeasePolicy::new(Duration::from_secs(30)).unwrap();
    assert!(config.clone().with_lease(short).is_err());
    // A lease exactly as long as the heartbeat interval is just as useless.
    let equal = LeasePolicy::new(Duration::from_secs(60)).unwrap();
    assert!(config.clone().with_lease(equal).is_err());
    let roomy = LeasePolicy::new(Duration::from_secs(61)).unwrap();
    assert!(config.with_lease(roomy).is_ok());
}

#[test]
fn failures_are_built_with_the_right_kind() {
    assert_eq!(Failure::transient("x").kind, FailureKind::Transient);
    assert_eq!(
        Failure::provider_unavailable("x").kind,
        FailureKind::ProviderUnavailable
    );
    assert_eq!(Failure::needs_review("x").kind, FailureKind::NeedsReview);
    assert_eq!(Failure::transient("boom").message, "boom");
}

// ---------- one job, start to finish ----------

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn nothing_due_means_nothing_to_do(pool: PgPool) {
    let w = world(&pool).await;
    let r = runner(&w.store, script(|_, _| async { Ok(Finished::Succeeded) }), fast());
    assert!(r.run_once().await.unwrap().is_none());
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_successful_handler_finishes_the_job_and_the_document(pool: PgPool) {
    let w = world(&pool).await;
    let (doc, id) = w.job(1).await;
    let r = runner(&w.store, script(|_, _| async { Ok(Finished::Succeeded) }), fast());
    let report = run_once(&r).await;
    assert_eq!(report.job, id);
    assert_eq!(report.tenant, w.tenant);
    assert_eq!(report.ended, Ended::Succeeded);
    assert_eq!(w.state(id).await, "succeeded");
    assert_eq!(w.doc_status(doc).await, DocumentStatus::Done);
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn the_handler_gets_the_job_as_claimed(pool: PgPool) {
    let w = world(&pool).await;
    let (doc, id) = w.job(1).await;
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let r = runner(
        &w.store,
        script(move |job, ctx| {
            let sink = sink.clone();
            async move {
                sink.lock()
                    .unwrap()
                    .push((job.id, job.document, job.attempts, job.kind, job.state));
                assert_eq!(ctx.job().id, job.id);
                Ok(Finished::Succeeded)
            }
        }),
        fast(),
    );
    run_once(&r).await;
    assert_eq!(
        *seen.lock().unwrap(),
        [(id, doc, 1, JobKind::ProcessDocument, JobState::Running)]
    );
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_handler_can_hand_the_document_to_a_person(pool: PgPool) {
    let w = world(&pool).await;
    let (doc, id) = w.job(1).await;
    let r = runner(
        &w.store,
        script(|_, _| async { Ok(Finished::NeedsReview("totals disagree by R2.00".into())) }),
        fast(),
    );
    assert_eq!(run_once(&r).await.ended, Ended::NeedsReview);
    let job = w.job_row(id).await;
    assert_eq!(job.state, JobState::NeedsReview);
    assert_eq!(job.last_error.as_deref(), Some("totals disagree by R2.00"));
    assert_eq!(w.doc_status(doc).await, DocumentStatus::NeedsReview);
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn only_the_configured_kinds_are_run(pool: PgPool) {
    let w = world(&pool).await;
    let (_, id) = w.job(1).await;
    let r = runner(
        &w.store,
        script(|_, _| async { Ok(Finished::Succeeded) }),
        fast().with_kinds(Vec::new()),
    );
    assert!(r.run_once().await.unwrap().is_none());
    assert_eq!(w.state(id).await, "queued");
}

// ---------- failures ----------

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_transient_failure_is_retried_later_with_the_policys_delay(pool: PgPool) {
    let w = world(&pool).await;
    let (_, id) = w.job(1).await;
    let r = runner(
        &w.store,
        script(|_, _| async { Err(Failure::transient("timed out")) }),
        fast(),
    );
    let report = run_once(&r).await;
    assert_eq!(
        report.ended,
        Ended::Retrying {
            after: Duration::from_secs(30),
            counts: true
        }
    );
    let job = w.job_row(id).await;
    assert_eq!((job.state, job.attempts), (JobState::Queued, 1));
    assert_eq!(job.last_error.as_deref(), Some("timed out"));
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn the_random_sample_spreads_the_retry_delay(pool: PgPool) {
    let w = world(&pool).await;
    let (_, id) = w.job(1).await;
    let r = Runner::new(
        w.store.clone(),
        script(|_, _| async { Err(Failure::transient("x")) }),
        fast(),
    )
    .with_jitter(|| 0);
    let report = run_once(&r).await;
    assert_eq!(
        report.ended,
        Ended::Retrying {
            after: Duration::from_secs(24),
            counts: true
        }
    );
    assert_eq!(w.state(id).await, "queued");
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_provider_outage_does_not_use_up_attempts(pool: PgPool) {
    let w = world(&pool).await;
    let (_, id) = w.job(1).await;
    let r = runner(
        &w.store,
        script(|_, _| async { Err(Failure::provider_unavailable("503")) }),
        fast(),
    );
    for _ in 0..8 {
        w.make_due(id).await;
        let report = run_once(&r).await;
        assert!(matches!(report.ended, Ended::Retrying { counts: false, .. }));
        let job = w.job_row(id).await;
        assert_eq!((job.state, job.attempts), (JobState::Queued, 0));
    }
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_failure_the_top_tier_could_not_fix_goes_straight_to_review(pool: PgPool) {
    let w = world(&pool).await;
    let (doc, id) = w.job(1).await;
    let r = runner(
        &w.store,
        script(|_, _| async { Err(Failure::needs_review("totals never reconcile")) }),
        fast(),
    );
    assert_eq!(run_once(&r).await.ended, Ended::NeedsReview);
    assert_eq!(w.state(id).await, "needs_review");
    assert_eq!(w.doc_status(doc).await, DocumentStatus::NeedsReview);
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn five_ordinary_failures_end_in_review(pool: PgPool) {
    let w = world(&pool).await;
    let (doc, id) = w.job(1).await;
    let r = runner(
        &w.store,
        script(|_, _| async { Err(Failure::transient("bad output")) }),
        fast(),
    );
    for attempt in 1..=5 {
        w.make_due(id).await;
        let report = run_once(&r).await;
        if attempt < 5 {
            assert!(
                matches!(report.ended, Ended::Retrying { counts: true, .. }),
                "{attempt}"
            );
        } else {
            assert_eq!(report.ended, Ended::NeedsReview);
        }
    }
    assert_eq!(w.state(id).await, "needs_review");
    assert_eq!(w.doc_status(doc).await, DocumentStatus::NeedsReview);
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_handler_that_panics_is_a_transient_failure_not_a_dead_runner(pool: PgPool) {
    let w = world(&pool).await;
    let (_, id) = w.job(1).await;
    let r = runner(
        &w.store,
        script(|_, _| async {
            if true {
                panic!("kaboom");
            }
            Ok(Finished::Succeeded)
        }),
        fast(),
    );
    let report = run_once(&r).await;
    assert!(
        matches!(report.ended, Ended::Retrying { counts: true, .. }),
        "{report:?}"
    );
    let job = w.job_row(id).await;
    assert_eq!(job.state, JobState::Queued);
    assert!(job.last_error.unwrap().contains("panicked"));
    // The runner is still usable.
    assert!(r.run_once().await.unwrap().is_none(), "the retry is not due yet");
}

// ---------- progress and resuming ----------

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn reported_progress_is_saved_at_once_and_a_retry_resumes_from_the_checkpoint(pool: PgPool) {
    let w = world(&pool).await;
    let (_, id) = w.job(1).await;
    let resumed = Arc::new(Mutex::new(None));
    let sink = resumed.clone();
    let r = runner(
        &w.store,
        script(move |job, ctx| {
            let sink = sink.clone();
            async move {
                if job.attempts == 1 {
                    ctx.report(
                        Progress::new(2, Some(5)).unwrap(),
                        Some("page 2 of 5"),
                        Some(json!({ "next_page": 3 })),
                    )
                    .await
                    .unwrap();
                    Err(Failure::transient("model timed out on page 3"))
                } else {
                    *sink.lock().unwrap() = Some((ctx.checkpoint(), ctx.job().progress));
                    Ok(Finished::Succeeded)
                }
            }
        }),
        fast(),
    );
    run_once(&r).await;
    let after_failure = w.job_row(id).await;
    assert_eq!(after_failure.progress, Progress::new(2, Some(5)).unwrap());
    assert_eq!(after_failure.progress_note.as_deref(), Some("page 2 of 5"));
    assert_eq!(after_failure.checkpoint, Some(json!({ "next_page": 3 })));

    w.make_due(id).await;
    run_once(&r).await;
    assert_eq!(
        *resumed.lock().unwrap(),
        Some((
            Some(json!({ "next_page": 3 })),
            Progress::new(2, Some(5)).unwrap()
        )),
        "the second run starts where the first stopped"
    );
    assert_eq!(w.state(id).await, "succeeded");
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn the_background_heartbeat_keeps_reporting_the_latest_progress(pool: PgPool) {
    let w = world(&pool).await;
    let (_, id) = w.job(1).await;
    let r = runner(
        &w.store,
        script(|_, ctx| async move {
            ctx.report(Progress::new(2, Some(5)).unwrap(), None, None)
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(400)).await; // several heartbeats go by
            Ok(Finished::Succeeded)
        }),
        fast(),
    );
    let task = tokio::spawn({
        let r = r.clone();
        async move { r.run_once().await }
    });
    w.wait_for_state(id, "running").await;
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(
        w.job_row(id).await.progress,
        Progress::new(2, Some(5)).unwrap(),
        "not reset by a heartbeat"
    );
    task.await.unwrap().unwrap().unwrap();
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_long_job_keeps_its_lease_so_nobody_else_takes_it(pool: PgPool) {
    let w = world(&pool).await;
    let (_, id) = w.job(1).await;
    let config = fast()
        .with_lease(LeasePolicy::new(Duration::from_secs(3)).unwrap())
        .unwrap()
        .with_heartbeat_every(Duration::from_millis(300))
        .unwrap();
    let r = runner(
        &w.store,
        script(|_, _| async {
            tokio::time::sleep(Duration::from_millis(4500)).await; // longer than the lease
            Ok(Finished::Succeeded)
        }),
        config,
    );
    let task = tokio::spawn({
        let r = r.clone();
        async move { r.run_once().await }
    });
    w.wait_for_state(id, "running").await;
    tokio::time::sleep(Duration::from_millis(3700)).await; // past the original lease
    let thief = w
        .store
        .claim_job("thief", LeasePolicy::default(), &JobKind::ALL, None)
        .await
        .unwrap();
    assert!(thief.is_none(), "the heartbeat must have extended the lease");
    let report = task.await.unwrap().unwrap().unwrap();
    assert_eq!(report.ended, Ended::Succeeded);
    assert_eq!(w.job_row(id).await.attempts, 1);
}

// ---------- cancellation and lost leases ----------

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_handler_is_told_to_stop_when_cancellation_is_requested(pool: PgPool) {
    let w = world(&pool).await;
    let (doc, id) = w.job(1).await;
    let r = runner(
        &w.store,
        script(|_, ctx| async move {
            for _ in 0..500 {
                if ctx.is_cancelled() {
                    return Ok(Finished::Cancelled);
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Ok(Finished::Succeeded)
        }),
        fast(),
    );
    let task = tokio::spawn({
        let r = r.clone();
        async move { r.run_once().await }
    });
    w.wait_for_state(id, "running").await;
    w.store.request_cancel(w.tenant, id, &actor()).await.unwrap();
    let report = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .expect("the handler must notice the cancellation")
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(report.ended, Ended::Cancelled);
    assert_eq!(w.state(id).await, "cancelled");
    assert_eq!(w.doc_status(doc).await, DocumentStatus::Received);
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_worker_that_loses_its_lease_is_stopped_and_changes_nothing(pool: PgPool) {
    let w = world(&pool).await;
    let (_, id) = w.job(1).await;
    let dropped = Arc::new(AtomicBool::new(false));
    let flag = dropped.clone();
    let r = runner(
        &w.store,
        script(move |_, _| {
            let guard = DropFlag(flag.clone());
            async move {
                let _guard = guard;
                tokio::time::sleep(Duration::from_secs(60)).await;
                Ok(Finished::Succeeded)
            }
        }),
        // A slow heartbeat: the worker really has "stalled" while the thief
        // moves in. (A live worker that heartbeats cannot be robbed.)
        fast().with_heartbeat_every(Duration::from_millis(400)).unwrap(),
    );
    let task = tokio::spawn({
        let r = r.clone();
        async move { r.run_once().await }
    });
    w.wait_for_state(id, "running").await;

    // The worker stalls past its lease and another runner takes the job over.
    sqlx::query("update jobs set lease_expires_at = now() - interval '1 second' where id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    let thief = w
        .store
        .claim_job("thief", LeasePolicy::default(), &JobKind::ALL, None)
        .await
        .unwrap()
        .expect("the expired job is claimable");
    assert_eq!(thief.job.attempts, 2);

    let report = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .expect("the stale worker must stop")
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(report.ended, Ended::LeaseLost);
    assert!(dropped.load(Ordering::SeqCst), "the handler's work was abandoned");
    let job = w.job_row(id).await;
    assert_eq!(job.state, JobState::Running, "the new holder's job is untouched");
    assert_eq!(job.leased_by.as_deref(), Some("thief"));
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_worker_whose_document_was_erased_stops_too(pool: PgPool) {
    let w = world(&pool).await;
    let (doc, id) = w.job(1).await;
    let r = runner(
        &w.store,
        script(|_, _| async {
            tokio::time::sleep(Duration::from_secs(60)).await;
            Ok(Finished::Succeeded)
        }),
        fast(),
    );
    let task = tokio::spawn({
        let r = r.clone();
        async move { r.run_once().await }
    });
    w.wait_for_state(id, "running").await;
    w.remove_under_a_running_job(doc).await;
    let report = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .expect("the worker must stop")
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(report.ended, Ended::LeaseLost);
}

// ---------- the loop ----------

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn the_loop_runs_every_job_with_no_more_at_once_than_allowed(pool: PgPool) {
    let w = world(&pool).await;
    let mut ids = Vec::new();
    for seed in 1..=6 {
        ids.push(w.job(seed).await.1);
    }
    let now = Arc::new(AtomicUsize::new(0));
    let most = Arc::new(AtomicUsize::new(0));
    let (n, m) = (now.clone(), most.clone());
    let r = runner(
        &w.store,
        script(move |_, _| {
            let (n, m) = (n.clone(), m.clone());
            async move {
                let running = n.fetch_add(1, Ordering::SeqCst) + 1;
                m.fetch_max(running, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(100)).await;
                n.fetch_sub(1, Ordering::SeqCst);
                Ok(Finished::Succeeded)
            }
        }),
        fast().with_concurrency(3).unwrap(),
    );
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let looping = tokio::spawn({
        let r = r.clone();
        async move {
            r.run(async {
                stopped.await.ok();
            })
            .await
        }
    });
    for id in &ids {
        w.wait_for_state(*id, "succeeded").await;
    }
    stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(10), looping)
        .await
        .expect("run returns")
        .unwrap();
    let most = most.load(Ordering::SeqCst);
    assert!((2..=3).contains(&most), "ran {most} at once; wanted 2 or 3");
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn one_tenant_cannot_fill_every_slot_when_a_cap_is_set(pool: PgPool) {
    let w = world(&pool).await;
    let mut ids = Vec::new();
    for seed in 1..=3 {
        ids.push(w.job(seed).await.1);
    }
    let now = Arc::new(AtomicUsize::new(0));
    let most = Arc::new(AtomicUsize::new(0));
    let (n, m) = (now.clone(), most.clone());
    let r = runner(
        &w.store,
        script(move |_, _| {
            let (n, m) = (n.clone(), m.clone());
            async move {
                let running = n.fetch_add(1, Ordering::SeqCst) + 1;
                m.fetch_max(running, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(150)).await;
                n.fetch_sub(1, Ordering::SeqCst);
                Ok(Finished::Succeeded)
            }
        }),
        fast().with_concurrency(3).unwrap().with_per_tenant_cap(Some(1)),
    );
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let looping = tokio::spawn({
        let r = r.clone();
        async move {
            r.run(async {
                stopped.await.ok();
            })
            .await
        }
    });
    for id in &ids {
        w.wait_for_state(*id, "succeeded").await;
    }
    stop.send(()).unwrap();
    looping.await.unwrap();
    assert_eq!(most.load(Ordering::SeqCst), 1, "a cap of one for one tenant");
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn shutting_down_lets_the_jobs_in_flight_finish(pool: PgPool) {
    let w = world(&pool).await;
    let (doc, id) = w.job(1).await;
    let r = runner(
        &w.store,
        script(|_, _| async {
            tokio::time::sleep(Duration::from_millis(400)).await;
            Ok(Finished::Succeeded)
        }),
        fast(),
    );
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let looping = tokio::spawn({
        let r = r.clone();
        async move {
            r.run(async {
                stopped.await.ok();
            })
            .await
        }
    });
    w.wait_for_state(id, "running").await;
    stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(10), looping)
        .await
        .expect("run returns")
        .unwrap();
    assert_eq!(
        w.state(id).await,
        "succeeded",
        "the job was finished, not abandoned"
    );
    assert_eq!(w.doc_status(doc).await, DocumentStatus::Done);
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn an_idle_loop_stops_promptly_when_asked(pool: PgPool) {
    let w = world(&pool).await;
    let r = runner(&w.store, script(|_, _| async { Ok(Finished::Succeeded) }), fast());
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let looping = tokio::spawn({
        let r = r.clone();
        async move {
            r.run(async {
                stopped.await.ok();
            })
            .await
        }
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), looping)
        .await
        .expect("run returns")
        .unwrap();
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_database_outage_does_not_crash_the_loop(pool: PgPool) {
    let w = world(&pool).await;
    let r = runner(&w.store, script(|_, _| async { Ok(Finished::Succeeded) }), fast());
    w.store.pool().close().await; // every claim now fails
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let looping = tokio::spawn({
        let r = r.clone();
        async move {
            r.run(async {
                stopped.await.ok();
            })
            .await
        }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!looping.is_finished(), "the loop keeps trying");
    stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), looping)
        .await
        .expect("run returns")
        .unwrap();
}

#[test]
fn the_random_retry_sample_stays_in_range_and_varies() {
    let samples: Vec<u32> = (0..2000).map(|_| random_unit_bp()).collect();
    assert!(samples.iter().all(|&s| s <= 10_000));
    assert!(samples.iter().any(|&s| s != samples[0]), "not constant");
    assert!(
        samples.iter().any(|&s| s < 2_000) && samples.iter().any(|&s| s > 8_000),
        "spread across the range"
    );
}

// ---------- more guarantees ----------

/// Heartbeats so slow that only a handler's own `report()` can carry news.
fn quiet() -> RunnerConfig {
    fast().with_heartbeat_every(Duration::from_secs(30)).unwrap()
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_reported_checkpoint_is_what_the_handler_sees_next(pool: PgPool) {
    let w = world(&pool).await;
    let (_, _) = w.job(1).await;
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let r = runner(
        &w.store,
        script(move |_, ctx| {
            let sink = sink.clone();
            async move {
                sink.lock().unwrap().push(ctx.checkpoint());
                ctx.report(
                    Progress::new(1, Some(3)).unwrap(),
                    None,
                    Some(json!({ "page": 1 })),
                )
                .await
                .unwrap();
                sink.lock().unwrap().push(ctx.checkpoint());
                // A report without a checkpoint keeps the last one.
                ctx.report(Progress::new(2, Some(3)).unwrap(), None, None)
                    .await
                    .unwrap();
                sink.lock().unwrap().push(ctx.checkpoint());
                Ok(Finished::Succeeded)
            }
        }),
        fast(),
    );
    run_once(&r).await;
    assert_eq!(
        *seen.lock().unwrap(),
        [None, Some(json!({ "page": 1 })), Some(json!({ "page": 1 }))]
    );
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn reporting_progress_also_picks_up_a_cancellation_request(pool: PgPool) {
    let w = world(&pool).await;
    let (_, id) = w.job(1).await;
    let r = runner(
        &w.store,
        script(|_, ctx| async move {
            for step in 0..1000u32 {
                ctx.report(Progress::new(step, None).unwrap(), None, None)
                    .await
                    .unwrap();
                if ctx.is_cancelled() {
                    return Ok(Finished::Cancelled);
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Ok(Finished::Succeeded)
        }),
        quiet(), // the background heartbeat will not come in time to help
    );
    let task = tokio::spawn({
        let r = r.clone();
        async move { r.run_once().await }
    });
    w.wait_for_state(id, "running").await;
    w.store.request_cancel(w.tenant, id, &actor()).await.unwrap();
    let report = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .expect("report() must carry the cancellation to the handler")
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(report.ended, Ended::Cancelled);
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_report_that_finds_the_lease_gone_stops_the_worker_even_if_the_handler_ignores_it(pool: PgPool) {
    let w = world(&pool).await;
    let (doc, id) = w.job(1).await;
    let knew = Arc::new(AtomicBool::new(false));
    let sink = knew.clone();
    let r = runner(
        &w.store,
        script(move |_, ctx| {
            let sink = sink.clone();
            async move {
                loop {
                    if ctx
                        .report(Progress::new(1, None).unwrap(), None, None)
                        .await
                        .is_err()
                    {
                        // A careless handler: it sees the error and carries on regardless.
                        sink.store(ctx.lease_lost(), Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_secs(60)).await;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            }
        }),
        quiet(),
    );
    let task = tokio::spawn({
        let r = r.clone();
        async move { r.run_once().await }
    });
    w.wait_for_state(id, "running").await;
    w.remove_under_a_running_job(doc).await;
    let report = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .expect("the runner must stop a worker whose report found the lease gone")
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(report.ended, Ended::LeaseLost);
    assert!(knew.load(Ordering::SeqCst), "the context says the lease is lost");
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn the_configured_retry_policy_is_the_one_applied(pool: PgPool) {
    let w = world(&pool).await;
    let (_, id) = w.job(1).await;
    let policy = RetryPolicy::new(
        2,
        Duration::from_secs(10),
        Duration::from_secs(100),
        Duration::from_secs(60),
        0,
    )
    .unwrap();
    let r = runner(
        &w.store,
        script(|_, _| async { Err(Failure::transient("bad")) }),
        fast().with_retry(policy),
    );
    let first = run_once(&r).await;
    assert_eq!(
        first.ended,
        Ended::Retrying {
            after: Duration::from_secs(10),
            counts: true
        }
    );
    w.make_due(id).await;
    let second = run_once(&r).await;
    assert_eq!(second.ended, Ended::NeedsReview, "two attempts, then a person");
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_failure_after_a_cancellation_request_ends_the_job_cancelled(pool: PgPool) {
    let w = world(&pool).await;
    let (doc, id) = w.job(1).await;
    let r = runner(
        &w.store,
        script(|_, ctx| async move {
            while !ctx.is_cancelled() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Err(Failure::transient("gave up while stopping"))
        }),
        fast(),
    );
    let task = tokio::spawn({
        let r = r.clone();
        async move { r.run_once().await }
    });
    w.wait_for_state(id, "running").await;
    w.store.request_cancel(w.tenant, id, &actor()).await.unwrap();
    let report = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(report.ended, Ended::Cancelled);
    assert_eq!(w.state(id).await, "cancelled");
    assert_eq!(w.doc_status(doc).await, DocumentStatus::Received);
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn finishing_after_being_replaced_is_refused_and_reported_as_a_lost_lease(pool: PgPool) {
    let w = world(&pool).await;
    let (_, id) = w.job(1).await;
    let store = w.store.clone();
    let owner = pool.clone();
    let r = runner(
        &w.store,
        script(move |_, _| {
            let (store, owner) = (store.clone(), owner.clone());
            async move {
                // Just before finishing, the lease runs out and another runner takes over.
                sqlx::query("update jobs set lease_expires_at = now() - interval '1 second'")
                    .execute(&owner)
                    .await
                    .unwrap();
                let thief = store
                    .claim_job("thief", LeasePolicy::default(), &JobKind::ALL, None)
                    .await
                    .unwrap();
                assert!(thief.is_some());
                Ok(Finished::Succeeded)
            }
        }),
        quiet(),
    );
    let report = run_once(&r).await;
    assert_eq!(report.ended, Ended::LeaseLost);
    let job = w.job_row(id).await;
    assert_eq!(job.state, JobState::Running, "the new holder's job is untouched");
    assert_eq!(job.leased_by.as_deref(), Some("thief"));
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn an_idle_loop_with_a_long_poll_still_stops_at_once(pool: PgPool) {
    let w = world(&pool).await;
    let r = runner(
        &w.store,
        script(|_, _| async { Ok(Finished::Succeeded) }),
        fast().with_poll_interval(Duration::from_secs(5)).unwrap(),
    );
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let looping = tokio::spawn({
        let r = r.clone();
        async move {
            r.run(async {
                stopped.await.ok();
            })
            .await
        }
    });
    tokio::time::sleep(Duration::from_millis(300)).await; // now asleep in the 5 s wait
    stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(1), looping)
        .await
        .expect("shutdown must not wait out the poll interval")
        .unwrap();
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn the_workers_name_is_recorded_on_the_job_it_runs(pool: PgPool) {
    let w = world(&pool).await;
    w.job(1).await;
    let seen = Arc::new(Mutex::new(None));
    let sink = seen.clone();
    let r = runner(
        &w.store,
        script(move |job, _| {
            let sink = sink.clone();
            async move {
                *sink.lock().unwrap() = Some(job.leased_by.clone());
                Ok(Finished::Succeeded)
            }
        }),
        fast(),
    );
    run_once(&r).await;
    assert_eq!(*seen.lock().unwrap(), Some(Some("test-worker".to_string())));
}

async fn db_now(pool: &PgPool) -> time::OffsetDateTime {
    sqlx::query_scalar("select now()").fetch_one(pool).await.unwrap()
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn the_configured_lease_is_the_one_requested(pool: PgPool) {
    let w = world(&pool).await;
    w.job(1).await;
    let seen = Arc::new(Mutex::new(None));
    let sink = seen.clone();
    let r = runner(
        &w.store,
        script(move |job, _| {
            let sink = sink.clone();
            async move {
                *sink.lock().unwrap() = job.lease_expires_at;
                Ok(Finished::Succeeded)
            }
        }),
        fast()
            .with_lease(LeasePolicy::new(Duration::from_secs(20)).unwrap())
            .unwrap(),
    );
    let before = db_now(&pool).await;
    run_once(&r).await;
    let expires = seen.lock().unwrap().expect("the job was leased");
    let lease = expires - before;
    assert!(
        lease > time::Duration::seconds(18) && lease < time::Duration::seconds(22),
        "asked for 20 s, got {lease}"
    );
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn the_per_tenant_cap_holds_for_a_single_run_too(pool: PgPool) {
    let w = world(&pool).await;
    let (_, first) = w.job(1).await;
    let (_, second) = w.job(2).await;
    let r = runner(
        &w.store,
        script(|_, _| async {
            tokio::time::sleep(Duration::from_millis(600)).await;
            Ok(Finished::Succeeded)
        }),
        fast().with_per_tenant_cap(Some(1)),
    );
    let running = tokio::spawn({
        let r = r.clone();
        async move { r.run_once().await }
    });
    // One job of the tenant is now running.
    for _ in 0..200 {
        if w.state(first).await == "running" || w.state(second).await == "running" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(r.run_once().await.unwrap().is_none(), "the tenant is at its cap");
    running.await.unwrap().unwrap().unwrap();
    assert!(
        r.run_once().await.unwrap().is_some(),
        "free again once the first finished"
    );
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn the_loop_uses_the_configured_worker_name_and_lease(pool: PgPool) {
    let w = world(&pool).await;
    let (_, id) = w.job(1).await;
    let seen = Arc::new(Mutex::new(None));
    let sink = seen.clone();
    let r = runner(
        &w.store,
        script(move |job, _| {
            let sink = sink.clone();
            async move {
                *sink.lock().unwrap() = Some((job.leased_by.clone(), job.lease_expires_at));
                Ok(Finished::Succeeded)
            }
        }),
        RunnerConfig::new("loop-worker")
            .unwrap()
            .with_lease(LeasePolicy::new(Duration::from_secs(20)).unwrap())
            .unwrap()
            .with_heartbeat_every(Duration::from_millis(50))
            .unwrap()
            .with_poll_interval(Duration::from_millis(20))
            .unwrap(),
    );
    let before = db_now(&pool).await;
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let looping = tokio::spawn({
        let r = r.clone();
        async move {
            r.run(async {
                stopped.await.ok();
            })
            .await
        }
    });
    w.wait_for_state(id, "succeeded").await;
    stop.send(()).unwrap();
    looping.await.unwrap();
    let (worker, expires) = seen.lock().unwrap().clone().expect("the job ran");
    assert_eq!(worker.as_deref(), Some("loop-worker"));
    let lease = expires.expect("leased") - before;
    assert!(
        lease > time::Duration::seconds(18) && lease < time::Duration::seconds(22),
        "{lease}"
    );
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn the_loop_leaves_alone_kinds_it_was_not_given(pool: PgPool) {
    let w = world(&pool).await;
    let (_, id) = w.job(1).await;
    let r = runner(
        &w.store,
        script(|_, _| async { Ok(Finished::Succeeded) }),
        fast().with_kinds(Vec::new()),
    );
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let looping = tokio::spawn({
        let r = r.clone();
        async move {
            r.run(async {
                stopped.await.ok();
            })
            .await
        }
    });
    tokio::time::sleep(Duration::from_millis(400)).await;
    stop.send(()).unwrap();
    looping.await.unwrap();
    assert_eq!(w.state(id).await, "queued");
    assert!(r.polls() >= 1);
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn an_idle_loop_asks_for_work_once_per_poll_interval_not_in_a_spin(pool: PgPool) {
    let w = world(&pool).await;
    let r = runner(
        &w.store,
        script(|_, _| async { Ok(Finished::Succeeded) }),
        fast().with_poll_interval(Duration::from_millis(200)).unwrap(),
    );
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let looping = tokio::spawn({
        let r = r.clone();
        async move {
            r.run(async {
                stopped.await.ok();
            })
            .await
        }
    });
    tokio::time::sleep(Duration::from_millis(700)).await;
    stop.send(()).unwrap();
    looping.await.unwrap();
    let polls = r.polls();
    assert!(
        (2..=6).contains(&polls),
        "polled {polls} times in 700 ms with a 200 ms interval"
    );
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn run_once_counts_as_a_poll(pool: PgPool) {
    let w = world(&pool).await;
    let r = runner(&w.store, script(|_, _| async { Ok(Finished::Succeeded) }), fast());
    assert_eq!(r.polls(), 0);
    r.run_once().await.unwrap();
    r.run_once().await.unwrap();
    assert_eq!(r.polls(), 2);
}
