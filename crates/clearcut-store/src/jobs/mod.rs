// Written by Kone Tshivhinda & Claude
//! The durable job queue (ADR-0008) on Postgres.
//!
//! The rules (which states exist, which moves are legal, how long to wait
//! before a retry, what a finished job does to its document) live in
//! `clearcut_core::jobs`. This module applies them to the database and keeps
//! the guarantees the schema makes:
//!
//! - **The application role can only add and read jobs.** Every later change
//!   goes through a vetted `SECURITY DEFINER` function: [`Store::claim_job`],
//!   [`Store::heartbeat_job`], [`Store::complete_job`], [`Store::fail_job`] and
//!   [`Store::request_cancel`]. Each checks the tenant, and the ones a worker
//!   calls check the **lease token**, so a worker that lost its lease (it was
//!   slow, and the job went to someone else) cannot change anything.
//! - **The document moves with the job in the same transaction**, so a job and
//!   its document never disagree, even if the process dies in between.
//! - **Time is the database's.** Leases, retry times and deadlines are computed
//!   by Postgres, never from a worker's clock.
//! - **Text that goes into a column is trimmed first** (`short_error`,
//!   `short_note`), so a long message cannot make the write that records a
//!   failure fail itself.
//!
//! A job holds parameters and progress, never invoice content.

use crate::{insert_audit, AuditEntry, DocId, Error, Result, Store, TenantId};
use clearcut_core::jobs::{
    short_error, short_note, Decision, FailureKind, JobKind, JobState, LeasePolicy, Outcome, Progress,
    RetryPolicy, DEFAULT_DEADLINE,
};
use serde_json::Value;
use sqlx::Row;
use std::time::Duration;
use time::OffsetDateTime;
use uuid::Uuid;

/// What a caller decides when it queues a job. Everything else (state,
/// attempts, the lease, timestamps) is set by the database.
#[derive(Debug, Clone)]
pub struct NewJob {
    pub document: DocId,
    pub kind: JobKind,
    /// Parameters for the worker: a JSON object of at most 16 KiB, never invoice content.
    pub payload: Value,
    /// Higher runs first.
    pub priority: i16,
    /// Counted failed runs allowed before the job goes to review. Normally the
    /// retry policy's `max_attempts`.
    pub max_attempts: u32,
    /// Wait this long before the job is first due.
    pub delay: Duration,
    /// Hand the job to a person if it has not finished this long from now.
    pub deadline: Duration,
}

impl NewJob {
    /// A job due now with the default priority, the default retry policy's
    /// attempts and the default 48-hour deadline.
    pub fn new(document: DocId, kind: JobKind) -> Self {
        NewJob {
            document,
            kind,
            payload: serde_json::json!({}),
            priority: 0,
            max_attempts: RetryPolicy::default().max_attempts(),
            delay: Duration::ZERO,
            deadline: DEFAULT_DEADLINE,
        }
    }
}

/// The result of queuing a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnqueuedJob {
    pub id: Uuid,
    /// True when the document already had an unfinished job of this kind, so
    /// `id` is that one and nothing new was queued.
    pub already_active: bool,
}

/// A job as the database holds it. Never includes the lease token.
#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub id: Uuid,
    pub tenant: TenantId,
    pub document: DocId,
    pub kind: JobKind,
    pub payload: Value,
    pub state: JobState,
    pub priority: i16,
    pub attempts: u32,
    pub max_attempts: u32,
    pub run_after: OffsetDateTime,
    pub deadline_at: OffsetDateTime,
    pub leased_by: Option<String>,
    pub lease_expires_at: Option<OffsetDateTime>,
    pub progress: Progress,
    pub progress_note: Option<String>,
    pub checkpoint: Option<Value>,
    pub last_error: Option<String>,
    pub cancel_requested_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub started_at: Option<OffsetDateTime>,
    pub finished_at: Option<OffsetDateTime>,
}

/// The fencing token that proves a worker still holds a job. A new one is made
/// at every claim, so a worker that was replaced holds a dead one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeaseToken(pub Uuid);

/// A job a worker has claimed, with the token to present on every later call.
#[derive(Debug, Clone)]
pub struct ClaimedJob {
    pub job: Job,
    pub lease: LeaseToken,
}

/// What a heartbeat tells the worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Heartbeat {
    /// Someone asked for the job to stop; finish with [`Outcome::Cancelled`]
    /// when it is safe to.
    pub cancel_requested: bool,
}

