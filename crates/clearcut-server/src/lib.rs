// Written by Kone Tshivhinda & Claude
//! ClearCut HTTP API. Thin transport over `clearcut-core`.
//! The MCP host (Phase 1) will expose the same operations as MCP tools.

use axum::{
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use clearcut_core::{compliance_za, Invoice, InvoiceTotals, Money};
use serde::Serialize;

pub fn app() -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/invoices/calculate", post(calculate))
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
    version: &'static str,
}

async fn health() -> Json<Health> {
    Json(Health {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
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

async fn calculate(Json(invoice): Json<Invoice>) -> Result<Json<Calculation>, (StatusCode, Json<ApiError>)> {
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
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    #[tokio::test]
    async fn health_ok() {
        let res = app()
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn calculates_vat() {
        let body = r#"{"number":"INV-1","currency":"ZAR","status":"draft","lines":[
            {"description":"Consulting","quantity_milli":10000,
             "unit_price":{"minor":150000,"currency":"ZAR"},"tax":"standard"}]}"#;
        let req = Request::post("/v1/invoices/calculate")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();
        let res = app().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["totals"]["gross"]["minor"], 1_725_000);
        assert_eq!(v["required_document"], "full");
        assert_eq!(v["display_gross"], "R17250.00");
    }

    #[tokio::test]
    async fn empty_invoice_is_422() {
        let body = r#"{"number":"X","currency":"ZAR","status":"draft","lines":[]}"#;
        let req = Request::post("/v1/invoices/calculate")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();
        let res = app().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
}
