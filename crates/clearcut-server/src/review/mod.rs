// Written by Kone Tshivhinda & Claude
//! Review: a person looks at the pages the reader could not settle, corrects them and confirms.
//!
//! - `GET /v1/documents/{id}/review` (needs `documents:read`): the pages waiting for a person,
//!   each with the fixed reason it stopped, what could not be read and where, and the model's best
//!   draft (or `null` when there is none). The draft is text from the document: show it as text.
//! - `POST /v1/documents/{id}/pages/{page}/review` (needs `documents:write`): the person's reading
//!   of the page, in the same shape a model returns. It goes through the same conversion and the
//!   same checks as the model's ([`clearcut_core::review`]). If anything is still wrong the answer
//!   is `422` with the fixed names and places that say what. If it passes, the page is stored as
//!   **confirmed by a person**, the action is audited, and the document is queued to be finished
//!   (put together, checked, turned into events) with no model call. `409` if the page is not
//!   waiting for review (or the document is being worked on right now).
//!
//! Both answer `404` for a document that does not exist **or belongs to another tenant**.

use crate::auth::{ApiProblem, AppState, Authed};
use axum::extract::rejection::{JsonRejection, PathRejection};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use clearcut_core::auth::Scope;
use clearcut_core::escalation::{ConfidenceBasis, ReviewReason};
use clearcut_core::jobs::JobKind;
use clearcut_core::reading::PageReading;
use clearcut_core::review::{check_confirmed, Refusal};
use clearcut_store::page_readings::{KeptReading, PageState};
use clearcut_store::{AuditEntry, DocId, NewJob, Store, TenantId};
use serde::Serialize;
use serde_json::{json, Value};
use std::future::Future;
use std::num::NonZeroU32;
use std::pin::Pin;
use uuid::Uuid;

pub type ReviewFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, ReviewError>> + Send + 'a>>;

/// A page waiting for a person.
#[derive(Debug, Clone, Serialize)]
pub struct WaitingPage {
    pub page: u32,
    /// The fixed name of why it stopped (`tax_not_shown`, `top_rung_failed`, ...).
    pub reason: &'static str,
    /// What could not be read exactly, and where (fixed names and places).
    pub problems: Value,
    /// The checks that failed (these carry amounts).
    pub findings: Value,
    /// The model's best draft, in the shape the person sends back; `null` if there is none.
    pub reading: Option<Value>,
}

/// Who confirms: the key they used. Never a name or an address.
#[derive(Debug, Clone)]
pub struct Reviewer {
    pub label: String,
    pub key_id: Uuid,
}

#[derive(Debug)]
pub enum ReviewError {
    Store(Box<clearcut_store::Error>),
    /// The page is not waiting for review.
    NotWaiting,
    /// A job is queued or running for the document.
    Busy,
    /// The reading is not good enough yet.
    Refused(Box<Refusal>),
}

impl From<clearcut_store::Error> for ReviewError {
    fn from(error: clearcut_store::Error) -> Self {
        ReviewError::Store(Box::new(error))
    }
}

/// What the HTTP layer needs for review. The real one is the store; tests use a fake.
pub trait Reviews: Send + Sync + 'static {
    /// The pages of the document that wait for a person, in page order.
    fn waiting<'a>(&'a self, tenant: TenantId, document: DocId) -> ReviewFuture<'a, Vec<WaitingPage>>;

    /// Checks and stores the person's reading of `page`, and queues the document to be finished.
    /// Returns the queued job.
    fn confirm<'a>(
        &'a self,
        tenant: TenantId,
        document: DocId,
        page: u32,
        reading: PageReading,
        by: Reviewer,
    ) -> ReviewFuture<'a, Uuid>;
}

impl Reviews for Store {
    fn waiting<'a>(&'a self, tenant: TenantId, document: DocId) -> ReviewFuture<'a, Vec<WaitingPage>> {
        Box::pin(async move {
            let pages = self.load_page_readings(tenant, document).await?;
            Ok(pages
                .into_iter()
                .filter_map(|stored| {
                    let PageState::NeedsReview { reason, draft } = stored.state else {
                        return None;
                    };
                    Some(WaitingPage {
                        page: stored.page,
                        reason: reason.as_str(),
                        problems: stored.details.problems,
                        findings: stored.details.findings,
                        reading: draft.map(|kept| kept.reading),
                    })
                })
                .collect())
        })
    }

    fn confirm<'a>(
        &'a self,
        tenant: TenantId,
        document: DocId,
        page: u32,
        reading: PageReading,
        by: Reviewer,
    ) -> ReviewFuture<'a, Uuid> {
        Box::pin(async move {
            let stored = self.load_page_readings(tenant, document).await?;
            let reason: ReviewReason = match stored.iter().find(|p| p.page == page).map(|p| &p.state) {
                Some(PageState::NeedsReview { reason, .. }) => *reason,
                _ => return Err(ReviewError::NotWaiting),
            };
            // A job that is running would read the page as it was: wait for it to end.
            let jobs = self.jobs_for_document(tenant, document).await?;
            if jobs.iter().any(|job| !job.state.is_terminal()) {
                return Err(ReviewError::Busy);
            }
            let pages = self.load_page_reads(tenant, document).await?.len();
            let number = NonZeroU32::new(page).ok_or(ReviewError::NotWaiting)?;
            check_confirmed(&reading, document.as_core_id(), number, &by.label, pages == 1)
                .map_err(|refusal| ReviewError::Refused(Box::new(refusal)))?;

            let kept = KeptReading {
                rung: 0,
                attempt: 1,
                confidence: ConfidenceBasis::PersonConfirmed,
                // Who confirmed it, where a model's name would be (the pipeline reads it back as that).
                model: by.label.clone(),
                reading: serde_json::to_value(&reading).map_err(|_| ReviewError::NotWaiting)?,
            };
            self.confirm_page_reading(tenant, document, page, &kept, &json!([]))
                .await
                .map_err(|error| match error {
                    clearcut_store::Error::PageNotAwaitingReview => ReviewError::NotWaiting,
                    other => other.into(),
                })?;
            let actor = json!({ "type": "api_key", "id": by.key_id.to_string() });
            self.record_audit(
                tenant,
                &AuditEntry {
                    actor: actor.clone(),
                    action: "page.reviewed".to_string(),
                    subject_type: "document".to_string(),
                    subject_id: document.0.to_string(),
                    detail: json!({ "page": page, "was": reason.as_str() }),
                },
            )
            .await?;
            let queued = self
                .enqueue_job(tenant, &NewJob::new(document, JobKind::ProcessDocument), &actor)
                .await?;
            Ok(queued.id)
        })
    }
}