/// Where a failed run left the job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureResult {
    /// Queued again, due after this delay (`counts`: the failed run stays
    /// counted against the attempts).
    Retrying { after: Duration, counts: bool },
    /// Out of options: a person must look. The document is `needs_review`.
    NeedsReview,
    /// Cancellation had been requested, so the job was not retried.
    Cancelled,
}

const JOB_COLUMNS: &str =
    "id, tenant_id, document_id, kind, payload, state, priority, attempts, max_attempts, \
     run_after, deadline_at, leased_by, lease_expires_at, progress_done, progress_total, progress_note, \
     checkpoint, last_error, cancel_requested_at, created_at, updated_at, started_at, finished_at";

impl Store {
    /// Queues a job for a document. At most one unfinished job of a kind exists
    /// per document, so asking again while one is queued or running returns
    /// that job (`already_active`). A document of another tenant is
    /// [`Error::DocumentNotFound`]. `actor` is who asked, for the audit log.
    pub async fn enqueue_job(&self, tenant: TenantId, new: &NewJob, actor: &Value) -> Result<EnqueuedJob> {
        let mut tx = self.begin(tenant).await?;
        // A job can finish between the insert and the lookup; try again if so.
        for _ in 0..3 {
            let inserted: std::result::Result<Option<Uuid>, sqlx::Error> = sqlx::query_scalar(
                "insert into jobs (tenant_id, document_id, kind, payload, priority, max_attempts, run_after, deadline_at) \
                 values ($1, $2, $3, $4, $5, $6, now() + $7::bigint * interval '1 second', now() + $8::bigint * interval '1 second') \
                 on conflict (tenant_id, document_id, kind) where state in ('queued', 'running') do nothing \
                 returning id",
            )
            .bind(tenant.0)
            .bind(new.document.0)
            .bind(new.kind.as_str())
            .bind(&new.payload)
            .bind(new.priority)
            .bind(i32::try_from(new.max_attempts).unwrap_or(i32::MAX))
            .bind(i64::try_from(new.delay.as_secs()).unwrap_or(i64::MAX))
            .bind(i64::try_from(new.deadline.as_secs()).unwrap_or(i64::MAX))
            .fetch_optional(&mut *tx)
            .await;
            let inserted = match inserted {
                Ok(inserted) => inserted,
                // The composite foreign key: no such document for this tenant.
                Err(sqlx::Error::Database(error)) if error.code().as_deref() == Some("23503") => {
                    return Err(Error::DocumentNotFound);
                }
                Err(error) => return Err(error.into()),
            };
            if let Some(id) = inserted {
                insert_audit(
                    &mut tx,
                    tenant,
                    &AuditEntry {
                        actor: actor.clone(),
                        action: "job.enqueued".into(),
                        subject_type: "job".into(),
                        subject_id: id.to_string(),
                        detail: serde_json::json!({
                            "document_id": new.document.0.to_string(),
                            "kind": new.kind.as_str(),
                        }),
                    },
                )
                .await?;
                tx.commit().await?;
                return Ok(EnqueuedJob {
                    id,
                    already_active: false,
                });
            }
            let existing: Option<Uuid> = sqlx::query_scalar(
                "select id from jobs where tenant_id = $1 and document_id = $2 and kind = $3 \
                 and state in ('queued', 'running')",
            )
            .bind(tenant.0)
            .bind(new.document.0)
            .bind(new.kind.as_str())
            .fetch_optional(&mut *tx)
            .await?;
            if let Some(id) = existing {
                tx.commit().await?;
                return Ok(EnqueuedJob {
                    id,
                    already_active: true,
                });
            }
        }
        Err(Error::InvalidStoredJob(
            "could not queue the job: it kept finishing while being queued".into(),
        ))
    }

    /// One job of the tenant, or [`Error::JobNotFound`] (also for another
    /// tenant's job).
    pub async fn get_job(&self, tenant: TenantId, job: Uuid) -> Result<Job> {
        let mut tx = self.begin(tenant).await?;
        let row = sqlx::query(sqlx::AssertSqlSafe(format!(
            "select {JOB_COLUMNS} from jobs where tenant_id = $1 and id = $2"
        )))
        .bind(tenant.0)
        .bind(job)
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        read_job(&row.ok_or(Error::JobNotFound)?)
    }

