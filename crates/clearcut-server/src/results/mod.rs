// Written by Kone Tshivhinda & Claude
//! Reading a document back: its status, its events, the narrated report and the downloads.
//!
//! - `GET /v1/documents/{id}`: the document's status, page count and file name.
//! - `GET /v1/documents/{id}/events`: the events as stored (the source of truth).
//! - `GET /v1/documents/{id}/report?mode=summary|timeline|grouped`: the narration, every
//!   sentence with the ids of the events it cites (a number that is not in them cannot be in it).
//! - `GET /v1/documents/{id}/export?format=json|csv|md`: the same, as a file to download.
//!
//! All need `documents:read`; all answer `404` for something that does not exist **or belongs to
//! another tenant**, `400` for an id or a mode that makes no sense, `503` without a store.
//! Everything is made from the stored events, nothing else (ADR-0004).

mod export;

use crate::auth::{ApiProblem, AppState, Authed};
use axum::extract::rejection::PathRejection;
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use clearcut_core::auth::Scope;
use clearcut_core::event::Event;
use clearcut_core::narration::report::{narrate, NarrationMode};
use clearcut_store::{DocId, DocumentInfo, DocumentSummary, Store, TenantId};
use serde::Serialize;
use serde_json::json;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use uuid::Uuid;

pub type ResultsFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, clearcut_store::Error>> + Send + 'a>>;

/// What the HTTP layer needs to read a document back. The real one is the store; tests use a fake.
pub trait Results: Send + Sync + 'static {
    /// The tenant's most recent documents, newest first.
    fn list<'a>(&'a self, tenant: TenantId, limit: u32) -> ResultsFuture<'a, Vec<DocumentSummary>>;

    /// [`clearcut_store::Error::DocumentNotFound`] if the tenant has no such document.
    fn info<'a>(&'a self, tenant: TenantId, document: DocId) -> ResultsFuture<'a, DocumentInfo>;

    /// Every event of the document, oldest first.
    fn events<'a>(&'a self, tenant: TenantId, document: DocId) -> ResultsFuture<'a, Vec<Event>>;
}

impl Results for Store {
    fn list<'a>(&'a self, tenant: TenantId, limit: u32) -> ResultsFuture<'a, Vec<DocumentSummary>> {
        Box::pin(async move { self.list_documents(tenant, limit).await })
    }

    fn info<'a>(&'a self, tenant: TenantId, document: DocId) -> ResultsFuture<'a, DocumentInfo> {
        Box::pin(async move { self.document_info(tenant, document).await })
    }

    fn events<'a>(&'a self, tenant: TenantId, document: DocId) -> ResultsFuture<'a, Vec<Event>> {
        Box::pin(async move { self.load_events(tenant, document).await })
    }
}

// ---------------------------------------------------------------- problems ----

