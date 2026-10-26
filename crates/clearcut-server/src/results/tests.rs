// Written by Kone Tshivhinda & Claude
//! The HTTP rules for reading a document back, against a fake store (no database).

use super::*;
use crate::tests::{bearer, json, send, FakeAuth, ADMIN_ONLY, READER, TENANT};
use crate::{finish, routes, AppState};
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use clearcut_core::document::DocumentStatus;
use clearcut_core::event::{Citation, Confidence, DocumentId, EventId, EventParts, EventPayload, Extractor};
use clearcut_store::DocumentSummary;
use clearcut_store::Error;
use std::num::NonZeroU32;
use std::sync::Arc;

const DOC: u128 = 0xD0C;

struct Fake {
    supplier: String,
    missing: bool,
}

impl Results for Arc<Fake> {
    fn list<'a>(&'a self, _tenant: TenantId, _limit: u32) -> ResultsFuture<'a, Vec<DocumentSummary>> {
        Box::pin(async move {
            Ok(vec![DocumentSummary {
                id: DocId(Uuid::from_u128(DOC)),
                status: DocumentStatus::Done,
                page_count: Some(2),
                original_filename: "invoice.pdf".into(),
                created_at: time::OffsetDateTime::UNIX_EPOCH,
            }])
        })
    }

    fn info<'a>(&'a self, _tenant: TenantId, _document: DocId) -> ResultsFuture<'a, DocumentInfo> {
        Box::pin(async move {
            if self.missing {
                return Err(Error::DocumentNotFound);
            }
            Ok(DocumentInfo {
                status: DocumentStatus::Done,
                page_count: Some(2),
                original_filename: "invoice.pdf".into(),
            })
        })
    }

    fn events<'a>(&'a self, _tenant: TenantId, _document: DocId) -> ResultsFuture<'a, Vec<Event>> {
        Box::pin(async move {
            let event = Event::new(EventParts {
                id: EventId::new("doc_1:r1:supplier"),
                date: None,
                payload: EventPayload::SupplierIdentified {
                    name: self.supplier.clone(),
                    vat_number: Some("4123456789".into()),
                },
                citation: Citation {
                    document: DocumentId::new("doc_1"),
                    page: NonZeroU32::new(1).unwrap(),
                    locator: None,
                },
                extractor: Extractor::System,
                confidence: Confidence::new(950).unwrap(),
                supersedes: None,
            })
            .unwrap();
            Ok(vec![event])
        })
    }
}

fn app(fake: &Arc<Fake>) -> Router {
    finish(
        routes(),
        AppState::shared(FakeAuth::new()).with_results(fake.clone()),
    )
}

fn fake(supplier: &str) -> Arc<Fake> {
    Arc::new(Fake {
        supplier: supplier.into(),
        missing: false,
    })
}

fn get(token: Option<&str>, path: &str) -> Request<Body> {
    let mut builder = Request::get(format!("/v1/documents/{}{path}", Uuid::from_u128(DOC)));
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, bearer(token));
    }
    builder.body(Body::empty()).unwrap()
}

#[tokio::test]
async fn a_key_with_documents_read_sees_the_document_and_others_do_not() {
    let app = app(&fake("Acme (Pty) Ltd"));
    let (status, _, body) = send(&app, get(Some(READER), "")).await;
    assert_eq!(status, StatusCode::OK);
    let view = json(&body);
    assert_eq!(view["status"], "done");
    assert_eq!(view["page_count"], 2);
    assert_eq!(view["events"], 1);
    for path in ["", "/events", "/report", "/export"] {
        assert_eq!(
            send(&app, get(None, path)).await.0,
            StatusCode::UNAUTHORIZED,
            "{path}"
        );
        assert_eq!(
            send(&app, get(Some(ADMIN_ONLY), path)).await.0,
            StatusCode::FORBIDDEN,
            "{path}"
        );
    }
    let _ = TENANT;
}

