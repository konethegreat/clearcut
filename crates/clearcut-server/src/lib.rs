// Written by Kone Tshivhinda & Claude
//! ClearCut HTTP API. Thin transport over `clearcut-core`.
//! The MCP host (Phase 1) will expose the same operations as MCP tools.
//!
//! Every route except `/health` requires an API key (ADR-0007); see [`auth`].

mod auth;

pub use auth::{ApiProblem, AppState, AuthFailure, Authed, Authenticator};

use axum::{
    http::{header, StatusCode},
    routing::{get, post},
    Json, Router,
};
use clearcut_core::{compliance_za, Invoice, InvoiceTotals, Money};
use serde::Serialize;
use tower_http::sensitive_headers::SetSensitiveRequestHeadersLayer;

/// The API's routes, before the shared middleware and state are attached.
/// (Split from [`app`] so tests can add routes of their own.)
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/health", get(health))
        .route("/v1/whoami", get(whoami))
        .route("/v1/invoices/calculate", post(calculate))
}

/// Attaches the middleware and state. The `Authorization` header is marked
/// sensitive for everything downstream, so it prints as redacted if a request
/// is ever logged or debug-printed.
pub fn finish(routes: Router<AppState>, state: AppState) -> Router {
    routes
        .layer(SetSensitiveRequestHeadersLayer::new([header::AUTHORIZATION]))
        .with_state(state)
}

/// The complete API.
pub fn app(state: AppState) -> Router {
    finish(routes(), state)
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
    version: &'static str,
}

/// The only route that needs no key.
async fn health() -> Json<Health> {
    Json(Health {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

#[derive(Serialize)]
struct WhoAmI {
    tenant: String,
    key_id: String,
    scopes: Vec<&'static str>,
}

/// Lets a client check which tenant, key and scopes its credentials carry.
/// Needs a valid key but no particular scope. Never echoes the key.
async fn whoami(auth: Authed) -> Json<WhoAmI> {
    let principal = auth.principal();
    Json(WhoAmI {
        tenant: principal.tenant().to_string(),
        key_id: principal.key_id().to_string(),
        scopes: principal
            .scopes()
            .into_iter()
            .map(|scope| scope.as_str())
            .collect(),
    })
}

#[derive(Serialize)]
struct Calculation {
    totals: InvoiceTotals,
    required_document: compliance_za::TaxInvoiceKind,
    display_gross: String,
}

#[derive(Serialize)]
struct ApiError {
    error: String,
}

/// Pure arithmetic on what the caller sends: it touches no stored data, so it
/// needs a valid key but no particular scope. The key is checked before the
/// body is read.
async fn calculate(
    _auth: Authed,
    Json(invoice): Json<Invoice>,
) -> Result<Json<Calculation>, (StatusCode, Json<ApiError>)> {
    let totals = invoice.totals().map_err(|e| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ApiError { error: e.to_string() }),
        )
    })?;
    let gross: Money = totals.gross;
    Ok(Json(Calculation {
        totals,
        required_document: compliance_za::required_kind(gross),
        display_gross: gross.to_string(),
    }))
}

#[cfg(test)]
mod tests;
