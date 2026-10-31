// Written by Kone Tshivhinda & Claude
//! The HTTP rules for the register, against a fake store (no database).

use crate::results::{Results, ResultsFuture};
use crate::tests::{bearer, json, send, FakeAuth, ADMIN_ONLY, READER};
use crate::{finish, routes, AppState};
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use clearcut_core::document::DocumentStatus;
use clearcut_core::event::{
    Citation, Confidence, DocumentId, Event, EventId, EventParts, EventPayload, Extractor,
};
use clearcut_core::register::{RegisterFacts, RegisterInput, Selection, MAX_DOCUMENTS};
use clearcut_store::{DocId, DocumentInfo, DocumentSummary, Error, SearchHit, TenantId};
use std::num::NonZeroU32;
use std::sync::Arc;
use time::OffsetDateTime;

const ONE: &str = "00000000-0000-0000-0000-000000000001";

/// What the store would hand in: a fixed number of finished documents, or the error a named id
/// that is not a UUID produces.
struct Fake {
    facts: usize,
    invalid_id: bool,
}

impl Fake {
    fn fact(&self, n: usize) -> RegisterFacts {
        RegisterFacts {
            id: format!("00000000-0000-0000-0000-{n:012}"),
            filename: format!("invoice-{n}.pdf"),
            uploaded_at: OffsetDateTime::UNIX_EPOCH,
            status: DocumentStatus::Done,
            page_count: Some(1),
            pages: Vec::new(),
            events: vec![Event::new(EventParts {
                id: EventId::new(format!("doc_{n}:r1:supplier")),
                date: None,
                payload: EventPayload::SupplierIdentified {
                    name: format!("Acme {n} (Pty) Ltd"),
                    vat_number: None,
                },
                citation: Citation {
                    document: DocumentId::new(format!("doc_{n}")),
                    page: NonZeroU32::new(1).unwrap(),
                    locator: None,
                },
                extractor: Extractor::System,
                confidence: Confidence::new(950).unwrap(),
                supersedes: None,
            })
            .unwrap()],
        }
    }
}

impl Results for Arc<Fake> {
    fn list<'a>(&'a self, _tenant: TenantId, _limit: u32) -> ResultsFuture<'a, Vec<DocumentSummary>> {
        Box::pin(async { unreachable!("the register has its own tests") })
    }

    fn info<'a>(&'a self, _tenant: TenantId, _document: DocId) -> ResultsFuture<'a, DocumentInfo> {
        Box::pin(async { unreachable!("the register has its own tests") })
    }

    fn events<'a>(&'a self, _tenant: TenantId, _document: DocId) -> ResultsFuture<'a, Vec<Event>> {
        Box::pin(async { unreachable!("the register has its own tests") })
    }

    fn search<'a>(
        &'a self,
        _tenant: TenantId,
        _query: &'a str,
        _limit: u32,
    ) -> ResultsFuture<'a, Vec<SearchHit>> {
        Box::pin(async { unreachable!("the register has its own tests") })
    }

    fn register<'a>(
        &'a self,
        _tenant: TenantId,
        selection: &'a Selection,
        only_ready: bool,
    ) -> ResultsFuture<'a, RegisterInput> {
        Box::pin(async move {
            if self.invalid_id {
                return Err(Error::InvalidSelectionId("not-a-uuid".into()));
            }
            Ok(RegisterInput {
                selection: selection.clone(),
                only_ready,
                undated_in_tenant: 0,
                facts: (0..self.facts).map(|n| self.fact(n)).collect(),
            })
        })
    }
}

fn app(fake: &Arc<Fake>) -> Router {
    finish(
        routes(),
        AppState::shared(FakeAuth::new()).with_results(fake.clone()),
    )
}

fn fake(facts: usize, invalid_id: bool) -> Arc<Fake> {
    Arc::new(Fake { facts, invalid_id })
}

fn post(token: Option<&str>, body: &str) -> Request<Body> {
    let mut builder = Request::post("/v1/register").header(header::CONTENT_TYPE, "application/json");
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, bearer(token));
    }
    builder.body(Body::from(body.to_string())).unwrap()
}

/// A request that names one document, whatever the format.
fn chosen(format: &str) -> String {
    serde_json::json!({ "document_ids": [ONE], "format": format }).to_string()
}

#[tokio::test]
async fn the_key_is_checked_before_the_body_and_the_scope_is_documents_read() {
    let app = app(&fake(1, false));
    for (token, expected) in [
        (None, StatusCode::UNAUTHORIZED),
        (Some(ADMIN_ONLY), StatusCode::FORBIDDEN),
    ] {
        let (status, _, body) = send(&app, post(token, "{not json")).await;
        assert_eq!(status, expected, "{body}");
    }
}

