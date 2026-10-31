// Written by Kone Tshivhinda & Claude
//! `POST /v1/register`: one period's documents, read and checked, in a file a bookkeeper or a
//! finance team can use: Excel, a zipped CSV bundle, JSON, PDF, HTML, Markdown or plain text.
//!
//! The register is built from the stored events and page states each time it is asked for
//! (`clearcut_core::register::build`); nothing is stored and no background job runs. Needs
//! `documents:read`. A period (`from` and `to`, both included) or a list of `document_ids` says
//! which documents; the list wins when both are given. Over 500 documents or 50 000 line items is
//! a `400` asking the caller to split the selection. A named document that is not this account's
//! is a `404`. The file says at its foot that its figures must be checked against the original.

use crate::auth::{ApiProblem, AppState, Authed};
use crate::results::ResultsProblem;
use axum::extract::rejection::JsonRejection;
use axum::extract::State;
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::Json;
use clearcut_core::auth::Scope;
use clearcut_core::register::{build, parse_date, RegisterError, Selection, MAX_DOCUMENTS};
use clearcut_export::{render_register, RegisterFormat};
use clearcut_store::TenantId;
use serde::Deserialize;

/// The body of `POST /v1/register`. A field is optional on its own; `select` says which
/// combinations make sense.
#[derive(Debug, Deserialize)]
pub(crate) struct RegisterRequest {
    from: Option<String>,
    to: Option<String>,
    document_ids: Option<Vec<String>>,
    format: Option<String>,
    only_ready: Option<bool>,
}

/// Which documents the request names, checked. The list wins over the period; the period's two
/// ends go together; an end before the start is refused. Repeats in the list are counted once.
fn select(body: &RegisterRequest) -> Result<Selection, ResultsProblem> {
    if let Some(ids) = &body.document_ids {
        let mut seen = std::collections::HashSet::new();
        let unique: Vec<String> = ids
            .iter()
            .filter(|id| seen.insert(id.as_str()))
            .cloned()
            .collect();
        if unique.is_empty() {
            return Err(ResultsProblem::BadChoice(
                "document_ids must name at least one document",
            ));
        }
        if unique.len() > MAX_DOCUMENTS {
            return Err(ResultsProblem::BadChoice(
                "document_ids may name at most 500 documents",
            ));
        }
        return Ok(Selection::Documents { ids: unique });
    }
    let from = match body.from.as_deref() {
        None => None,
        Some(text) => Some(parse_date(text).ok_or(ResultsProblem::BadChoice(
            "from must be a date written as YYYY-MM-DD",
        ))?),
    };
    let to = match body.to.as_deref() {
        None => None,
        Some(text) => Some(parse_date(text).ok_or(ResultsProblem::BadChoice(
            "to must be a date written as YYYY-MM-DD",
        ))?),
    };
    if from.is_some() != to.is_some() {
        return Err(ResultsProblem::BadChoice("give from and to together, or neither"));
    }
    let (Some(from), Some(to)) = (from, to) else {
        return Err(ResultsProblem::BadChoice(
            "name a period with from and to, or up to 500 documents",
        ));
    };
    if to < from {
        return Err(ResultsProblem::BadChoice("to is before from"));
    }
    Ok(Selection::Period { from, to })
}

/// The download's file name: the period it covers, or the selection, and the file's own
/// extension (the CSV bundle is a `.zip`).
fn filename(selection: &Selection, extension: &str) -> String {
    match selection {
        Selection::Period { from, to } => format!("clearcut-register-{from}-to-{to}.{extension}"),
        Selection::Documents { .. } => format!("clearcut-register-selection.{extension}"),
    }
}

/// Why a register could not be built, as a sentence for the caller. All three ask for a smaller
/// selection; none of them is the caller's fault in the sense of a bad format.
fn why(error: RegisterError) -> ResultsProblem {
    match error {
        RegisterError::TooManyDocuments => ResultsProblem::BadChoice(
            "the selection names more than 500 documents; split the period or the list",
        ),
        RegisterError::TooManyLines => {
            ResultsProblem::BadChoice("the selection holds more than 50 000 line items; split the period")
        }
        RegisterError::Overflow => ResultsProblem::BadChoice(
            "the amounts in this selection are too large to add up; split the period",
        ),
    }
}

/// `POST /v1/register`
pub(crate) async fn make_register(
    State(state): State<AppState>,
    auth: Authed,
    body: Result<Json<RegisterRequest>, JsonRejection>,
) -> Result<Response, ResultsProblem> {
    // The scope is checked before the body's rejection is looked at, so a key without
    // `documents:read` learns nothing about how the body is parsed.
    let principal = auth.require(Scope::DocumentsRead)?;
    let Json(body) = body.map_err(|_| ResultsProblem::BadChoice("the body must be a JSON object"))?;
    let selection = select(&body)?;
    let format = RegisterFormat::parse(body.format.as_deref().unwrap_or("xlsx")).ok_or(
        ResultsProblem::BadChoice("format must be json, html, md, txt, xlsx, csv or pdf"),
    )?;
    let results = state.results().ok_or(ResultsProblem::NotConfigured)?;
    let tenant = TenantId(principal.tenant());
    let only_ready = body.only_ready.unwrap_or(false);
    let input = results.register(tenant, &selection, only_ready).await?;
    if let Selection::Documents { ids } = &selection {
        let returned: std::collections::HashSet<&str> =
            input.facts.iter().map(|fact| fact.id.as_str()).collect();
        if input.facts.len() != ids.len() || ids.iter().any(|id| !returned.contains(id.as_str())) {
            return Err(ResultsProblem::Store(Box::new(
                clearcut_store::Error::DocumentNotFound,
            )));
        }
    }
    let register = build(input).map_err(why)?;
    tracing::info!(
        tenant = %tenant.0,
        format = format.name(),
        documents = register.rows.len(),
        lines = register.lines.len(),
        only_ready,
        "a register was made"
    );
    let file = render_register(format, &register).map_err(|error| {
        tracing::error!(%error, "a register could not be made");
        ResultsProblem::Api(ApiProblem::Internal)
    })?;
    Ok((
        [
            (header::CONTENT_TYPE, file.content_type.to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!(
                    "attachment; filename=\"{}\"",
                    filename(&selection, file.extension)
                ),
            ),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
        file.bytes,
    )
        .into_response())
}

#[cfg(test)]
mod tests;
