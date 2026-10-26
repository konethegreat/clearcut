// Written by Kone Tshivhinda & Claude
//! The tenant's own Anthropic key (ADR-0019). All three need `keys:admin`; **none is ever
//! answered with the key**.
//!
//! - `PUT /v1/anthropic-key` with `{ "key": "sk-ant-..." }`: refuses an admin key or a login token
//!   (`422`), asks Anthropic (free, no tokens) and refuses a key it does not accept (`422`) or says
//!   it could not check just now (`502`, nothing stored); otherwise stores it sealed and answers
//!   `{ last4, models, checked_at }`. `503` when the server has no master key.
//! - `GET /v1/anthropic-key`: `{ configured, last4, set_at, checked_at, check_ok, source, mode,
//!   tenant_keys_enabled }`, where `source` says whose key the next job would use (`tenant`,
//!   `server` or `none`).
//! - `DELETE /v1/anthropic-key`: `{ removed }`.
//!
//! The body of the `PUT` is never logged (this server logs no bodies), a body that cannot be read
//! is answered with a fixed sentence that repeats none of it, and the key appears in no response.

use crate::auth::{ApiProblem, AppState, Authed};
use axum::extract::rejection::JsonRejection;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use clearcut_core::auth::{Principal, Scope};
use clearcut_keys::{KeySet, KeyView, KeysError, TenantKeys};
use clearcut_store::TenantId;
use serde_json::{json, Value};
use std::future::Future;
use std::pin::Pin;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

pub type KeysFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, KeysError>> + Send + 'a>>;

/// What the HTTP layer needs. The real one is [`TenantKeys`]; tests use a fake.
pub trait KeyAdmin: Send + Sync + 'static {
    fn status<'a>(&'a self, tenant: TenantId) -> KeysFuture<'a, KeyView>;
    fn set<'a>(&'a self, by: &'a Principal, text: &'a str) -> KeysFuture<'a, KeySet>;
    fn remove<'a>(&'a self, by: &'a Principal) -> KeysFuture<'a, bool>;
}

impl KeyAdmin for TenantKeys {
    fn status<'a>(&'a self, tenant: TenantId) -> KeysFuture<'a, KeyView> {
        Box::pin(TenantKeys::status(self, tenant))
    }

    fn set<'a>(&'a self, by: &'a Principal, text: &'a str) -> KeysFuture<'a, KeySet> {
        Box::pin(TenantKeys::set(self, by, text))
    }

    fn remove<'a>(&'a self, by: &'a Principal) -> KeysFuture<'a, bool> {
        Box::pin(TenantKeys::remove(self, by))
    }
}

pub(crate) enum KeyProblem {
    Api(ApiProblem),
    NotConfigured,
    BadBody,
    Keys(KeysError),
}

impl From<ApiProblem> for KeyProblem {
    fn from(problem: ApiProblem) -> Self {
        KeyProblem::Api(problem)
    }
}

impl From<KeysError> for KeyProblem {
    fn from(error: KeysError) -> Self {
        KeyProblem::Keys(error)
    }
}

fn message(status: StatusCode, text: &str) -> Response {
    (status, Json(json!({ "error": text }))).into_response()
}

impl IntoResponse for KeyProblem {
    fn into_response(self) -> Response {
        match self {
            KeyProblem::Api(problem) => problem.into_response(),
            KeyProblem::NotConfigured => message(StatusCode::SERVICE_UNAVAILABLE, "keys are not available"),
            KeyProblem::BadBody => message(
                StatusCode::BAD_REQUEST,
                "send a JSON object with one text field called key",
            ),
            KeyProblem::Keys(error) => match error {
                KeysError::Forbidden => ApiProblem::Forbidden(Scope::KeysAdmin).into_response(),
                KeysError::NotConfigured => message(StatusCode::SERVICE_UNAVAILABLE, &error.to_string()),
                KeysError::BadKey(_) | KeysError::Refused(_) => {
                    message(StatusCode::UNPROCESSABLE_ENTITY, &error.to_string())
                }
                KeysError::CheckUnavailable(_) => message(StatusCode::BAD_GATEWAY, &error.to_string()),
                KeysError::Store(_) | KeysError::Vault(_) => {
                    tracing::error!(%error, "a tenant key request failed");
                    ApiProblem::Internal.into_response()
                }
            },
        }
    }
}

fn stamp(time: OffsetDateTime) -> String {
    time.format(&Rfc3339).unwrap_or_default()
}

/// `GET /v1/anthropic-key`
pub(crate) async fn status(State(state): State<AppState>, auth: Authed) -> Result<Json<Value>, KeyProblem> {
    let principal = auth.require(Scope::KeysAdmin)?;
    let keys = state.keys().ok_or(KeyProblem::NotConfigured)?;
    let view = keys.status(TenantId(principal.tenant())).await?;
    Ok(Json(json!({
        "configured": view.configured,
        "last4": view.last4,
        "set_at": view.set_at.map(stamp),
        "checked_at": view.checked_at.map(stamp),
        "check_ok": view.check_ok,
        "source": view.source.as_str(),
        "mode": view.mode.as_str(),
        "tenant_keys_enabled": view.tenant_keys_enabled,
    })))
}

/// `PUT /v1/anthropic-key`
pub(crate) async fn set(
    State(state): State<AppState>,
    auth: Authed,
    body: Result<Json<Value>, JsonRejection>,
) -> Result<Json<Value>, KeyProblem> {
    let principal = auth.require(Scope::KeysAdmin)?;
    let keys = state.keys().ok_or(KeyProblem::NotConfigured)?;
    // A body that does not parse is answered with a fixed sentence: the rejection's own text could
    // quote part of what was sent.
    let Json(body) = body.map_err(|_| KeyProblem::BadBody)?;
    let text = body
        .as_object()
        .filter(|object| object.len() == 1)
        .and_then(|object| object.get("key"))
        .and_then(Value::as_str)
        .ok_or(KeyProblem::BadBody)?;
    let set = keys.set(principal, text).await?;
    Ok(Json(json!({
        "last4": set.last4,
        "models": set.models,
        "checked_at": stamp(set.checked_at),
    })))
}

/// `DELETE /v1/anthropic-key`
pub(crate) async fn remove(State(state): State<AppState>, auth: Authed) -> Result<Json<Value>, KeyProblem> {
    let principal = auth.require(Scope::KeysAdmin)?;
    let keys = state.keys().ok_or(KeyProblem::NotConfigured)?;
    let removed = keys.remove(principal).await?;
    Ok(Json(json!({ "removed": removed })))
}

#[cfg(test)]
mod tests;
