// Written by Kone Tshivhinda & Claude
//! `POST /v1/documents`: upload one invoice.
//!
//! The request is the file itself, not a form: the body is the raw bytes, the
//! `Content-Type` header says what kind of file it is, and an optional
//! `filename` query parameter carries its name. Sending the bytes raw means the
//! server can stream them to storage as they arrive instead of holding the
//! whole file in memory. (A file name in a URL can end up in a proxy's log; this
//! server does not log URLs, and the name is only a label.)
//!
//! This is only the HTTP layer. Every rule about *accepting* a file lives in
//! `clearcut-intake`; this module checks the caller's scope, translates the
//! request into an [`Upload`], and translates the outcome into a status code:
//!
//! | outcome | answer |
//! |---|---|
//! | stored | `201 Created`, or `200 OK` if this tenant had uploaded the same bytes before |
//! | no valid key | `401` (identical for every failure, see [`crate::auth`]) |
//! | key lacks `documents:write` | `403`, naming the scope |
//! | not a PDF, PNG, JPEG, TIFF or XML | `415`, listing what is accepted |
//! | over the size limit | `413`, saying the limit |
//! | empty, or the sender went away | `400` |
//! | our storage or database failed | `500`, with no detail (it goes to the log) |
//! | no intake configured | `503` |

use crate::auth::{ApiProblem, AppState, Authed};
use axum::body::Body;
use axum::extract::rejection::PathRejection;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use clearcut_core::auth::{Principal, Scope};
use clearcut_intake::{Intake, IntakeError, MediaType, Upload, Uploaded};
use clearcut_objects::ObjectStore;
use clearcut_store::DocId;
use futures_util::TryStreamExt;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::io;
use std::pin::Pin;
use tokio::io::AsyncRead;
use tokio_util::io::StreamReader;
use uuid::Uuid;

/// The request body as the intake reads it: a stream of bytes.
pub type UploadBody = Pin<Box<dyn AsyncRead + Send>>;

pub type IngestFuture<'a> = Pin<Box<dyn Future<Output = Result<Uploaded, IntakeError>> + Send + 'a>>;
pub type EraseFuture<'a> = Pin<Box<dyn Future<Output = Result<(), IntakeError>> + Send + 'a>>;

/// Something that can accept an upload. The real one is
/// [`clearcut_intake::Intake`]; tests use a fake so the HTTP rules can be
/// checked without a database. (The future is boxed so the trait can be used
/// as `dyn`, the same way as [`crate::Authenticator`].)
pub trait Ingest: Send + Sync + 'static {
    fn ingest<'a>(&'a self, by: &'a Principal, upload: Upload<UploadBody>) -> IngestFuture<'a>;

    /// Erases a document and its file for good (see [`Intake::erase`]).
    fn erase<'a>(&'a self, by: &'a Principal, document: DocId) -> EraseFuture<'a>;
}

impl<O: ObjectStore + 'static> Ingest for Intake<O> {
    fn ingest<'a>(&'a self, by: &'a Principal, upload: Upload<UploadBody>) -> IngestFuture<'a> {
        Box::pin(self.upload(by, upload))
    }

    fn erase<'a>(&'a self, by: &'a Principal, document: DocId) -> EraseFuture<'a> {
        Box::pin(Intake::erase(self, by, document))
    }
}

