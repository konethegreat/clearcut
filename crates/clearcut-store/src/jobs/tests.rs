// Written by Kone Tshivhinda & Claude
//! The queue against a real Postgres, as the restricted `clearcut_app` role the
//! service runs as. `pool` (from `sqlx::test`) is the owner connection: tests
//! use it to look from outside and to move time (expire a lease, make a job
//! due) without waiting.

use super::*;
use crate::tests::{count, db_message, new_document, new_tenant, setup};
use crate::DocumentStatus;
use clearcut_core::jobs::{FailureKind, JobKind, JobState, LeasePolicy, Outcome, Progress, RetryPolicy};
use serde_json::json;
use sqlx::PgPool;
use std::time::Duration;
use time::OffsetDateTime;

const WORKER: &str = "worker-1";

fn actor() -> serde_json::Value {
    json!({ "type": "api_key", "id": "00000000-0000-0000-0000-00000000000a" })
}

async fn db_now(pool: &PgPool) -> OffsetDateTime {
    sqlx::query_scalar("select now()").fetch_one(pool).await.unwrap()
}

async fn enqueue(store: &Store, tenant: TenantId, doc: DocId) -> Uuid {
    store
        .enqueue_job(tenant, &NewJob::new(doc, JobKind::ProcessDocument), &actor())
        .await
        .unwrap()
        .id
}

async fn claim(store: &Store) -> Option<ClaimedJob> {
    store
        .claim_job(WORKER, LeasePolicy::default(), &JobKind::ALL, None)
        .await
        .unwrap()
}

async fn claim_expect(store: &Store) -> ClaimedJob {
    claim(store).await.expect("a job should have been claimable")
}

/// Pretend the retry delay has passed.
async fn make_due(pool: &PgPool, job: Uuid) {
    sqlx::query("update jobs set run_after = now() - interval '1 second' where id = $1")
        .bind(job)
        .execute(pool)
        .await
        .unwrap();
}

/// Pretend the worker has stopped heartbeating.
async fn expire_lease(pool: &PgPool, job: Uuid) {
    sqlx::query("update jobs set lease_expires_at = now() - interval '1 second' where id = $1")
        .bind(job)
        .execute(pool)
        .await
        .unwrap();
}