// ---------------------------------------------------------------- problems ----

pub(crate) enum ReviewProblem {
    Api(ApiProblem),
    NotConfigured,
    BadId,
    BadForm,
    Review(ReviewError),
}

impl From<ApiProblem> for ReviewProblem {
    fn from(problem: ApiProblem) -> Self {
        ReviewProblem::Api(problem)
    }
}

impl From<ReviewError> for ReviewProblem {
    fn from(error: ReviewError) -> Self {
        ReviewProblem::Review(error)
    }
}

fn message(status: StatusCode, text: &str) -> Response {
    (status, Json(json!({ "error": text }))).into_response()
}

impl IntoResponse for ReviewProblem {
    fn into_response(self) -> Response {
        match self {
            ReviewProblem::Api(problem) => problem.into_response(),
            ReviewProblem::BadId => message(StatusCode::BAD_REQUEST, "invalid id"),
            ReviewProblem::BadForm => message(
                StatusCode::BAD_REQUEST,
                "that is not a complete reading of a page",
            ),
            ReviewProblem::NotConfigured => {
                message(StatusCode::SERVICE_UNAVAILABLE, "review is not available")
            }
            ReviewProblem::Review(ReviewError::NotWaiting) => {
                message(StatusCode::CONFLICT, "that page is not waiting for review")
            }
            ReviewProblem::Review(ReviewError::Busy) => message(
                StatusCode::CONFLICT,
                "the document is being worked on; try again when it has finished",
            ),
            ReviewProblem::Review(ReviewError::Refused(refusal)) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "error": "the page cannot be confirmed yet", "refusal": *refusal })),
            )
                .into_response(),
            ReviewProblem::Review(ReviewError::Store(error)) => match *error {
                clearcut_store::Error::DocumentNotFound => {
                    message(StatusCode::NOT_FOUND, "document not found")
                }
                other => {
                    tracing::error!(error = %other, "a review request failed");
                    ApiProblem::Internal.into_response()
                }
            },
        }
    }
}

// ---------------------------------------------------------------- handlers ----

/// `GET /v1/documents/{id}/review`
pub(crate) async fn waiting(
    State(state): State<AppState>,
    auth: Authed,
    id: Result<Path<Uuid>, PathRejection>,
) -> Result<Json<Value>, ReviewProblem> {
    let principal = auth.require(Scope::DocumentsRead)?;
    let Path(id) = id.map_err(|_| ReviewProblem::BadId)?;
    let reviews = state.reviews().ok_or(ReviewProblem::NotConfigured)?;
    let pages = reviews.waiting(TenantId(principal.tenant()), DocId(id)).await?;
    Ok(Json(json!({ "pages": pages })))
}

/// `POST /v1/documents/{id}/pages/{page}/review`
pub(crate) async fn confirm(
    State(state): State<AppState>,
    auth: Authed,
    path: Result<Path<(Uuid, u32)>, PathRejection>,
    body: Result<Json<Value>, JsonRejection>,
) -> Result<Json<Value>, ReviewProblem> {
    let principal = auth.require(Scope::DocumentsWrite)?;
    let Path((id, page)) = path.map_err(|_| ReviewProblem::BadId)?;
    let reviews = state.reviews().ok_or(ReviewProblem::NotConfigured)?;
    let Json(body) = body.map_err(|_| ReviewProblem::BadForm)?;
    let reading: PageReading = serde_json::from_value(body).map_err(|_| ReviewProblem::BadForm)?;
    let by = Reviewer {
        label: format!("api_key:{}", principal.key_id()),
        key_id: principal.key_id(),
    };
    let job = reviews
        .confirm(TenantId(principal.tenant()), DocId(id), page, reading, by)
        .await?;
    Ok(Json(json!({ "job_id": job.to_string() })))
}

#[cfg(test)]
mod tests;
