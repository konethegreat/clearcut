// Written by Kone Tshivhinda & Claude
//! The HTTP rules for review, against a fake (no database). What a person's reading must satisfy
//! is tested in `clearcut-core`; what is stored in `clearcut-store`; the whole path in
//! `clearcut-pipeline`.

use super::*;
use crate::tests::{bearer, json, send, FakeAuth, ADMIN_ONLY, READER, WRITER};
use crate::{finish, routes, AppState};
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use std::sync::{Arc, Mutex};

const DOC: u128 = 0xD0C;

#[derive(Default)]
struct Fake {
    /// What `confirm` answers; the reviewers it was asked for are kept.
    answer: Mutex<Option<ReviewError>>,
    asked: Mutex<Vec<(u32, String)>>,
}

impl Reviews for Arc<Fake> {
    fn waiting<'a>(&'a self, _tenant: TenantId, _document: DocId) -> ReviewFuture<'a, Vec<WaitingPage>> {
        Box::pin(async move {
            Ok(vec![WaitingPage {
                page: 1,
                reason: "tax_not_shown",
                problems: json!([{ "path": "lines/1/tax", "kind": "tax_not_shown" }]),
                findings: json!([]),
                reading: None,
            }])
        })
    }

    fn confirm<'a>(
        &'a self,
        _tenant: TenantId,
        _document: DocId,
        page: u32,
        _reading: PageReading,
        by: Reviewer,
    ) -> ReviewFuture<'a, Uuid> {
        Box::pin(async move {
            self.asked.lock().unwrap().push((page, by.label));
            match self.answer.lock().unwrap().take() {
                Some(error) => Err(error),
                None => Ok(Uuid::from_u128(0x70B)),
            }
        })
    }
}

fn app(fake: &Arc<Fake>) -> Router {
    finish(
        routes(),
        AppState::shared(FakeAuth::new()).with_reviews(fake.clone()),
    )
}

fn reading() -> Value {
    json!({
        "currency": "ZAR", "amounts_include_vat": "no", "invoice_number": "", "invoice_date": "",
        "supplier": { "name": "", "address": "", "vat_number": "" },
        "recipient": { "name": "", "address": "", "vat_number": "" },
        "lines": [], "totals": { "net": "", "vat": "", "gross": "" },
        "due": { "date": "", "terms": "" },
        "credit_note": { "original_invoice_number": "", "amount": "" }
    })
}

fn post(token: &str, body: &Value) -> Request<Body> {
    Request::post(format!("/v1/documents/{}/pages/1/review", Uuid::from_u128(DOC)))
        .header(header::AUTHORIZATION, bearer(token))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

#[tokio::test]
async fn reading_the_waiting_pages_needs_documents_read_and_confirming_needs_documents_write() {
    let fake = Arc::new(Fake::default());
    let app = app(&fake);
    let with = |token: &str| {
        Request::get(format!("/v1/documents/{}/review", Uuid::from_u128(DOC)))
            .header(header::AUTHORIZATION, bearer(token))
            .body(Body::empty())
            .unwrap()
    };
    let (status, _, body) = send(&app, with(READER)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json(&body)["pages"][0]["reason"], "tax_not_shown");
    assert_eq!(send(&app, with(ADMIN_ONLY)).await.0, StatusCode::FORBIDDEN);

    // A reader cannot confirm; nothing reaches the review code.
    assert_eq!(
        send(&app, post(READER, &reading())).await.0,
        StatusCode::FORBIDDEN
    );
    assert!(fake.asked.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_confirmation_names_the_key_that_made_it_and_answers_with_the_queued_job() {
    let fake = Arc::new(Fake::default());
    let (status, _, body) = send(&app(&fake), post(WRITER, &reading())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json(&body)["job_id"], Uuid::from_u128(0x70B).to_string());
    let asked = fake.asked.lock().unwrap();
    assert_eq!(asked[0].0, 1);
    assert!(asked[0].1.starts_with("api_key:"), "{}", asked[0].1);
}

#[tokio::test]
async fn a_refusal_says_where_and_other_answers_are_plain() {
    let fake = Arc::new(Fake::default());
    let app = app(&fake);
    let refusal = Refusal {
        currency_missing: false,
        problems: vec![clearcut_core::reading::ReadingProblem {
            path: "lines/1/tax".into(),
            kind: clearcut_core::reading::ProblemKind::TaxNotShown,
        }],
        findings: vec![],
    };
    *fake.answer.lock().unwrap() = Some(ReviewError::Refused(Box::new(refusal)));
    let (status, _, body) = send(&app, post(WRITER, &reading())).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(json(&body)["refusal"]["problems"][0]["path"], "lines/1/tax");

    for (error, expected) in [
        (ReviewError::NotWaiting, StatusCode::CONFLICT),
        (ReviewError::Busy, StatusCode::CONFLICT),
        (
            ReviewError::Store(Box::new(clearcut_store::Error::DocumentNotFound)),
            StatusCode::NOT_FOUND,
        ),
    ] {
        *fake.answer.lock().unwrap() = Some(error);
        assert_eq!(send(&app, post(WRITER, &reading())).await.0, expected);
    }

    // A body that is not a reading is refused before the review code is reached.
    let before = fake.asked.lock().unwrap().len();
    assert_eq!(
        send(&app, post(WRITER, &json!({ "currency": "ZAR" }))).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(fake.asked.lock().unwrap().len(), before);
}
