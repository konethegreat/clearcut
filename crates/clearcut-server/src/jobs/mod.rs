// Written by Kone Tshivhinda & Claude
//! The job endpoints: see what is happening to a document, and stop it.
//!
//! - `GET /v1/documents/{id}/jobs` (needs `documents:read`): every job of the
//!   document, newest first, with state and progress.
//! - `POST /v1/jobs/{id}/cancel` (needs `documents:write`): `200` with the job if
//!   it is now cancelled or had already finished, `202` if it was running and
//!   has only been **asked** to stop (the worker stops at its next heartbeat).
//!
//! Both answer `404` for something that does not exist **or belongs to another
//! tenant**, `401`/`403` as everywhere else (authentication first, before the
//! path is read), `500` with no detail when the database fails, and `503` if the
//! server was started without a queue.
//!
//! A job is shown with its state, progress and timestamps. Its parameters,
//! checkpoint, the name of the worker holding it, its lease and its tenant are
//! internal and never appear.

use crate::auth::{ApiProblem, AppState, Authed};
use axum::extract::rejection::PathRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use clearcut_core::auth::Scope;
use clearcut_core::jobs::JobState;
use clearcut_store::{DocId, Job, Store, TenantId};
use serde::Serialize;
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;
use uuid::Uuid;

pub type QueueFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, clearcut_store::Error>> + Send + 'a>>;

/// What the HTTP layer needs from the job queue. The real one is the store;
/// tests use a fake so the HTTP rules can be checked without a database.
pub trait JobQueue: Send + Sync + 'static {
    /// Every job of a document, newest first. [`clearcut_store::Error::DocumentNotFound`]
    /// if the tenant has no such document.
    fn jobs_for_document<'a>(&'a self, tenant: TenantId, document: DocId) -> QueueFuture<'a, Vec<Job>>;

    /// Cancels a job (see `Store::request_cancel`) and returns it as it is
    /// afterwards. [`clearcut_store::Error::JobNotFound`] if the tenant has no
    /// such job. `actor` is who asked, for the audit log.
    fn cancel_job<'a>(&'a self, tenant: TenantId, job: Uuid, actor: Value) -> QueueFuture<'a, Job>;
}

impl JobQueue for Store {
    fn jobs_for_document<'a>(&'a self, tenant: TenantId, document: DocId) -> QueueFuture<'a, Vec<Job>> {
        Box::pin(async move {
            // An unknown document is a 404, not an empty list.
            self.document_status(tenant, document).await?;
            Store::jobs_for_document(self, tenant, document).await
        })
    }

    fn cancel_job<'a>(&'a self, tenant: TenantId, job: Uuid, actor: Value) -> QueueFuture<'a, Job> {
        Box::pin(async move {
            self.request_cancel(tenant, job, &actor).await?;
            self.get_job(tenant, job).await
        })
    }
}

// ---------------------------------------------------------------- the view ----

#[derive(Serialize)]
struct ProgressView {
    done: u32,
    total: Option<u32>,
    percent: Option<u8>,
}

#[derive(Serialize)]
struct JobView {
    id: String,
    document_id: String,
    kind: &'static str,
    state: &'static str,
    attempts: u32,
    max_attempts: u32,
    progress: ProgressView,
    progress_note: Option<String>,
    last_error: Option<String>,
    cancel_requested: bool,
    created_at: String,
    updated_at: String,
    started_at: Option<String>,
    finished_at: Option<String>,
    run_after: String,
    deadline_at: String,
}

fn stamp(time: OffsetDateTime) -> String {
    time.format(&Rfc3339).unwrap_or_default()
}

impl From<&Job> for JobView {
    fn from(job: &Job) -> Self {
        JobView {
            id: job.id.to_string(),
            document_id: job.document.0.to_string(),
            kind: job.kind.as_str(),
            state: job.state.as_str(),
            attempts: job.attempts,
            max_attempts: job.max_attempts,
            progress: ProgressView {
                done: job.progress.done(),
                total: job.progress.total(),
                percent: job.progress.percent(),
            },
            progress_note: job.progress_note.clone(),
            last_error: job.last_error.clone(),
            cancel_requested: job.cancel_requested_at.is_some(),
            created_at: stamp(job.created_at),
            updated_at: stamp(job.updated_at),
            started_at: job.started_at.map(stamp),
            finished_at: job.finished_at.map(stamp),
            run_after: stamp(job.run_after),
            deadline_at: stamp(job.deadline_at),
        }
    }
}