    /// Every job of a document, newest first.
    pub async fn jobs_for_document(&self, tenant: TenantId, document: DocId) -> Result<Vec<Job>> {
        let mut tx = self.begin(tenant).await?;
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "select {JOB_COLUMNS} from jobs where tenant_id = $1 and document_id = $2 \
             order by created_at desc, id desc"
        )))
        .bind(tenant.0)
        .bind(document.0)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.iter().map(read_job).collect()
    }

    /// Takes the best job that is due, for a runner serving every tenant. The
    /// job is leased to `worker` for `lease`, its `attempts` goes up by one and
    /// its document becomes `processing`. Jobs found past their deadline, out of
    /// attempts or cancelled are finished on the way (their documents move
    /// accordingly) rather than run. `per_tenant_cap`, when given, skips
    /// tenants that already have that many jobs running. `None` when nothing is
    /// due.
    pub async fn claim_job(
        &self,
        worker: &str,
        lease: LeasePolicy,
        kinds: &[JobKind],
        per_tenant_cap: Option<u32>,
    ) -> Result<Option<ClaimedJob>> {
        let kinds: Vec<String> = kinds.iter().map(|kind| kind.as_str().to_string()).collect();
        let row = sqlx::query(sqlx::AssertSqlSafe(format!(
            "select {JOB_COLUMNS}, lease_token from claim_job($1, $2, $3, $4)"
        )))
        .bind(worker)
        .bind(lease_seconds(lease))
        .bind(&kinds)
        .bind(per_tenant_cap.map(|cap| i32::try_from(cap).unwrap_or(i32::MAX)))
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let token: Option<Uuid> = row.try_get("lease_token")?;
        let token =
            token.ok_or_else(|| Error::InvalidStoredJob("a claimed job has no lease token".into()))?;
        Ok(Some(ClaimedJob {
            job: read_job(&row)?,
            lease: LeaseToken(token),
        }))
    }

    /// A sign of life from the worker holding `claimed`: extends the lease,
    /// stores progress, and optionally a note and a checkpoint (`None` keeps
    /// what is stored). Tells the worker whether cancellation was requested.
    /// [`Error::LeaseLost`] if the job has been taken over or is no longer
    /// running; [`Error::JobNotFound`] if it was erased.
    pub async fn heartbeat_job(
        &self,
        claimed: &ClaimedJob,
        lease: LeasePolicy,
        progress: Progress,
        note: Option<&str>,
        checkpoint: Option<&Value>,
    ) -> Result<Heartbeat> {
        let mut tx = self.begin(claimed.job.tenant).await?;
        let result = sqlx::query_scalar::<_, bool>("select heartbeat_job($1, $2, $3, $4, $5, $6, $7, $8)")
            .bind(claimed.job.tenant.0)
            .bind(claimed.job.id)
            .bind(claimed.lease.0)
            .bind(lease_seconds(lease))
            .bind(i32::try_from(progress.done()).unwrap_or(i32::MAX))
            .bind(
                progress
                    .total()
                    .map(|total| i32::try_from(total).unwrap_or(i32::MAX)),
            )
            .bind(note.map(short_note))
            .bind(checkpoint)
            .fetch_one(&mut *tx)
            .await;
        let cancel_requested = result.map_err(job_error)?;
        tx.commit().await?;
        Ok(Heartbeat { cancel_requested })
    }

    /// The worker finishes the job. The document becomes `done`,
    /// `needs_review` or `received` to match. `reason` (trimmed to fit) is kept
    /// for a person to read, for example why a document needs review.
    pub async fn complete_job(
        &self,
        claimed: &ClaimedJob,
        outcome: Outcome,
        reason: Option<&str>,
    ) -> Result<()> {
        let mut tx = self.begin(claimed.job.tenant).await?;
        let result = sqlx::query("select complete_job($1, $2, $3, $4, $5)")
            .bind(claimed.job.tenant.0)
            .bind(claimed.job.id)
            .bind(claimed.lease.0)
            .bind(outcome.as_str())
            .bind(reason.map(short_error))
            .execute(&mut *tx)
            .await;
        result.map_err(job_error)?;
        tx.commit().await?;
        Ok(())
    }

    /// The worker reports a failed run. `policy` decides what happens next
    /// (retry later, or hand the document to a person); `unit_bp` is a random
    /// sample in `0..=10_000` for jitter (see `RetryPolicy`). The database has
    /// the last word: out of attempts, past the deadline, or cancellation
    /// requested all end the job even if the policy said to retry.
    pub async fn fail_job(
        &self,
        claimed: &ClaimedJob,
        kind: FailureKind,
        error: &str,
        policy: &RetryPolicy,
        unit_bp: u32,
    ) -> Result<FailureResult> {
        match policy.decide(kind, claimed.job.attempts, unit_bp) {
            Decision::NeedsReview => {
                self.complete_job(claimed, Outcome::NeedsReview, Some(error))
                    .await?;
                Ok(FailureResult::NeedsReview)
            }
            Decision::Retry { after, counts } => {
                let mut tx = self.begin(claimed.job.tenant).await?;
                let result = sqlx::query_scalar::<_, String>("select fail_job($1, $2, $3, $4, $5, $6)")
                    .bind(claimed.job.tenant.0)
                    .bind(claimed.job.id)
                    .bind(claimed.lease.0)
                    .bind(i32::try_from(after.as_secs()).unwrap_or(i32::MAX))
                    .bind(counts)
                    .bind(short_error(error))
                    .fetch_one(&mut *tx)
                    .await;
                let state = result.map_err(job_error)?;
                tx.commit().await?;
                match JobState::parse(&state)? {
                    JobState::Queued => Ok(FailureResult::Retrying { after, counts }),
                    JobState::Cancelled => Ok(FailureResult::Cancelled),
                    _ => Ok(FailureResult::NeedsReview),
                }
            }
        }
    }

    /// Cancels a job on behalf of `actor` (recorded in the audit log). A queued
    /// job is cancelled at once; a running one is asked to stop and the worker
    /// finds out at its next heartbeat; a finished one is left alone. Returns
    /// the state afterwards.
    pub async fn request_cancel(&self, tenant: TenantId, job: Uuid, actor: &Value) -> Result<JobState> {
        let mut tx = self.begin(tenant).await?;
        let result = sqlx::query_scalar::<_, String>("select cancel_job($1, $2, $3)")
            .bind(tenant.0)
            .bind(job)
            .bind(actor)
            .fetch_one(&mut *tx)
            .await;
        let state = result.map_err(job_error)?;
        tx.commit().await?;
        Ok(JobState::parse(&state)?)
    }
}