pub(crate) enum ResultsProblem {
    Api(ApiProblem),
    NotConfigured,
    BadId,
    BadChoice(&'static str),
    Store(Box<clearcut_store::Error>),
}

impl From<ApiProblem> for ResultsProblem {
    fn from(problem: ApiProblem) -> Self {
        ResultsProblem::Api(problem)
    }
}

impl From<clearcut_store::Error> for ResultsProblem {
    fn from(error: clearcut_store::Error) -> Self {
        ResultsProblem::Store(Box::new(error))
    }
}

fn message(status: StatusCode, text: &str) -> Response {
    (status, Json(json!({ "error": text }))).into_response()
}

impl IntoResponse for ResultsProblem {
    fn into_response(self) -> Response {
        match self {
            ResultsProblem::Api(problem) => problem.into_response(),
            ResultsProblem::BadId => message(StatusCode::BAD_REQUEST, "invalid id"),
            ResultsProblem::BadChoice(text) => message(StatusCode::BAD_REQUEST, text),
            ResultsProblem::NotConfigured => message(
                StatusCode::SERVICE_UNAVAILABLE,
                "reading documents is not available",
            ),
            ResultsProblem::Store(error) => match *error {
                clearcut_store::Error::DocumentNotFound => {
                    message(StatusCode::NOT_FOUND, "document not found")
                }
                other => {
                    tracing::error!(error = %other, "a document request failed");
                    ApiProblem::Internal.into_response()
                }
            },
        }
    }
}

// ---------------------------------------------------------------- handlers ----

type Id = Result<Path<Uuid>, PathRejection>;

async fn load(
    state: &AppState,
    auth: &Authed,
    id: Id,
) -> Result<(DocumentInfo, Vec<Event>, Uuid), ResultsProblem> {
    let principal = auth.require(Scope::DocumentsRead)?;
    let Path(id) = id.map_err(|_| ResultsProblem::BadId)?;
    let results = state.results().ok_or(ResultsProblem::NotConfigured)?;
    let tenant = TenantId(principal.tenant());
    let info = results.info(tenant, DocId(id)).await?;
    let events = results.events(tenant, DocId(id)).await?;
    Ok((info, events, id))
}

#[derive(Serialize)]
pub(crate) struct DocumentView {
    id: String,
    status: &'static str,
    page_count: Option<i32>,
    original_filename: String,
    events: usize,
}

#[derive(Serialize)]
struct SummaryView {
    id: String,
    status: &'static str,
    page_count: Option<i32>,
    original_filename: String,
    created_at: String,
}

/// `GET /v1/documents`: the most recent documents, newest first.
pub(crate) async fn list_documents(
    State(state): State<AppState>,
    auth: Authed,
) -> Result<Json<serde_json::Value>, ResultsProblem> {
    let principal = auth.require(Scope::DocumentsRead)?;
    let results = state.results().ok_or(ResultsProblem::NotConfigured)?;
    let documents = results.list(TenantId(principal.tenant()), 100).await?;
    let documents: Vec<SummaryView> = documents
        .into_iter()
        .map(|d| SummaryView {
            id: d.id.0.to_string(),
            status: d.status.as_str(),
            page_count: d.page_count,
            original_filename: d.original_filename,
            created_at: d
                .created_at
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_default(),
        })
        .collect();
    Ok(Json(json!({ "documents": documents })))
}

/// `GET /v1/documents/{id}`
pub(crate) async fn get_document(
    State(state): State<AppState>,
    auth: Authed,
    id: Id,
) -> Result<Json<DocumentView>, ResultsProblem> {
    let (info, events, id) = load(&state, &auth, id).await?;
    Ok(Json(DocumentView {
        id: id.to_string(),
        status: info.status.as_str(),
        page_count: info.page_count,
        original_filename: info.original_filename,
        events: events.len(),
    }))
}

/// `GET /v1/documents/{id}/events`
pub(crate) async fn get_events(
    State(state): State<AppState>,
    auth: Authed,
    id: Id,
) -> Result<Json<serde_json::Value>, ResultsProblem> {
    let (_, events, _) = load(&state, &auth, id).await?;
    // `events` is what is stored; `table` is the same events, one line each, with the sentence that
    // tells them (for showing in a table).
    Ok(Json(json!({ "events": events, "table": export::rows(&events) })))
}

fn mode_of(query: &HashMap<String, String>) -> Result<NarrationMode, ResultsProblem> {
    match query.get("mode").map(String::as_str) {
        None | Some("summary") => Ok(NarrationMode::Summary),
        Some("timeline") => Ok(NarrationMode::Timeline),
        Some("grouped") => Ok(NarrationMode::GroupedReport),
        Some(_) => Err(ResultsProblem::BadChoice(
            "mode must be summary, timeline or grouped",
        )),
    }
}

/// `GET /v1/documents/{id}/report`
pub(crate) async fn get_report(
    State(state): State<AppState>,
    auth: Authed,
    id: Id,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, ResultsProblem> {
    let mode = mode_of(&query)?;
    let (_, events, _) = load(&state, &auth, id).await?;
    Ok(Json(export::report_json(&narrate(&events, mode))))
}

/// `GET /v1/documents/{id}/export`
pub(crate) async fn export_document(
    State(state): State<AppState>,
    auth: Authed,
    id: Id,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Response, ResultsProblem> {
    let format = query.get("format").map_or("json", String::as_str);
    if !matches!(format, "json" | "csv" | "md") {
        return Err(ResultsProblem::BadChoice("format must be json, csv or md"));
    }
    let (info, events, id) = load(&state, &auth, id).await?;
    let (content_type, extension, body) = match format {
        "csv" => ("text/csv; charset=utf-8", "csv", export::csv(&events)),
        "md" => (
            "text/markdown; charset=utf-8",
            "md",
            export::markdown(&info, &events),
        ),
        _ => ("application/json", "json", export::json(&info, &events, id)),
    };
    Ok((
        [
            (header::CONTENT_TYPE, content_type.to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"clearcut-{id}.{extension}\""),
            ),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
        body,
    )
        .into_response())
}

#[cfg(test)]
mod tests;