#[derive(Serialize)]
pub(crate) struct JobList {
    jobs: Vec<JobView>,
}

#[derive(Serialize)]
pub(crate) struct JobAnswer {
    job: JobView,
}

// ---------------------------------------------------------------- problems ----

/// Why a job request did not go ahead. (The store error is boxed to keep this small.)
pub(crate) enum JobProblem {
    Api(ApiProblem),
    NotConfigured,
    /// The id in the address is not a UUID.
    BadId,
    Store(Box<clearcut_store::Error>),
}

impl From<ApiProblem> for JobProblem {
    fn from(problem: ApiProblem) -> Self {
        JobProblem::Api(problem)
    }
}

impl From<clearcut_store::Error> for JobProblem {
    fn from(error: clearcut_store::Error) -> Self {
        JobProblem::Store(Box::new(error))
    }
}

fn message(status: StatusCode, text: &str) -> Response {
    (status, Json(serde_json::json!({ "error": text }))).into_response()
}

impl IntoResponse for JobProblem {
    fn into_response(self) -> Response {
        match self {
            JobProblem::Api(problem) => problem.into_response(),
            JobProblem::BadId => message(StatusCode::BAD_REQUEST, "invalid id"),
            JobProblem::NotConfigured => message(StatusCode::SERVICE_UNAVAILABLE, "jobs are not available"),
            JobProblem::Store(error) => match *error {
                clearcut_store::Error::DocumentNotFound => {
                    message(StatusCode::NOT_FOUND, "document not found")
                }
                clearcut_store::Error::JobNotFound => message(StatusCode::NOT_FOUND, "job not found"),
                other => {
                    // The detail stays in the log; the caller is told nothing about it.
                    tracing::error!(error = %other, "a job request failed");
                    ApiProblem::Internal.into_response()
                }
            },
        }
    }
}

// ---------------------------------------------------------------- handlers ----

/// `GET /v1/documents/{id}/jobs`
pub(crate) async fn list_for_document(
    State(state): State<AppState>,
    auth: Authed,
    document: Result<Path<Uuid>, PathRejection>,
) -> Result<Json<JobList>, JobProblem> {
    let principal = auth.require(Scope::DocumentsRead)?;
    let Path(document) = document.map_err(|_| JobProblem::BadId)?;
    let queue = state.jobs().ok_or(JobProblem::NotConfigured)?;
    let jobs = queue
        .jobs_for_document(TenantId(principal.tenant()), DocId(document))
        .await?;
    Ok(Json(JobList {
        jobs: jobs.iter().map(JobView::from).collect(),
    }))
}

/// `POST /v1/jobs/{id}/cancel`
pub(crate) async fn cancel(
    State(state): State<AppState>,
    auth: Authed,
    job: Result<Path<Uuid>, PathRejection>,
) -> Result<(StatusCode, Json<JobAnswer>), JobProblem> {
    let principal = auth.require(Scope::DocumentsWrite)?;
    let Path(job) = job.map_err(|_| JobProblem::BadId)?;
    let queue = state.jobs().ok_or(JobProblem::NotConfigured)?;
    let actor = serde_json::json!({ "type": "api_key", "id": principal.key_id().to_string() });
    let after = queue.cancel_job(TenantId(principal.tenant()), job, actor).await?;
    // A job still running has only been asked to stop: accepted, not done.
    let status = if after.state == JobState::Running {
        StatusCode::ACCEPTED
    } else {
        StatusCode::OK
    };
    Ok((
        status,
        Json(JobAnswer {
            job: JobView::from(&after),
        }),
    ))
}

#[cfg(test)]
mod tests;