fn lease_seconds(lease: LeasePolicy) -> i32 {
    i32::try_from(lease.duration().as_secs()).unwrap_or(i32::MAX)
}

/// Turns the queue functions' error codes into typed errors.
fn job_error(error: sqlx::Error) -> Error {
    if let sqlx::Error::Database(database) = &error {
        match database.code().as_deref() {
            Some("CL001") => return Error::LeaseLost,
            Some("CL002") => return Error::JobNotFound,
            _ => {}
        }
    }
    error.into()
}

fn read_job(row: &sqlx::postgres::PgRow) -> Result<Job> {
    let count = |column: &str| -> Result<u32> {
        let value: i32 = row.try_get(column)?;
        u32::try_from(value).map_err(|_| Error::InvalidStoredJob(format!("{column} is negative")))
    };
    let total: Option<i32> = row.try_get("progress_total")?;
    let total = total
        .map(|total| {
            u32::try_from(total).map_err(|_| Error::InvalidStoredJob("progress_total is negative".into()))
        })
        .transpose()?;
    let kind: String = row.try_get("kind")?;
    let state: String = row.try_get("state")?;
    Ok(Job {
        id: row.try_get("id")?,
        tenant: TenantId(row.try_get("tenant_id")?),
        document: DocId(row.try_get("document_id")?),
        kind: JobKind::parse(&kind)?,
        payload: row.try_get("payload")?,
        state: JobState::parse(&state)?,
        priority: row.try_get("priority")?,
        attempts: count("attempts")?,
        max_attempts: count("max_attempts")?,
        run_after: row.try_get("run_after")?,
        deadline_at: row.try_get("deadline_at")?,
        leased_by: row.try_get("leased_by")?,
        lease_expires_at: row.try_get("lease_expires_at")?,
        progress: Progress::new(count("progress_done")?, total)?,
        progress_note: row.try_get("progress_note")?,
        checkpoint: row.try_get("checkpoint")?,
        last_error: row.try_get("last_error")?,
        cancel_requested_at: row.try_get("cancel_requested_at")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
        started_at: row.try_get("started_at")?,
        finished_at: row.try_get("finished_at")?,
    })
}

#[cfg(test)]
mod tests;