#[tokio::test]
async fn a_document_that_is_not_the_tenants_is_not_found_and_a_bad_id_is_refused() {
    let missing = Arc::new(Fake {
        supplier: String::new(),
        missing: true,
    });
    let app = app(&missing);
    assert_eq!(send(&app, get(Some(READER), "")).await.0, StatusCode::NOT_FOUND);
    let bad = Request::get("/v1/documents/not-an-id")
        .header(header::AUTHORIZATION, bearer(READER))
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&app, bad).await.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn the_events_the_report_and_the_exports_are_made_from_the_stored_events() {
    let app = app(&fake("Acme (Pty) Ltd"));

    let (status, _, body) = send(&app, get(Some(READER), "/events")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json(&body)["events"][0]["id"], "doc_1:r1:supplier");

    let (status, _, body) = send(&app, get(Some(READER), "/report?mode=timeline")).await;
    assert_eq!(status, StatusCode::OK);
    let report = json(&body);
    let sentence = &report["sections"][0]["sentences"][0];
    assert!(sentence["text"].as_str().unwrap().contains("Acme (Pty) Ltd"));
    assert_eq!(sentence["cites"][0], "doc_1:r1:supplier");

    let (status, headers, body) = send(&app, get(Some(READER), "/export?format=md")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers[header::CONTENT_TYPE]
        .to_str()
        .unwrap()
        .starts_with("text/markdown"));
    assert!(headers[header::CONTENT_DISPOSITION]
        .to_str()
        .unwrap()
        .contains("attachment"));
    assert!(
        body.contains("# invoice.pdf")
            && body.contains("Acme (Pty) Ltd")
            && body.contains("doc_1:r1:supplier")
    );

    let (status, headers, body) = send(&app, get(Some(READER), "/export")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "application/json");
    let file = json(&body);
    assert_eq!(file["document"]["status"], "done");
    assert_eq!(file["events"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn text_read_off_a_document_cannot_become_a_formula_in_the_csv() {
    let app = app(&fake("=HYPERLINK(\"http://example.invalid\",\"x\")"));
    let (status, headers, body) = send(&app, get(Some(READER), "/export?format=csv")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers[header::CONTENT_TYPE]
        .to_str()
        .unwrap()
        .starts_with("text/csv"));
    let mut lines = body.lines();
    assert_eq!(
        lines.next().unwrap(),
        "event_id,date,group,type,severity,confidence_per_mille,page,text"
    );
    let row = lines.next().unwrap();
    assert!(row.starts_with("doc_1:r1:supplier,"), "{row}");
    // The sentence starts with words, so the "=" sits mid-text and runs nothing; no cell starts with one.
    assert!(row.contains("The supplier is =HYPERLINK"), "{row}");
    assert!(!row.contains(",=") && !row.contains(",\"="), "{row}");
}

#[tokio::test]
async fn a_mode_or_a_format_that_does_not_exist_is_refused() {
    let app = app(&fake("Acme"));
    assert_eq!(
        send(&app, get(Some(READER), "/report?mode=poem")).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        send(&app, get(Some(READER), "/export?format=exe")).await.0,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn without_a_store_reading_is_unavailable() {
    let app = finish(routes(), AppState::shared(FakeAuth::new()));
    assert_eq!(
        send(&app, get(Some(READER), "")).await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
async fn the_list_is_for_documents_read_and_shows_what_a_table_needs() {
    let app = app(&fake("Acme"));
    let list = |token: Option<&str>| {
        let mut builder = Request::get("/v1/documents");
        if let Some(token) = token {
            builder = builder.header(header::AUTHORIZATION, bearer(token));
        }
        builder.body(Body::empty()).unwrap()
    };
    assert_eq!(send(&app, list(None)).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(send(&app, list(Some(ADMIN_ONLY))).await.0, StatusCode::FORBIDDEN);
    let (status, _, body) = send(&app, list(Some(READER))).await;
    assert_eq!(status, StatusCode::OK);
    let first = &json(&body)["documents"][0];
    assert_eq!(first["original_filename"], "invoice.pdf");
    assert_eq!(first["status"], "done");
    assert_eq!(first["created_at"], "1970-01-01T00:00:00Z");
}

#[tokio::test]
async fn the_events_come_with_a_table_of_one_line_each() {
    let app = app(&fake("Acme (Pty) Ltd"));
    let (_, _, body) = send(&app, get(Some(READER), "/events")).await;
    let row = &json(&body)["table"][0];
    assert_eq!(row["id"], "doc_1:r1:supplier");
    assert_eq!(row["group"], "parties");
    assert_eq!(row["page"], 1);
    assert_eq!(row["confidence_per_mille"], 950);
    assert!(row["text"].as_str().unwrap().contains("Acme (Pty) Ltd"));
}

#[tokio::test]
async fn every_export_format_comes_back_as_its_own_kind_of_file() {
    use tower::ServiceExt;
    let app = app(&fake("Acme (Pty) Ltd"));
    for (format, content_type, magic) in [
        ("pdf", "application/pdf", &b"%PDF-"[..]),
        (
            "xlsx",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            b"PK",
        ),
        (
            "docx",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            b"PK",
        ),
        ("html", "text/html; charset=utf-8", b"<!doctype html>"),
        ("txt", "text/plain; charset=utf-8", b"invoice.pdf"),
    ] {
        let response = app
            .clone()
            .oneshot(get(Some(READER), &format!("/export?format={format}")))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{format}");
        assert_eq!(response.headers()[header::CONTENT_TYPE], content_type);
        assert!(response.headers()[header::CONTENT_DISPOSITION]
            .to_str()
            .unwrap()
            .ends_with(&format!(".{format}\"")));
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(bytes.starts_with(magic), "{format}");
    }
}