#[derive(Deserialize)]
pub(crate) struct UploadParams {
    /// What the sender calls the file. Cleaned by the intake before it is kept.
    filename: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct UploadedBody {
    document_id: String,
    /// The job that reads the document (absent for a repeat upload of one already worked on).
    job_id: Option<String>,
    duplicate: bool,
    sha256: String,
    byte_size: u64,
    filename: String,
    media_type: &'static str,
}

/// The extractors run in this order on purpose: the caller is authenticated
/// first, then the query is read, and the body (last, as axum requires) is not
/// touched until the scope has been checked.
pub(crate) async fn upload_document(
    State(state): State<AppState>,
    auth: Authed,
    Query(params): Query<UploadParams>,
    headers: HeaderMap,
    body: Body,
) -> Result<(StatusCode, Json<UploadedBody>), UploadFailure> {
    let principal = auth.require(Scope::DocumentsWrite)?;
    let Some(ingest) = state.ingest() else {
        return Err(UploadFailure::NotConfigured);
    };

    let text_of = |name| headers.get(name).and_then(|value| value.to_str().ok());
    let upload = Upload {
        filename: params.filename,
        content_type: text_of(header::CONTENT_TYPE).unwrap_or("").to_string(),
        declared_length: text_of(header::CONTENT_LENGTH).and_then(|value| value.parse().ok()),
        body: reader(body),
    };

    match ingest.ingest(principal, upload).await {
        Ok(done) => Ok((
            if done.duplicate {
                StatusCode::OK
            } else {
                StatusCode::CREATED
            },
            Json(UploadedBody {
                document_id: done.document.0.to_string(),
                job_id: done.job.map(|job| job.to_string()),
                duplicate: done.duplicate,
                sha256: done.sha256_hex,
                byte_size: done.byte_size,
                filename: done.filename,
                media_type: done.media_type.as_str(),
            }),
        )),
        Err(error) => Err(UploadFailure::Refused(Box::new(error))),
    }
}

/// `DELETE /v1/documents/{id}`: erase a document and its file for good (POPIA). Needs
/// `documents:write`. `204` when it is gone, `404` for one that does not exist or belongs to
/// another tenant, `409` while a job is queued or running on it.
pub(crate) async fn erase_document(
    State(state): State<AppState>,
    auth: Authed,
    id: Result<Path<Uuid>, PathRejection>,
) -> Result<StatusCode, UploadFailure> {
    let principal = auth.require(Scope::DocumentsWrite)?;
    let Path(id) = id.map_err(|_| UploadFailure::Refused(Box::new(IntakeError::Empty)))?;
    let Some(ingest) = state.ingest() else {
        return Err(UploadFailure::NotConfigured);
    };
    match ingest.erase(principal, DocId(id)).await {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(IntakeError::Database(clearcut_store::Error::DocumentNotFound)) => Err(UploadFailure::NotFound),
        Err(error) => Err(UploadFailure::Refused(Box::new(error))),
    }
}

/// Why an upload did not go ahead. (Boxed where large, to keep the error small.)
pub(crate) enum UploadFailure {
    /// No scope: the usual 403.
    Api(ApiProblem),
    /// The server was started without an intake.
    NotConfigured,
    /// The document does not exist, or belongs to another tenant.
    NotFound,
    /// The intake said no, or failed.
    Refused(Box<IntakeError>),
}

impl From<ApiProblem> for UploadFailure {
    fn from(problem: ApiProblem) -> Self {
        UploadFailure::Api(problem)
    }
}

impl IntoResponse for UploadFailure {
    fn into_response(self) -> Response {
        match self {
            UploadFailure::Api(problem) => problem.into_response(),
            UploadFailure::NotConfigured => {
                problem(StatusCode::SERVICE_UNAVAILABLE, "uploads are not available")
            }
            UploadFailure::NotFound => problem(StatusCode::NOT_FOUND, "document not found"),
            UploadFailure::Refused(error) => explain(*error),
        }
    }
}

/// Turns the request body into something the intake can read. A body that
/// breaks off (the client disconnected) is reported as an aborted connection,
/// which the intake recognises as "interrupted" rather than as our failure.
fn reader(body: Body) -> UploadBody {
    let stream = body
        .into_data_stream()
        .map_err(|error| io::Error::new(io::ErrorKind::ConnectionAborted, error));
    Box::pin(StreamReader::new(stream))
}

fn problem(status: StatusCode, message: &str) -> Response {
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}

fn explain(error: IntakeError) -> Response {
    match error {
        IntakeError::Forbidden(scope) => ApiProblem::Forbidden(scope).into_response(),
        IntakeError::UnsupportedMediaType => (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Json(serde_json::json!({
                "error": "unsupported media type",
                "accepted": [
                    MediaType::Pdf.as_str(),
                    MediaType::Png.as_str(),
                    MediaType::Jpeg.as_str(),
                    MediaType::Tiff.as_str(),
                    MediaType::Xml.as_str(),
                ],
            })),
        )
            .into_response(),
        IntakeError::TooLarge { max_bytes } => (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({ "error": "file too large", "max_bytes": max_bytes })),
        )
            .into_response(),
        IntakeError::Empty => problem(StatusCode::BAD_REQUEST, "the file is empty"),
        IntakeError::Interrupted => problem(StatusCode::BAD_REQUEST, "the upload was interrupted"),
        IntakeError::Busy => problem(
            StatusCode::CONFLICT,
            "the document is being worked on; try again when it has finished",
        ),
        IntakeError::Storage(_) | IntakeError::Database(_) => {
            // The detail stays in the log; the caller is told nothing about it.
            tracing::error!(%error, "an upload failed");
            ApiProblem::Internal.into_response()
        }
    }
}

#[cfg(test)]
mod tests;