async fn state_in_db(pool: &PgPool, job: Uuid) -> String {
    sqlx::query_scalar("select state from jobs where id = $1")
        .bind(job)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn audit_actions(pool: &PgPool, job: Uuid) -> Vec<String> {
    sqlx::query_scalar(
        "select action from audit_log where subject_type = 'job' and subject_id = $1 order by id",
    )
    .bind(job.to_string())
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn doc_status(store: &Store, tenant: TenantId, doc: DocId) -> DocumentStatus {
    store.document_status(tenant, doc).await.unwrap()
}

// ---------- enqueue and read ----------

#[sqlx::test]
async fn a_new_job_is_queued_with_the_agreed_defaults(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let before = db_now(&pool).await;
    let id = enqueue(&store, tenant, doc).await;
    let job = store.get_job(tenant, id).await.unwrap();
    assert_eq!(job.id, id);
    assert_eq!(job.tenant, tenant);
    assert_eq!(job.document, doc);
    assert_eq!(job.kind, JobKind::ProcessDocument);
    assert_eq!(job.state, JobState::Queued);
    assert_eq!(job.attempts, 0);
    assert_eq!(job.max_attempts, 5);
    assert_eq!(job.priority, 0);
    assert_eq!(job.payload, json!({}));
    assert_eq!(job.progress, Progress::new(0, None).unwrap());
    assert!(job.leased_by.is_none() && job.lease_expires_at.is_none());
    assert!(job.started_at.is_none() && job.finished_at.is_none() && job.cancel_requested_at.is_none());
    assert!(job.last_error.is_none() && job.checkpoint.is_none() && job.progress_note.is_none());
    // The deadline is 48 hours out and the job is due now.
    let deadline = job.deadline_at - before;
    assert!(
        deadline > time::Duration::hours(47) && deadline < time::Duration::hours(49),
        "{deadline}"
    );
    assert!(job.run_after - before < time::Duration::seconds(30));
    // Enqueuing does not touch the document.
    assert_eq!(doc_status(&store, tenant, doc).await, DocumentStatus::Received);
    assert_eq!(audit_actions(&pool, id).await, ["job.enqueued"]);
    let who: serde_json::Value =
        sqlx::query_scalar("select actor from audit_log where subject_id = $1 and action = 'job.enqueued'")
            .bind(id.to_string())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(who, actor());
}

#[sqlx::test]
async fn the_options_of_a_new_job_are_kept(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let mut new = NewJob::new(doc, JobKind::ProcessDocument);
    new.priority = 7;
    new.max_attempts = 3;
    new.payload = json!({ "reason": "re-run" });
    new.delay = Duration::from_secs(3600);
    new.deadline = Duration::from_secs(2 * 3600);
    let before = db_now(&pool).await;
    let id = store.enqueue_job(tenant, &new, &actor()).await.unwrap().id;
    let job = store.get_job(tenant, id).await.unwrap();
    assert_eq!((job.priority, job.max_attempts), (7, 3));
    assert_eq!(job.payload, json!({ "reason": "re-run" }));
    assert!(job.run_after - before > time::Duration::minutes(59));
    assert!(job.deadline_at - before > time::Duration::minutes(119));
    assert!(job.deadline_at - before < time::Duration::minutes(121));
    // Not due yet, so nobody can claim it.
    assert!(claim(&store).await.is_none());
}

#[sqlx::test]
async fn enqueuing_the_same_work_twice_returns_the_job_already_waiting(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let first = store
        .enqueue_job(tenant, &NewJob::new(doc, JobKind::ProcessDocument), &actor())
        .await
        .unwrap();
    let second = store
        .enqueue_job(tenant, &NewJob::new(doc, JobKind::ProcessDocument), &actor())
        .await
        .unwrap();
    assert!(!first.already_active);
    assert!(second.already_active);
    assert_eq!(first.id, second.id);
    assert_eq!(count(&pool, "select count(*) from jobs").await, 1);
    assert_eq!(
        audit_actions(&pool, first.id).await,
        ["job.enqueued"],
        "audited once"
    );
    // Still the same while it is running.
    let _ = claim_expect(&store).await;
    let third = store
        .enqueue_job(tenant, &NewJob::new(doc, JobKind::ProcessDocument), &actor())
        .await
        .unwrap();
    assert!(third.already_active && third.id == first.id);
}

#[sqlx::test]
async fn once_a_job_has_finished_the_document_can_be_queued_again(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let first = enqueue(&store, tenant, doc).await;
    store.request_cancel(tenant, first, &actor()).await.unwrap();
    let again = store
        .enqueue_job(tenant, &NewJob::new(doc, JobKind::ProcessDocument), &actor())
        .await
        .unwrap();
    assert!(!again.already_active);
    assert_ne!(again.id, first);
    assert_eq!(store.jobs_for_document(tenant, doc).await.unwrap().len(), 2);
}

#[sqlx::test]
async fn a_job_for_a_document_of_another_tenant_is_refused(pool: PgPool) {
    let (store, _, doc) = setup(&pool).await;
    let other = new_tenant(&pool, "Other").await.unwrap();
    let refused = store
        .enqueue_job(other, &NewJob::new(doc, JobKind::ProcessDocument), &actor())
        .await;
    assert!(matches!(refused, Err(Error::DocumentNotFound)), "{refused:?}");
    assert_eq!(count(&pool, "select count(*) from jobs").await, 0);
}

#[sqlx::test]
async fn a_payload_must_be_a_small_object(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    for bad in [
        json!([1, 2]),
        json!("text"),
        json!(null),
        json!({ "blob": "x".repeat(20_000) }),
    ] {
        let mut new = NewJob::new(doc, JobKind::ProcessDocument);
        new.payload = bad.clone();
        assert!(store.enqueue_job(tenant, &new, &actor()).await.is_err(), "{bad}");
    }
    assert_eq!(count(&pool, "select count(*) from jobs").await, 0);
}

#[sqlx::test]
async fn another_tenant_cannot_see_a_job(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let id = enqueue(&store, tenant, doc).await;
    let other = new_tenant(&pool, "Other").await.unwrap();
    assert!(matches!(store.get_job(other, id).await, Err(Error::JobNotFound)));
    assert!(store.jobs_for_document(other, doc).await.unwrap().is_empty());
}

#[sqlx::test]
async fn jobs_for_a_document_come_newest_first(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let first = enqueue(&store, tenant, doc).await;
    store.request_cancel(tenant, first, &actor()).await.unwrap();
    let second = enqueue(&store, tenant, doc).await;
    let ids: Vec<_> = store
        .jobs_for_document(tenant, doc)
        .await
        .unwrap()
        .into_iter()
        .map(|job| job.id)
        .collect();
    assert_eq!(ids, [second, first]);
}

// ---------- what the application role may and may not do ----------

#[sqlx::test]
async fn the_application_role_can_neither_change_nor_remove_jobs_directly(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let id = enqueue(&store, tenant, doc).await;
    let mut tx = crate::tests::as_tenant(store.pool(), tenant).await;
    for statement in [
        "update jobs set state = 'succeeded'",
        "update jobs set attempts = 0",
        "update jobs set cancel_requested_at = now()",
        "delete from jobs",
    ] {
        let refused = sqlx::query(statement).execute(&mut *tx).await;
        assert!(db_message(refused).contains("permission denied"), "{statement}");
        tx = crate::tests::as_tenant(store.pool(), tenant).await;
    }
    drop(tx);
    assert_eq!(state_in_db(&pool, id).await, "queued");
}

#[sqlx::test]
async fn the_application_role_may_name_only_what_a_caller_decides_when_queuing_a_job(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    // Each of these is set by the database or a queue function, never by the caller.
    for (column, value) in [
        ("id", "gen_random_uuid()"),
        ("state", "'queued'"),
        ("attempts", "0"),
        ("lease_token", "gen_random_uuid()"),
        ("leased_by", "'me'"),
        ("lease_expires_at", "now() + interval '1 day'"),
        ("progress_done", "3"),
        ("progress_total", "3"),
        ("progress_note", "'x'"),
        ("checkpoint", "'{}'::jsonb"),
        ("last_error", "'x'"),
        ("cancel_requested_at", "now()"),
        ("created_at", "now()"),
        ("updated_at", "now()"),
        ("started_at", "now()"),
        ("finished_at", "now()"),
    ] {
        let mut tx = crate::tests::as_tenant(store.pool(), tenant).await;
        let sql = format!(
            "insert into jobs (tenant_id, document_id, kind, {column}) values ($1, $2, 'process_document', {value})"
        );
        let refused = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(tenant.0)
            .bind(doc.0)
            .execute(&mut *tx)
            .await;
        assert!(db_message(refused).contains("permission denied"), "{column}");
    }
    assert_eq!(count(&pool, "select count(*) from jobs").await, 0);
}

#[sqlx::test]
async fn the_application_role_cannot_read_a_lease_token(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    enqueue(&store, tenant, doc).await;
    let _ = claim_expect(&store).await;
    let mut tx = crate::tests::as_tenant(store.pool(), tenant).await;
    let refused = sqlx::query("select lease_token from jobs")
        .execute(&mut *tx)
        .await;
    assert!(db_message(refused).contains("permission denied"));
}

#[sqlx::test]
async fn the_application_role_cannot_write_a_job_for_another_tenant(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let other = new_tenant(&pool, "Other").await.unwrap();
    let mut tx = crate::tests::as_tenant(store.pool(), other).await;
    let refused =
        sqlx::query("insert into jobs (tenant_id, document_id, kind) values ($1, $2, 'process_document')")
            .bind(tenant.0)
            .bind(doc.0)
            .execute(&mut *tx)
            .await;
    assert!(db_message(refused).contains("row-level security"));
}

// ---------- the table's own rules ----------

#[sqlx::test]
async fn the_database_refuses_incoherent_rows(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let id = enqueue(&store, tenant, doc).await;
    for (what, statement) in [
        ("running without a lease", "update jobs set state = 'running' where id = $1"),
        ("a lease on a queued job", "update jobs set lease_token = gen_random_uuid(), leased_by = 'w', lease_expires_at = now() where id = $1"),
        ("finished but not terminal", "update jobs set finished_at = now() where id = $1"),
        ("terminal but not finished", "update jobs set state = 'succeeded' where id = $1"),
        ("progress ahead of its total", "update jobs set progress_done = 5, progress_total = 4 where id = $1"),
        ("no such state", "update jobs set state = 'failed' where id = $1"),
        ("zero attempts allowed", "update jobs set max_attempts = 0 where id = $1"),
        ("too many attempts allowed", "update jobs set max_attempts = 21 where id = $1"),
        ("a blank kind", "update jobs set kind = '  ' where id = $1"),
        ("an overlong error", "update jobs set last_error = repeat('e', 501) where id = $1"),
        ("an overlong note", "update jobs set progress_note = repeat('n', 201) where id = $1"),
    ] {
        let refused = sqlx::query(statement).bind(id).execute(&pool).await;
        assert!(refused.is_err(), "{what} should be refused");
    }
    assert_eq!(state_in_db(&pool, id).await, "queued");
}

// ---------- claiming ----------

#[sqlx::test]
async fn claiming_a_job_leases_it_and_marks_the_document_processing(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let id = enqueue(&store, tenant, doc).await;
    let before = db_now(&pool).await;
    let claimed = claim_expect(&store).await;
    assert_eq!(claimed.job.id, id);
    assert_eq!(claimed.job.tenant, tenant);
    assert_eq!(claimed.job.document, doc);
    assert_eq!(claimed.job.state, JobState::Running);
    assert_eq!(claimed.job.attempts, 1);
    assert_eq!(claimed.job.leased_by.as_deref(), Some(WORKER));
    let lease = claimed.job.lease_expires_at.unwrap() - before;
    assert!(
        lease > time::Duration::seconds(290) && lease < time::Duration::seconds(310),
        "{lease}"
    );
    assert!(claimed.job.started_at.is_some());
    assert_eq!(doc_status(&store, tenant, doc).await, DocumentStatus::Processing);
    assert!(
        claim(&store).await.is_none(),
        "a running job is not claimed again"
    );
}

#[sqlx::test]
async fn nothing_to_claim_is_none(pool: PgPool) {
    let (store, ..) = setup(&pool).await;
    assert!(claim(&store).await.is_none());
}

#[sqlx::test]
async fn the_highest_priority_goes_first_then_the_oldest_due_job(pool: PgPool) {
    let (store, tenant, _) = setup(&pool).await;
    let mut docs = Vec::new();
    for seed in 10..14u8 {
        docs.push(store.add_document(tenant, &new_document(seed)).await.unwrap().id);
    }
    let mut ids = Vec::new();
    for (doc, priority) in docs.iter().zip([0i16, 5, 0, 5]) {
        let mut new = NewJob::new(*doc, JobKind::ProcessDocument);
        new.priority = priority;
        ids.push(store.enqueue_job(tenant, &new, &actor()).await.unwrap().id);
    }
    let mut order = Vec::new();
    while let Some(claimed) = claim(&store).await {
        order.push(claimed.job.id);
    }
    assert_eq!(order, [ids[1], ids[3], ids[0], ids[2]]);
}

#[sqlx::test]
async fn only_due_jobs_of_the_wanted_kinds_are_claimed(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let id = enqueue(&store, tenant, doc).await;
    // A kind the worker does not handle (an empty list handles nothing).
    let none = store
        .claim_job(WORKER, LeasePolicy::default(), &[], None)
        .await
        .unwrap();
    assert!(none.is_none());
    sqlx::query("update jobs set run_after = now() + interval '1 hour' where id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(claim(&store).await.is_none(), "not due yet");
    make_due(&pool, id).await;
    assert!(claim(&store).await.is_some());
}

#[sqlx::test]
async fn a_runner_serves_every_tenant_and_each_job_knows_its_own(pool: PgPool) {
    let (store, tenant_a, doc_a) = setup(&pool).await;
    let tenant_b = new_tenant(&pool, "Tenant B").await.unwrap();
    let doc_b = store.add_document(tenant_b, &new_document(2)).await.unwrap().id;
    enqueue(&store, tenant_a, doc_a).await;
    enqueue(&store, tenant_b, doc_b).await;
    let first = claim_expect(&store).await;
    let second = claim_expect(&store).await;
    let mut seen = [
        (first.job.tenant, first.job.document),
        (second.job.tenant, second.job.document),
    ];
    seen.sort_by_key(|(t, _)| t.0);
    let mut expected = [(tenant_a, doc_a), (tenant_b, doc_b)];
    expected.sort_by_key(|(t, _)| t.0);
    assert_eq!(seen, expected);
    assert_eq!(
        doc_status(&store, tenant_a, doc_a).await,
        DocumentStatus::Processing
    );
    assert_eq!(
        doc_status(&store, tenant_b, doc_b).await,
        DocumentStatus::Processing
    );
}

#[sqlx::test]
async fn many_runners_never_claim_the_same_job_twice(pool: PgPool) {
    let (store, tenant, _) = setup(&pool).await;
    let mut ids = Vec::new();
    for seed in 20..26u8 {
        let doc = store.add_document(tenant, &new_document(seed)).await.unwrap().id;
        ids.push(enqueue(&store, tenant, doc).await);
    }
    let mut tasks = Vec::new();
    for n in 0..12 {
        let store = store.clone();
        tasks.push(tokio::spawn(async move {
            store
                .claim_job(
                    &format!("worker-{n}"),
                    LeasePolicy::default(),
                    &JobKind::ALL,
                    None,
                )
                .await
                .unwrap()
                .map(|claimed| claimed.job.id)
        }));
    }
    let mut got = Vec::new();
    for task in tasks {
        if let Some(id) = task.await.unwrap() {
            got.push(id);
        }
    }
    got.sort();
    let mut expected = ids.clone();
    expected.sort();
    assert_eq!(
        got, expected,
        "six jobs, each claimed by exactly one of twelve runners"
    );
}

#[sqlx::test]
async fn one_tenant_cannot_occupy_every_runner(pool: PgPool) {
    let (store, tenant_a, _) = setup(&pool).await;
    let tenant_b = new_tenant(&pool, "Tenant B").await.unwrap();
    for seed in 30..33u8 {
        let doc = store
            .add_document(tenant_a, &new_document(seed))
            .await
            .unwrap()
            .id;
        enqueue(&store, tenant_a, doc).await;
    }
    let doc_b = store.add_document(tenant_b, &new_document(40)).await.unwrap().id;
    enqueue(&store, tenant_b, doc_b).await;
    let capped = |store: Store| async move {
        store
            .claim_job(WORKER, LeasePolicy::default(), &JobKind::ALL, Some(1))
            .await
            .unwrap()
    };
    let first = capped(store.clone()).await.expect("first");
    let second = capped(store.clone()).await.expect("second");
    let tenants = [first.job.tenant, second.job.tenant];
    assert!(
        tenants.contains(&tenant_a) && tenants.contains(&tenant_b),
        "with a cap of one each, both tenants get a runner"
    );
    assert!(
        capped(store.clone()).await.is_none(),
        "both tenants are at their cap"
    );
    // Without a cap the waiting jobs of tenant A are available.
    assert!(claim(&store).await.is_some());
}

#[sqlx::test]
async fn a_job_whose_lease_expired_is_claimed_again_and_the_old_worker_is_locked_out(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let id = enqueue(&store, tenant, doc).await;
    let stale = claim_expect(&store).await;
    expire_lease(&pool, id).await;
    let fresh = claim_expect(&store).await;
    assert_eq!(fresh.job.id, id);
    assert_eq!(fresh.job.attempts, 2);
    assert_ne!(fresh.lease, stale.lease, "a new fencing token");

    // The first worker wakes up and tries to carry on: refused, and harmless.
    let heartbeat = store
        .heartbeat_job(
            &stale,
            LeasePolicy::default(),
            Progress::new(1, Some(2)).unwrap(),
            None,
            None,
        )
        .await;
    assert!(matches!(heartbeat, Err(Error::LeaseLost)), "{heartbeat:?}");
    let complete = store.complete_job(&stale, Outcome::Succeeded, None).await;
    assert!(matches!(complete, Err(Error::LeaseLost)), "{complete:?}");
    let fail = store
        .fail_job(
            &stale,
            FailureKind::Transient,
            "late",
            &RetryPolicy::default(),
            5000,
        )
        .await;
    assert!(matches!(fail, Err(Error::LeaseLost)), "{fail:?}");
    assert_eq!(state_in_db(&pool, id).await, "running");
    assert_eq!(doc_status(&store, tenant, doc).await, DocumentStatus::Processing);

    // The new holder is unaffected.
    store
        .complete_job(&fresh, Outcome::Succeeded, None)
        .await
        .unwrap();
    assert_eq!(state_in_db(&pool, id).await, "succeeded");
}

#[sqlx::test]
async fn a_worker_lost_on_its_final_attempt_hands_the_document_to_a_person(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let mut new = NewJob::new(doc, JobKind::ProcessDocument);
    new.max_attempts = 2;
    let id = store.enqueue_job(tenant, &new, &actor()).await.unwrap().id;
    claim_expect(&store).await;
    expire_lease(&pool, id).await;
    claim_expect(&store).await; // attempt 2 of 2
    expire_lease(&pool, id).await;
    assert!(claim(&store).await.is_none(), "out of attempts: swept, not run");
    let job = store.get_job(tenant, id).await.unwrap();
    assert_eq!(job.state, JobState::NeedsReview);
    assert!(job.finished_at.is_some() && job.leased_by.is_none());
    assert!(job.last_error.unwrap().contains("lost"));
    assert_eq!(doc_status(&store, tenant, doc).await, DocumentStatus::NeedsReview);
    assert_eq!(
        audit_actions(&pool, id).await,
        ["job.enqueued", "job.needs_review"]
    );
}

#[sqlx::test]
async fn a_job_past_its_deadline_is_handed_to_a_person_not_run(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let id = enqueue(&store, tenant, doc).await;
    sqlx::query("update jobs set deadline_at = now() - interval '1 second' where id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(claim(&store).await.is_none());
    let job = store.get_job(tenant, id).await.unwrap();
    assert_eq!(job.state, JobState::NeedsReview);
    assert!(job.last_error.unwrap().contains("deadline"));
    assert_eq!(doc_status(&store, tenant, doc).await, DocumentStatus::NeedsReview);
}

#[sqlx::test]
async fn sweeping_a_dead_job_does_not_stop_the_next_job_being_claimed(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let doc2 = store.add_document(tenant, &new_document(2)).await.unwrap().id;
    let dead = enqueue(&store, tenant, doc).await;
    let live = enqueue(&store, tenant, doc2).await;
    sqlx::query("update jobs set deadline_at = now() - interval '1 second' where id = $1")
        .bind(dead)
        .execute(&pool)
        .await
        .unwrap();
    let claimed = claim_expect(&store).await;
    assert_eq!(claimed.job.id, live);
    assert_eq!(state_in_db(&pool, dead).await, "needs_review");
}

#[sqlx::test]
async fn a_cancel_requested_while_the_worker_vanished_cancels_the_job(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let id = enqueue(&store, tenant, doc).await;
    claim_expect(&store).await;
    assert_eq!(
        store.request_cancel(tenant, id, &actor()).await.unwrap(),
        JobState::Running
    );
    expire_lease(&pool, id).await;
    assert!(claim(&store).await.is_none(), "cancelled, not restarted");
    assert_eq!(state_in_db(&pool, id).await, "cancelled");
    assert_eq!(doc_status(&store, tenant, doc).await, DocumentStatus::Received);
}

// ---------- heartbeats ----------

#[sqlx::test]
async fn a_heartbeat_extends_the_lease_and_records_progress_and_a_checkpoint(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let id = enqueue(&store, tenant, doc).await;
    let claimed = claim_expect(&store).await;
    sqlx::query("update jobs set lease_expires_at = now() + interval '5 seconds' where id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    let before = db_now(&pool).await;
    let beat = store
        .heartbeat_job(
            &claimed,
            LeasePolicy::default(),
            Progress::new(3, Some(12)).unwrap(),
            Some("page 3 of 12"),
            Some(&json!({ "next_page": 4 })),
        )
        .await
        .unwrap();
    assert!(!beat.cancel_requested);
    let job = store.get_job(tenant, id).await.unwrap();
    assert!(job.lease_expires_at.unwrap() - before > time::Duration::seconds(290));
    assert_eq!(job.progress, Progress::new(3, Some(12)).unwrap());
    assert_eq!(job.progress_note.as_deref(), Some("page 3 of 12"));
    assert_eq!(job.checkpoint, Some(json!({ "next_page": 4 })));

    // A later heartbeat without a checkpoint or note keeps what was stored.
    store
        .heartbeat_job(
            &claimed,
            LeasePolicy::default(),
            Progress::new(4, Some(12)).unwrap(),
            None,
            None,
        )
        .await
        .unwrap();
    let job = store.get_job(tenant, id).await.unwrap();
    assert_eq!(job.progress.done(), 4);
    assert_eq!(job.checkpoint, Some(json!({ "next_page": 4 })));
    assert_eq!(job.progress_note.as_deref(), Some("page 3 of 12"));
}

#[sqlx::test]
async fn a_long_progress_note_is_trimmed_not_refused(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let id = enqueue(&store, tenant, doc).await;
    let claimed = claim_expect(&store).await;
    store
        .heartbeat_job(
            &claimed,
            LeasePolicy::default(),
            Progress::new(1, None).unwrap(),
            Some(&"n".repeat(1000)),
            None,
        )
        .await
        .unwrap();
    let note = store.get_job(tenant, id).await.unwrap().progress_note.unwrap();
    assert_eq!(note.chars().count(), 200);
}

#[sqlx::test]
async fn a_worker_that_was_slow_but_not_replaced_can_carry_on(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let id = enqueue(&store, tenant, doc).await;
    let claimed = claim_expect(&store).await;
    expire_lease(&pool, id).await; // late, but nobody else has taken it
    store
        .heartbeat_job(
            &claimed,
            LeasePolicy::default(),
            Progress::new(1, None).unwrap(),
            None,
            None,
        )
        .await
        .unwrap();
    assert!(claim(&store).await.is_none(), "the revived lease is respected");
    store
        .complete_job(&claimed, Outcome::Succeeded, None)
        .await
        .unwrap();
}

#[sqlx::test]
async fn a_heartbeat_tells_the_worker_when_cancellation_was_requested(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let id = enqueue(&store, tenant, doc).await;
    let claimed = claim_expect(&store).await;
    assert_eq!(
        store.request_cancel(tenant, id, &actor()).await.unwrap(),
        JobState::Running
    );
    let beat = store
        .heartbeat_job(
            &claimed,
            LeasePolicy::default(),
            Progress::new(1, None).unwrap(),
            None,
            None,
        )
        .await
        .unwrap();
    assert!(beat.cancel_requested);
    assert_eq!(
        state_in_db(&pool, id).await,
        "running",
        "the worker decides when to stop"
    );
}

#[sqlx::test]
async fn progress_ahead_of_its_total_is_refused_by_the_core_before_the_database(pool: PgPool) {
    let _ = setup(&pool).await;
    assert!(Progress::new(5, Some(4)).is_err());
}

#[sqlx::test]
async fn a_tenant_scoped_call_for_another_tenant_is_refused_by_the_database(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let other = new_tenant(&pool, "Other").await.unwrap();
    let id = enqueue(&store, tenant, doc).await;
    let claimed = claim_expect(&store).await;
    // The caller says it acts for `other` but names `tenant`'s job.
    let mut tx = store.begin(other).await.unwrap();
    let refused = sqlx::query("select heartbeat_job($1, $2, $3, 300, 0, null, null, null)")
        .bind(tenant.0)
        .bind(id)
        .bind(claimed.lease.0)
        .execute(&mut *tx)
        .await;
    assert!(db_message(refused).contains("tenant does not match"));
}

// ---------- completing ----------

#[sqlx::test]
async fn success_finishes_the_job_and_the_document(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let id = enqueue(&store, tenant, doc).await;
    let claimed = claim_expect(&store).await;
    store
        .complete_job(&claimed, Outcome::Succeeded, None)
        .await
        .unwrap();
    let job = store.get_job(tenant, id).await.unwrap();
    assert_eq!(job.state, JobState::Succeeded);
    assert!(job.finished_at.is_some());
    assert!(job.leased_by.is_none() && job.lease_expires_at.is_none());
    assert_eq!(doc_status(&store, tenant, doc).await, DocumentStatus::Done);
    assert_eq!(audit_actions(&pool, id).await, ["job.enqueued", "job.succeeded"]);
}

#[sqlx::test]
async fn needs_review_keeps_a_short_reason_and_flags_the_document(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let id = enqueue(&store, tenant, doc).await;
    let claimed = claim_expect(&store).await;
    store
        .complete_job(
            &claimed,
            Outcome::NeedsReview,
            Some(&format!("totals disagree\n{}", "x".repeat(900))),
        )
        .await
        .unwrap();
    let job = store.get_job(tenant, id).await.unwrap();
    assert_eq!(job.state, JobState::NeedsReview);
    let reason = job.last_error.unwrap();
    assert!(reason.starts_with("totals disagree x"));
    assert_eq!(reason.chars().count(), 500);
    assert_eq!(doc_status(&store, tenant, doc).await, DocumentStatus::NeedsReview);
}

#[sqlx::test]
async fn acknowledging_a_cancellation_returns_the_document_to_received(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let id = enqueue(&store, tenant, doc).await;
    let claimed = claim_expect(&store).await;
    store.request_cancel(tenant, id, &actor()).await.unwrap();
    store
        .complete_job(&claimed, Outcome::Cancelled, None)
        .await
        .unwrap();
    assert_eq!(state_in_db(&pool, id).await, "cancelled");
    assert_eq!(doc_status(&store, tenant, doc).await, DocumentStatus::Received);
    assert_eq!(
        audit_actions(&pool, id).await,
        ["job.enqueued", "job.cancel_requested", "job.cancelled"]
    );
}

#[sqlx::test]
async fn a_job_can_only_be_finished_once(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    enqueue(&store, tenant, doc).await;
    let claimed = claim_expect(&store).await;
    store
        .complete_job(&claimed, Outcome::Succeeded, None)
        .await
        .unwrap();
    for outcome in [Outcome::Succeeded, Outcome::NeedsReview, Outcome::Cancelled] {
        let again = store.complete_job(&claimed, outcome, None).await;
        assert!(matches!(again, Err(Error::LeaseLost)), "{outcome:?}: {again:?}");
    }
    assert_eq!(doc_status(&store, tenant, doc).await, DocumentStatus::Done);
}

// ---------- failing ----------

#[sqlx::test]
async fn a_transient_failure_is_retried_later_and_counts(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let id = enqueue(&store, tenant, doc).await;
    let claimed = claim_expect(&store).await;
    let before = db_now(&pool).await;
    let decision = store
        .fail_job(
            &claimed,
            FailureKind::Transient,
            "model timed out",
            &RetryPolicy::default(),
            5000,
        )
        .await
        .unwrap();
    assert_eq!(
        decision,
        FailureResult::Retrying {
            after: Duration::from_secs(30),
            counts: true
        }
    );
    let job = store.get_job(tenant, id).await.unwrap();
    assert_eq!(job.state, JobState::Queued);
    assert_eq!(job.attempts, 1, "the failed run still counts");
    assert!(job.leased_by.is_none() && job.lease_expires_at.is_none());
    assert_eq!(job.last_error.as_deref(), Some("model timed out"));
    let wait = job.run_after - before;
    assert!(
        wait > time::Duration::seconds(28) && wait < time::Duration::seconds(32),
        "{wait}"
    );
    assert_eq!(doc_status(&store, tenant, doc).await, DocumentStatus::Processing);
    assert!(claim(&store).await.is_none(), "not before its retry time");
    make_due(&pool, id).await;
    let again = claim_expect(&store).await;
    assert_eq!(again.job.attempts, 2);
}

#[sqlx::test]
async fn a_long_error_message_is_trimmed_so_it_cannot_block_the_failure_being_recorded(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let id = enqueue(&store, tenant, doc).await;
    let claimed = claim_expect(&store).await;
    store
        .fail_job(
            &claimed,
            FailureKind::Transient,
            &format!("boom\u{7}\n{}", "é".repeat(5000)),
            &RetryPolicy::default(),
            5000,
        )
        .await
        .unwrap();
    let error = store.get_job(tenant, id).await.unwrap().last_error.unwrap();
    assert_eq!(error.chars().count(), 500);
    assert!(error.starts_with("boom é"));
}

#[sqlx::test]
async fn a_provider_outage_never_uses_up_attempts(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let id = enqueue(&store, tenant, doc).await;
    for round in 0..12 {
        make_due(&pool, id).await;
        let claimed = claim_expect(&store).await;
        assert_eq!(claimed.job.attempts, 1, "round {round}");
        let decision = store
            .fail_job(
                &claimed,
                FailureKind::ProviderUnavailable,
                "503 from the provider",
                &RetryPolicy::default(),
                5000,
            )
            .await
            .unwrap();
        assert_eq!(
            decision,
            FailureResult::Retrying {
                after: Duration::from_secs(300),
                counts: false
            }
        );
        let job = store.get_job(tenant, id).await.unwrap();
        assert_eq!((job.state, job.attempts), (JobState::Queued, 0), "round {round}");
    }
}

#[sqlx::test]
async fn ordinary_failures_end_in_review_when_the_attempts_run_out(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let id = enqueue(&store, tenant, doc).await;
    let policy = RetryPolicy::default();
    for attempt in 1..=5 {
        make_due(&pool, id).await;
        let claimed = claim_expect(&store).await;
        assert_eq!(claimed.job.attempts, attempt);
        let decision = store
            .fail_job(&claimed, FailureKind::Transient, "bad output", &policy, 5000)
            .await
            .unwrap();
        if attempt < 5 {
            assert!(
                matches!(decision, FailureResult::Retrying { counts: true, .. }),
                "attempt {attempt}"
            );
            assert_eq!(state_in_db(&pool, id).await, "queued");
        } else {
            assert_eq!(decision, FailureResult::NeedsReview);
        }
    }
    let job = store.get_job(tenant, id).await.unwrap();
    assert_eq!(job.state, JobState::NeedsReview);
    assert_eq!(job.last_error.as_deref(), Some("bad output"));
    assert_eq!(doc_status(&store, tenant, doc).await, DocumentStatus::NeedsReview);
    assert!(claim(&store).await.is_none());
}

#[sqlx::test]
async fn a_failure_the_top_tier_could_not_fix_goes_straight_to_review(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let id = enqueue(&store, tenant, doc).await;
    let claimed = claim_expect(&store).await;
    let decision = store
        .fail_job(
            &claimed,
            FailureKind::NeedsReview,
            "totals never reconcile",
            &RetryPolicy::default(),
            5000,
        )
        .await
        .unwrap();
    assert_eq!(decision, FailureResult::NeedsReview);
    assert_eq!(state_in_db(&pool, id).await, "needs_review");
    assert_eq!(doc_status(&store, tenant, doc).await, DocumentStatus::NeedsReview);
}

#[sqlx::test]
async fn the_database_itself_ends_a_job_that_is_out_of_attempts(pool: PgPool) {
    // A caller (or a bug) that asks for a retry the job is not entitled to.
    let (store, tenant, doc) = setup(&pool).await;
    let mut new = NewJob::new(doc, JobKind::ProcessDocument);
    new.max_attempts = 1;
    let id = store.enqueue_job(tenant, &new, &actor()).await.unwrap().id;
    let claimed = claim_expect(&store).await;
    let mut tx = store.begin(tenant).await.unwrap();
    sqlx::query("select fail_job($1, $2, $3, 30, true, 'again please')")
        .bind(tenant.0)
        .bind(id)
        .bind(claimed.lease.0)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(state_in_db(&pool, id).await, "needs_review");
    assert_eq!(doc_status(&store, tenant, doc).await, DocumentStatus::NeedsReview);
}

#[sqlx::test]
async fn a_cancelled_job_is_not_retried(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let id = enqueue(&store, tenant, doc).await;
    let claimed = claim_expect(&store).await;
    store.request_cancel(tenant, id, &actor()).await.unwrap();
    let result = store
        .fail_job(
            &claimed,
            FailureKind::Transient,
            "timeout",
            &RetryPolicy::default(),
            5000,
        )
        .await
        .unwrap();
    assert_eq!(result, FailureResult::Cancelled);
    assert_eq!(state_in_db(&pool, id).await, "cancelled");
    assert_eq!(doc_status(&store, tenant, doc).await, DocumentStatus::Received);
}

#[sqlx::test]
async fn a_failure_after_the_deadline_goes_to_review_not_back_in_the_queue(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let id = enqueue(&store, tenant, doc).await;
    let claimed = claim_expect(&store).await;
    sqlx::query("update jobs set deadline_at = now() - interval '1 second' where id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    let result = store
        .fail_job(
            &claimed,
            FailureKind::ProviderUnavailable,
            "still down",
            &RetryPolicy::default(),
            5000,
        )
        .await
        .unwrap();
    assert_eq!(result, FailureResult::NeedsReview);
    assert_eq!(state_in_db(&pool, id).await, "needs_review");
    assert_eq!(doc_status(&store, tenant, doc).await, DocumentStatus::NeedsReview);
}

// ---------- cancelling ----------

#[sqlx::test]
async fn a_queued_job_is_cancelled_at_once(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let id = enqueue(&store, tenant, doc).await;
    assert_eq!(
        store.request_cancel(tenant, id, &actor()).await.unwrap(),
        JobState::Cancelled
    );
    let job = store.get_job(tenant, id).await.unwrap();
    assert_eq!(job.state, JobState::Cancelled);
    assert!(job.finished_at.is_some());
    assert!(claim(&store).await.is_none());
    assert_eq!(audit_actions(&pool, id).await, ["job.enqueued", "job.cancelled"]);
    // The audit entry names who asked.
    let who: serde_json::Value =
        sqlx::query_scalar("select actor from audit_log where subject_id = $1 and action = 'job.cancelled'")
            .bind(id.to_string())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(who, actor());
}

#[sqlx::test]
async fn cancelling_a_job_that_never_started_leaves_its_document_alone(pool: PgPool) {
    // A finished document queued again for a re-run: cancelling the re-run
    // must not take away its "done".
    let (store, tenant, doc) = setup(&pool).await;
    let first = enqueue(&store, tenant, doc).await;
    let claimed = claim_expect(&store).await;
    store
        .complete_job(&claimed, Outcome::Succeeded, None)
        .await
        .unwrap();
    assert_eq!(doc_status(&store, tenant, doc).await, DocumentStatus::Done);
    let rerun = enqueue(&store, tenant, doc).await;
    assert_ne!(rerun, first);
    store.request_cancel(tenant, rerun, &actor()).await.unwrap();
    assert_eq!(doc_status(&store, tenant, doc).await, DocumentStatus::Done);
}

#[sqlx::test]
async fn cancelling_a_running_job_only_asks_it_to_stop(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let id = enqueue(&store, tenant, doc).await;
    claim_expect(&store).await;
    assert_eq!(
        store.request_cancel(tenant, id, &actor()).await.unwrap(),
        JobState::Running
    );
    let job = store.get_job(tenant, id).await.unwrap();
    assert_eq!(job.state, JobState::Running);
    assert!(job.cancel_requested_at.is_some());
    // Asking again changes nothing and is not audited twice.
    let first_request = job.cancel_requested_at;
    assert_eq!(
        store.request_cancel(tenant, id, &actor()).await.unwrap(),
        JobState::Running
    );
    assert_eq!(
        store.get_job(tenant, id).await.unwrap().cancel_requested_at,
        first_request
    );
    assert_eq!(
        audit_actions(&pool, id).await,
        ["job.enqueued", "job.cancel_requested"]
    );
    let who: serde_json::Value = sqlx::query_scalar(
        "select actor from audit_log where subject_id = $1 and action = 'job.cancel_requested'",
    )
    .bind(id.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(who, actor());
}

#[sqlx::test]
async fn a_finished_job_cannot_be_cancelled(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let id = enqueue(&store, tenant, doc).await;
    let claimed = claim_expect(&store).await;
    store
        .complete_job(&claimed, Outcome::Succeeded, None)
        .await
        .unwrap();
    assert_eq!(
        store.request_cancel(tenant, id, &actor()).await.unwrap(),
        JobState::Succeeded
    );
    assert_eq!(state_in_db(&pool, id).await, "succeeded");
    assert_eq!(doc_status(&store, tenant, doc).await, DocumentStatus::Done);
}

#[sqlx::test]
async fn another_tenant_cannot_cancel_a_job(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let id = enqueue(&store, tenant, doc).await;
    let other = new_tenant(&pool, "Other").await.unwrap();
    assert!(matches!(
        store.request_cancel(other, id, &actor()).await,
        Err(Error::JobNotFound)
    ));
    assert_eq!(state_in_db(&pool, id).await, "queued");
}

// ---------- the database agrees with the core's rules ----------

/// Everything a worker or the API can attempt on a job.
#[derive(Debug, Clone, Copy)]
enum Operation {
    Claim,
    Heartbeat,
    Complete(Outcome),
    Fail(FailureKind),
    Cancel,
}

const OPERATIONS: [Operation; 9] = [
    Operation::Claim,
    Operation::Heartbeat,
    Operation::Complete(Outcome::Succeeded),
    Operation::Complete(Outcome::NeedsReview),
    Operation::Complete(Outcome::Cancelled),
    Operation::Fail(FailureKind::Transient),
    Operation::Fail(FailureKind::ProviderUnavailable),
    Operation::Fail(FailureKind::NeedsReview),
    Operation::Cancel,
];

/// A fresh job (on a document of its own, so jobs do not interfere) taken to
/// `state` through the real API, with the claim a worker would be holding.
/// For a queued job the claim is a stale one: the lease it names has been
/// given up, so a late worker holding it must be refused.
async fn job_in_state(
    pool: &PgPool,
    store: &Store,
    tenant: TenantId,
    state: JobState,
    seed: u8,
) -> (Uuid, ClaimedJob) {
    let doc = store.add_document(tenant, &new_document(seed)).await.unwrap().id;
    let id = enqueue(store, tenant, doc).await;
    // Only this job may be claimed: put every other waiting job off.
    sqlx::query("update jobs set run_after = now() + interval '1 hour' where state = 'queued' and id <> $1")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    let claimed = claim_expect(store).await;
    assert_eq!(claimed.job.id, id);
    match state {
        JobState::Running => {}
        JobState::Queued => {
            let policy = RetryPolicy::default();
            store
                .fail_job(&claimed, FailureKind::ProviderUnavailable, "down", &policy, 5000)
                .await
                .unwrap();
            make_due(pool, id).await;
        }
        JobState::Succeeded => store
            .complete_job(&claimed, Outcome::Succeeded, None)
            .await
            .unwrap(),
        JobState::NeedsReview => store
            .complete_job(&claimed, Outcome::NeedsReview, Some("x"))
            .await
            .unwrap(),
        JobState::Cancelled => store
            .complete_job(&claimed, Outcome::Cancelled, None)
            .await
            .unwrap(),
    }
    assert_eq!(state_in_db(pool, id).await, state.as_str());
    (id, claimed)
}

#[sqlx::test]
async fn every_operation_only_makes_moves_the_core_allows(pool: PgPool) {
    let (store, tenant, _) = setup(&pool).await;
    let policy = RetryPolicy::default();
    let mut seed = 100u8;
    for start in JobState::ALL {
        for operation in OPERATIONS {
            seed += 1;
            let (id, held) = job_in_state(&pool, &store, tenant, start, seed).await;
            // The operation may succeed or be refused; both are fine. What
            // matters is the move the database ends up making.
            let _ = match operation {
                Operation::Claim => store
                    .claim_job(WORKER, LeasePolicy::default(), &JobKind::ALL, None)
                    .await
                    .map(|_| ()),
                Operation::Heartbeat => store
                    .heartbeat_job(
                        &held,
                        LeasePolicy::default(),
                        Progress::new(1, None).unwrap(),
                        None,
                        None,
                    )
                    .await
                    .map(|_| ()),
                Operation::Complete(outcome) => store.complete_job(&held, outcome, None).await,
                Operation::Fail(kind) => store.fail_job(&held, kind, "x", &policy, 5000).await.map(|_| ()),
                Operation::Cancel => store.request_cancel(tenant, id, &actor()).await.map(|_| ()),
            };
            let end = JobState::parse(&state_in_db(&pool, id).await).unwrap();
            assert!(
                end == start || start.can_move_to(end),
                "{operation:?} on a {start:?} job left it {end:?}, which the core does not allow"
            );
            if start.is_terminal() {
                assert_eq!(end, start, "{operation:?} must not move a finished job");
            }
            // A stale lease holder can never change a job.
            if matches!(start, JobState::Queued) && !matches!(operation, Operation::Claim | Operation::Cancel)
            {
                assert_eq!(
                    end,
                    JobState::Queued,
                    "{operation:?} with a stale lease must be refused"
                );
            }
        }
    }
}

// ---------- more guarantees ----------

/// The SQLSTATE of a refused statement.
fn sqlstate(result: std::result::Result<sqlx::postgres::PgQueryResult, sqlx::Error>) -> String {
    match result.expect_err("statement should have been refused") {
        sqlx::Error::Database(error) => error.code().map(|c| c.to_string()).unwrap_or_default(),
        other => panic!("not a database error: {other}"),
    }
}

#[sqlx::test]
async fn a_job_another_transaction_is_working_on_is_skipped_not_waited_for(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let doc2 = store.add_document(tenant, &new_document(2)).await.unwrap().id;
    let mut urgent = NewJob::new(doc, JobKind::ProcessDocument);
    urgent.priority = 5;
    let first = store.enqueue_job(tenant, &urgent, &actor()).await.unwrap().id;
    let second = enqueue(&store, tenant, doc2).await;

    // Another runner is in the middle of claiming the best job (its row is locked).
    let mut other_runner = pool.begin().await.unwrap();
    sqlx::query("select id from jobs where id = $1 for update")
        .bind(first)
        .fetch_one(&mut *other_runner)
        .await
        .unwrap();

    let claimed = tokio::time::timeout(Duration::from_secs(10), claim(&store))
        .await
        .expect("a claim must move on from a locked row, not wait for it")
        .expect("the other job is free");
    assert_eq!(claimed.job.id, second);
}

#[sqlx::test]
async fn the_database_refuses_a_lease_too_short_to_heartbeat_and_a_blank_worker_name(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let id = enqueue(&store, tenant, doc).await;
    for (worker, seconds) in [
        ("worker", 2),
        ("worker", 0),
        ("worker", -5),
        ("", 300),
        ("   ", 300),
    ] {
        let refused = sqlx::query("select * from claim_job($1, $2, array['process_document'], null)")
            .bind(worker)
            .bind(seconds)
            .execute(store.pool())
            .await;
        assert_eq!(sqlstate(refused), "22023", "{worker:?} {seconds}");
    }
    let claimed = claim_expect(&store).await;
    let mut tx = store.begin(tenant).await.unwrap();
    let refused = sqlx::query("select heartbeat_job($1, $2, $3, 2, 0, null, null, null)")
        .bind(tenant.0)
        .bind(id)
        .bind(claimed.lease.0)
        .execute(&mut *tx)
        .await;
    assert_eq!(sqlstate(refused), "22023");
}

#[sqlx::test]
async fn the_database_refuses_an_unknown_outcome_or_an_unreasonable_retry_delay(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let id = enqueue(&store, tenant, doc).await;
    let claimed = claim_expect(&store).await;
    for outcome in ["bogus", "queued", "running", "failed", ""] {
        let mut tx = store.begin(tenant).await.unwrap();
        let refused = sqlx::query("select complete_job($1, $2, $3, $4, null)")
            .bind(tenant.0)
            .bind(id)
            .bind(claimed.lease.0)
            .bind(outcome)
            .execute(&mut *tx)
            .await;
        assert_eq!(sqlstate(refused), "22023", "{outcome:?}");
    }
    for seconds in [-1, 86_401, i32::MAX] {
        let mut tx = store.begin(tenant).await.unwrap();
        let refused = sqlx::query("select fail_job($1, $2, $3, $4, true, 'x')")
            .bind(tenant.0)
            .bind(id)
            .bind(claimed.lease.0)
            .bind(seconds)
            .execute(&mut *tx)
            .await;
        assert_eq!(sqlstate(refused), "22023", "{seconds}");
    }
    assert_eq!(state_in_db(&pool, id).await, "running");
}

#[sqlx::test]
async fn a_missing_token_cannot_finish_a_job_that_is_not_running(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let id = enqueue(&store, tenant, doc).await; // queued: it has no token either
    for statement in [
        "select complete_job($1, $2, $3, 'succeeded', null)",
        "select complete_job($1, $2, $3, 'needs_review', null)",
        "select complete_job($1, $2, $3, 'cancelled', null)",
        "select fail_job($1, $2, $3, 30, true, 'x')",
        "select heartbeat_job($1, $2, $3, 300, 0, null, null, null)",
    ] {
        let mut tx = store.begin(tenant).await.unwrap();
        let refused = sqlx::query(statement)
            .bind(tenant.0)
            .bind(id)
            .bind(None::<Uuid>)
            .execute(&mut *tx)
            .await;
        assert_eq!(sqlstate(refused), "CL001", "{statement}");
    }
    assert_eq!(state_in_db(&pool, id).await, "queued");
    assert_eq!(doc_status(&store, tenant, doc).await, DocumentStatus::Received);
}

#[sqlx::test]
async fn a_lease_token_does_not_open_another_tenants_job(pool: PgPool) {
    let (store, tenant_a, doc_a) = setup(&pool).await;
    let tenant_b = new_tenant(&pool, "Tenant B").await.unwrap();
    let doc_b = store.add_document(tenant_b, &new_document(2)).await.unwrap().id;
    enqueue(&store, tenant_a, doc_a).await;
    enqueue(&store, tenant_b, doc_b).await;
    let one = claim_expect(&store).await;
    let two = claim_expect(&store).await;
    let theirs = if one.job.tenant == tenant_b { one } else { two };
    assert_eq!(theirs.job.tenant, tenant_b);

    // Tenant A has somehow learned B's job id and token, and presents them as its own.
    let mut forged = theirs.clone();
    forged.job.tenant = tenant_a;
    let heartbeat = store
        .heartbeat_job(
            &forged,
            LeasePolicy::default(),
            Progress::new(1, None).unwrap(),
            None,
            None,
        )
        .await;
    assert!(matches!(heartbeat, Err(Error::JobNotFound)), "{heartbeat:?}");
    let complete = store.complete_job(&forged, Outcome::Succeeded, None).await;
    assert!(matches!(complete, Err(Error::JobNotFound)), "{complete:?}");
    let fail = store
        .fail_job(
            &forged,
            FailureKind::Transient,
            "x",
            &RetryPolicy::default(),
            5000,
        )
        .await;
    assert!(matches!(fail, Err(Error::JobNotFound)), "{fail:?}");
    assert_eq!(state_in_db(&pool, theirs.job.id).await, "running");
}

#[sqlx::test]
async fn cancelling_for_a_tenant_other_than_the_current_one_is_refused_by_the_database(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let other = new_tenant(&pool, "Other").await.unwrap();
    let id = enqueue(&store, tenant, doc).await;
    let mut tx = store.begin(other).await.unwrap();
    let refused = sqlx::query("select cancel_job($1, $2, '{}')")
        .bind(tenant.0)
        .bind(id)
        .execute(&mut *tx)
        .await;
    assert!(db_message(refused).contains("tenant does not match"));
    assert_eq!(state_in_db(&pool, id).await, "queued");
}

#[sqlx::test]
async fn row_level_security_alone_hides_one_tenants_jobs_from_another(pool: PgPool) {
    let (store, tenant_a, doc_a) = setup(&pool).await;
    let tenant_b = new_tenant(&pool, "Tenant B").await.unwrap();
    let doc_b = store.add_document(tenant_b, &new_document(2)).await.unwrap().id;
    enqueue(&store, tenant_a, doc_a).await;
    enqueue(&store, tenant_b, doc_b).await;
    // A deliberately sloppy query that forgets to filter by tenant.
    for tenant in [tenant_a, tenant_b] {
        let mut tx = crate::tests::as_tenant(store.pool(), tenant).await;
        let seen: i64 = sqlx::query_scalar("select count(*) from jobs")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        assert_eq!(seen, 1);
    }
    let seen: i64 = sqlx::query_scalar("select count(*) from jobs")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(seen, 0, "with no tenant set, nothing is visible");
}

#[sqlx::test]
async fn an_outage_on_the_final_attempt_still_does_not_end_the_job(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let mut new = NewJob::new(doc, JobKind::ProcessDocument);
    new.max_attempts = 1;
    let id = store.enqueue_job(tenant, &new, &actor()).await.unwrap().id;
    let claimed = claim_expect(&store).await;
    assert_eq!(claimed.job.attempts, 1); // the only attempt the job has
    let result = store
        .fail_job(
            &claimed,
            FailureKind::ProviderUnavailable,
            "down",
            &RetryPolicy::default(),
            5000,
        )
        .await
        .unwrap();
    assert!(matches!(result, FailureResult::Retrying { counts: false, .. }));
    let job = store.get_job(tenant, id).await.unwrap();
    assert_eq!((job.state, job.attempts), (JobState::Queued, 0));
}

#[sqlx::test]
async fn the_last_failure_message_stays_on_the_job_after_a_later_success(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let id = enqueue(&store, tenant, doc).await;
    let claimed = claim_expect(&store).await;
    store
        .fail_job(
            &claimed,
            FailureKind::Transient,
            "model timed out",
            &RetryPolicy::default(),
            5000,
        )
        .await
        .unwrap();
    make_due(&pool, id).await;
    let again = claim_expect(&store).await;
    store
        .complete_job(&again, Outcome::Succeeded, None)
        .await
        .unwrap();
    let job = store.get_job(tenant, id).await.unwrap();
    assert_eq!(job.state, JobState::Succeeded);
    assert_eq!(job.last_error.as_deref(), Some("model timed out"));
}

// ---------- erasure ----------

#[sqlx::test]
async fn erasing_a_document_erases_its_jobs_and_only_its_jobs(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let other_doc = store.add_document(tenant, &new_document(2)).await.unwrap().id;
    let finished = enqueue(&store, tenant, doc).await;
    let claimed = claim_expect(&store).await;
    assert_eq!(claimed.job.id, finished);
    store
        .complete_job(&claimed, Outcome::Succeeded, None)
        .await
        .unwrap();
    // A job still waiting makes the erasure fail (see below), so this one is cancelled first.
    let queued = enqueue(&store, tenant, doc).await;
    store
        .request_cancel(tenant, queued, &serde_json::json!({}))
        .await
        .unwrap();
    let kept = enqueue(&store, tenant, other_doc).await;

    store.erase_document(tenant, doc).await.unwrap();

    for gone in [finished, queued] {
        assert!(matches!(
            store.get_job(tenant, gone).await,
            Err(Error::JobNotFound)
        ));
    }
    assert_eq!(store.get_job(tenant, kept).await.unwrap().document, other_doc);
    assert_eq!(count(&pool, "select count(*) from jobs").await, 1);
    // The audit trail of the erasure survives and holds no invoice content.
    assert!(audit_actions(&pool, finished)
        .await
        .contains(&"job.succeeded".to_string()));
}

#[sqlx::test]
async fn a_document_cannot_be_erased_from_under_its_worker(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    enqueue(&store, tenant, doc).await;
    let claimed = claim_expect(&store).await;
    // A running job: the erasure is refused and rolled back, so the worker's lease and the
    // document are untouched (it used to succeed and leave the worker writing into nothing).
    assert!(matches!(
        store.erase_document(tenant, doc).await,
        Err(Error::DocumentBusy)
    ));
    let beat = store
        .heartbeat_job(
            &claimed,
            LeasePolicy::default(),
            Progress::new(1, None).unwrap(),
            None,
            None,
        )
        .await;
    assert!(beat.is_ok(), "{beat:?}");
    assert_eq!(count(&pool, "select count(*) from documents").await, 1);
}