#[tokio::test]
async fn without_a_store_the_register_is_unavailable() {
    let app = finish(routes(), AppState::shared(FakeAuth::new()));
    let (status, _, _) = send(
        &app,
        post(Some(READER), r#"{"from":"2026-03-01","to":"2026-03-31"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn the_default_file_is_an_excel_workbook_named_for_the_period() {
    let app = app(&fake(1, false));
    let (status, headers, body) = send(
        &app,
        post(Some(READER), r#"{"from":"2026-03-01","to":"2026-03-31"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers[header::CONTENT_TYPE],
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
    );
    assert_eq!(
        headers[header::CONTENT_DISPOSITION],
        "attachment; filename=\"clearcut-register-2026-03-01-to-2026-03-31.xlsx\""
    );
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    assert!(body.starts_with("PK"), "an Excel file is a zip");
}

#[tokio::test]
async fn a_chosen_list_wins_over_a_period_and_names_the_file_for_the_selection() {
    let app = app(&fake(1, false));
    let body = serde_json::json!({
        "from": "2026-03-01",
        "to": "2026-03-31",
        "document_ids": [ONE],
        "format": "json",
    })
    .to_string();
    let (status, headers, body) = send(&app, post(Some(READER), &body)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "application/json");
    assert_eq!(
        headers[header::CONTENT_DISPOSITION],
        "attachment; filename=\"clearcut-register-selection.json\""
    );
    let view = json(&body);
    assert_eq!(view["selection"]["type"], "documents");
    assert_eq!(view["manifest"]["documents"], 1);
    assert_eq!(view["rows"][0]["supplier"], "Acme 0 (Pty) Ltd");

    // The list wins even when the period beside it could not be used.
    let body = serde_json::json!({
        "from": "not-a-date",
        "document_ids": [ONE],
        "format": "json",
    })
    .to_string();
    let (status, _, answer) = send(&app, post(Some(READER), &body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
}

#[tokio::test]
async fn the_csv_bundle_is_a_zip_and_the_pdf_starts_with_pdf() {
    let app = app(&fake(1, false));
    for (format, content_type, extension, magic) in [
        ("csv", "application/zip", "zip", "PK"),
        ("pdf", "application/pdf", "pdf", "%PDF-"),
    ] {
        let (status, headers, body) = send(&app, post(Some(READER), &chosen(format))).await;
        assert_eq!(status, StatusCode::OK, "{format}");
        assert_eq!(headers[header::CONTENT_TYPE], content_type, "{format}");
        assert_eq!(
            headers[header::CONTENT_DISPOSITION],
            format!("attachment; filename=\"clearcut-register-selection.{extension}\""),
            "{format}"
        );
        assert!(body.starts_with(magic), "{format}");
    }
}

#[tokio::test]
async fn only_ready_is_honoured_and_the_file_says_so() {
    let app = app(&fake(1, false));
    let (status, _, body) = send(
        &app,
        post(
            Some(READER),
            r#"{"from":"2026-03-01","to":"2026-03-31","format":"json","only_ready":true}"#,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let view = json(&body);
    assert_eq!(view["only_ready"], true);
    assert_eq!(view["manifest"]["only_ready"], true);
}

#[tokio::test]
async fn a_period_or_a_list_that_makes_no_sense_is_refused() {
    let app = app(&fake(1, false));
    for body in [
        r#"{}"#,
        r#"{"from":"2026-03-01"}"#,
        r#"{"to":"2026-03-31"}"#,
        r#"{"from":"2026-13-01","to":"2026-03-31"}"#,
        r#"{"from":"2026-03-31","to":"2026-03-01"}"#,
        r#"{"document_ids":[]}"#,
    ] {
        let (status, _, answer) = send(&app, post(Some(READER), body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {answer}");
    }
    let many = serde_json::json!({
        "document_ids": (0..=MAX_DOCUMENTS)
            .map(|n| format!("00000000-0000-0000-0000-{n:012}"))
            .collect::<Vec<_>>(),
    })
    .to_string();
    let (status, _, answer) = send(&app, post(Some(READER), &many)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "501 ids: {answer}");
}

#[tokio::test]
async fn a_period_over_the_limit_is_refused_with_a_sentence() {
    let app = app(&fake(MAX_DOCUMENTS + 1, false));
    let (status, _, body) = send(
        &app,
        post(
            Some(READER),
            r#"{"from":"2026-03-01","to":"2026-03-31","format":"json"}"#,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("split the period"), "{body}");
}

#[tokio::test]
async fn a_format_that_does_not_exist_is_refused() {
    let app = app(&fake(1, false));
    // `zip` is the extension of the bundle; the API calls it `csv`.
    for format in ["exe", "zip"] {
        let (status, _, answer) = send(&app, post(Some(READER), &chosen(format))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{format}: {answer}");
    }
}

#[tokio::test]
async fn a_named_document_that_is_not_the_accounts_is_not_found() {
    let app = app(&fake(0, false));
    let (status, _, body) = send(&app, post(Some(READER), &chosen("json"))).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

#[tokio::test]
async fn an_id_that_is_not_a_document_id_is_refused() {
    let app = app(&fake(0, true));
    let (status, _, _) = send(&app, post(Some(READER), r#"{"document_ids":["not-a-uuid"]}"#)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
