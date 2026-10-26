// Written by Kone Tshivhinda & Claude
//! The HTTP rules for `POST /v1/documents`, against a fake intake (no
//! database). The last group runs the real intake, store and disk.

use super::*;
use crate::tests::{bearer, json, send, FakeAuth, ADMIN_ONLY, READER, TENANT, WRITER};
use crate::{finish, routes, AppState};
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use clearcut_intake::MediaType;
use clearcut_store::DocId;
use std::sync::{Arc, Mutex};
use tokio::io::AsyncReadExt;
use uuid::Uuid;

// ---------- test doubles ----------

/// What the fake intake was handed.
#[derive(Debug)]
struct Call {
    tenant: Uuid,
    filename: Option<String>,
    content_type: String,
    declared_length: Option<u64>,
    /// The bytes read from the body, or the kind of error reading it hit.
    body: Result<Vec<u8>, std::io::ErrorKind>,
}

/// An intake that reads the whole body, records what it saw, and answers with
/// whatever it was told to.
struct FakeIngest {
    calls: Mutex<Vec<Call>>,
    reply: fn() -> Result<Uploaded, IntakeError>,
}

impl FakeIngest {
    fn replying(reply: fn() -> Result<Uploaded, IntakeError>) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            reply,
        })
    }

    fn calls(&self) -> std::sync::MutexGuard<'_, Vec<Call>> {
        self.calls.lock().unwrap()
    }
}

impl Ingest for Arc<FakeIngest> {
    fn ingest<'a>(&'a self, by: &'a Principal, mut upload: Upload<UploadBody>) -> IngestFuture<'a> {
        Box::pin(async move {
            let mut bytes = Vec::new();
            let body = match upload.body.read_to_end(&mut bytes).await {
                Ok(_) => Ok(bytes),
                Err(error) => Err(error.kind()),
            };
            self.calls().push(Call {
                tenant: by.tenant(),
                filename: upload.filename,
                content_type: upload.content_type,
                declared_length: upload.declared_length,
                body,
            });
            (self.reply)()
        })
    }
}

fn stored(duplicate: bool) -> Uploaded {
    Uploaded {
        document: DocId(Uuid::from_u128(0xD0C)),
        duplicate,
        sha256_hex: "ab".repeat(32),
        byte_size: 7,
        filename: "March invoice.pdf".into(),
        media_type: MediaType::Pdf,
        job: Some(Uuid::from_u128(0x70B)),
    }
}

fn app_with(ingest: &Arc<FakeIngest>) -> (Router, Arc<FakeAuth>) {
    let fake = FakeAuth::new();
    let state = AppState::shared(fake.clone()).with_ingest(ingest.clone());
    (finish(routes(), state), fake)
}

fn post(
    uri: &str,
    token: Option<&str>,
    content_type: Option<&str>,
    length: Option<&str>,
    body: Body,
) -> Request<Body> {
    let mut builder = Request::post(uri);
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, bearer(token));
    }
    if let Some(content_type) = content_type {
        builder = builder.header(header::CONTENT_TYPE, content_type);
    }
    if let Some(length) = length {
        builder = builder.header(header::CONTENT_LENGTH, length);
    }
    builder.body(body).unwrap()
}

fn pdf_post(token: Option<&str>) -> Request<Body> {
    post(
        "/v1/documents?filename=March%20invoice.pdf",
        token,
        Some("application/pdf"),
        Some("7"),
        Body::from("%PDF-1."),
    )
}

// ---------- who may upload ----------

#[tokio::test]
async fn an_anonymous_upload_is_a_401_and_the_intake_is_never_called() {
    let ingest = FakeIngest::replying(|| Ok(stored(false)));
    let (app, _) = app_with(&ingest);
    let (status, headers, body) = send(&app, pdf_post(None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(headers.get(header::WWW_AUTHENTICATE).unwrap(), "Bearer");
    assert_eq!(
        json(&body),
        serde_json::json!({ "error": "authentication failed" })
    );
    assert!(ingest.calls().is_empty());
}

#[tokio::test]
async fn an_unknown_key_is_the_same_401() {
    let ingest = FakeIngest::replying(|| Ok(stored(false)));
    let (app, _) = app_with(&ingest);
    let (status, ..) = send(&app, pdf_post(Some("ccut_nobody"))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(ingest.calls().is_empty());
}

#[tokio::test]
async fn a_key_without_documents_write_is_a_403_naming_the_scope_and_reads_nothing() {
    for token in [READER, ADMIN_ONLY] {
        let ingest = FakeIngest::replying(|| Ok(stored(false)));
        let (app, _) = app_with(&ingest);
        let (status, _, body) = send(&app, pdf_post(Some(token))).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{token}");
        assert_eq!(json(&body)["required_scope"], "documents:write");
        assert!(ingest.calls().is_empty(), "{token}: the intake must not run");
    }
}

#[tokio::test]
async fn a_database_outage_while_authenticating_is_a_500_not_a_401() {
    let ingest = FakeIngest::replying(|| Ok(stored(false)));
    let (app, _) = app_with(&ingest);
    let (status, ..) = send(&app, pdf_post(Some("ccut_db_is_down"))).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(ingest.calls().is_empty());
}

// ---------- a good upload ----------

#[tokio::test]
async fn a_new_document_is_201_with_its_details_and_nothing_internal() {
    let ingest = FakeIngest::replying(|| Ok(stored(false)));
    let (app, _) = app_with(&ingest);
    let (status, _, body) = send(&app, pdf_post(Some(WRITER))).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        json(&body),
        serde_json::json!({
            "document_id": Uuid::from_u128(0xD0C).to_string(),
            "job_id": Uuid::from_u128(0x70B).to_string(),
            "duplicate": false,
            "sha256": "ab".repeat(32),
            "byte_size": 7,
            "filename": "March invoice.pdf",
            "media_type": "application/pdf",
        })
    );
    assert!(!body.contains("tenants/"), "storage keys are internal: {body}");
}

#[tokio::test]
async fn the_reported_media_type_is_the_one_the_intake_recorded() {
    let ingest = FakeIngest::replying(|| {
        Ok(Uploaded {
            media_type: MediaType::Png,
            ..stored(false)
        })
    });
    let (app, _) = app_with(&ingest);
    let (_, _, body) = send(&app, pdf_post(Some(WRITER))).await;
    assert_eq!(json(&body)["media_type"], "image/png");
}

#[tokio::test]
async fn a_repeat_upload_is_a_200_flagged_as_a_duplicate() {
    let ingest = FakeIngest::replying(|| Ok(stored(true)));
    let (app, _) = app_with(&ingest);
    let (status, _, body) = send(&app, pdf_post(Some(WRITER))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json(&body)["duplicate"], true);
}

#[tokio::test]
async fn the_intake_gets_the_callers_tenant_the_name_the_type_the_length_and_the_exact_bytes() {
    let ingest = FakeIngest::replying(|| Ok(stored(false)));
    let (app, _) = app_with(&ingest);
    send(&app, pdf_post(Some(WRITER))).await;
    let calls = ingest.calls();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(call.tenant, Uuid::from_u128(TENANT));
    assert_eq!(call.filename.as_deref(), Some("March invoice.pdf"));
    assert_eq!(call.content_type, "application/pdf");
    assert_eq!(call.declared_length, Some(7));
    assert_eq!(call.body.as_ref().unwrap(), b"%PDF-1.");
}

#[tokio::test]
async fn the_content_type_is_passed_exactly_as_sent_and_a_missing_one_as_empty() {
    let ingest = FakeIngest::replying(|| Ok(stored(false)));
    let (app, _) = app_with(&ingest);
    let sent = "Application/PDF; charset=binary";
    send(
        &app,
        post("/v1/documents", Some(WRITER), Some(sent), None, Body::from("x")),
    )
    .await;
    send(
        &app,
        post("/v1/documents", Some(WRITER), None, None, Body::from("x")),
    )
    .await;
    let calls = ingest.calls();
    assert_eq!(calls[0].content_type, sent);
    assert_eq!(calls[1].content_type, "");
}

#[tokio::test]
async fn no_filename_means_none_and_an_unreadable_length_means_none() {
    let ingest = FakeIngest::replying(|| Ok(stored(false)));
    let (app, _) = app_with(&ingest);
    send(
        &app,
        post(
            "/v1/documents",
            Some(WRITER),
            Some("application/pdf"),
            Some("seven"),
            Body::from("x"),
        ),
    )
    .await;
    let calls = ingest.calls();
    assert_eq!(calls[0].filename, None);
    assert_eq!(calls[0].declared_length, None);
}

#[tokio::test]
async fn a_filename_with_odd_characters_arrives_decoded() {
    let ingest = FakeIngest::replying(|| Ok(stored(false)));
    let (app, _) = app_with(&ingest);
    send(
        &app,
        post(
            "/v1/documents?filename=Caf%C3%A9%20%26%20Co%2FMarch.pdf",
            Some(WRITER),
            Some("application/pdf"),
            None,
            Body::from("x"),
        ),
    )
    .await;
    assert_eq!(ingest.calls()[0].filename.as_deref(), Some("Café & Co/March.pdf"));
}

#[tokio::test]
async fn a_large_body_streams_through_past_axums_default_two_megabyte_limit() {
    let big = vec![b'x'; 3 * 1024 * 1024];
    let ingest = FakeIngest::replying(|| Ok(stored(false)));
    let (app, _) = app_with(&ingest);
    let (status, ..) = send(
        &app,
        post(
            "/v1/documents",
            Some(WRITER),
            Some("application/pdf"),
            Some(&big.len().to_string()),
            Body::from(big.clone()),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(ingest.calls()[0].body.as_ref().unwrap().len(), big.len());
}

#[tokio::test]
async fn a_body_that_breaks_off_reaches_the_intake_as_an_aborted_connection() {
    let broken = futures_util::stream::iter(vec![
        Ok(bytes::Bytes::from_static(b"%PDF-1.")),
        Err(std::io::Error::other("the client went away")),
    ]);
    let ingest = FakeIngest::replying(|| Err(IntakeError::Interrupted));
    let (app, _) = app_with(&ingest);
    let (status, ..) = send(
        &app,
        post(
            "/v1/documents",
            Some(WRITER),
            Some("application/pdf"),
            None,
            Body::from_stream(broken),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        ingest.calls()[0].body.as_ref().unwrap_err(),
        &std::io::ErrorKind::ConnectionAborted
    );
}

// ---------- what each refusal looks like ----------

#[tokio::test]
async fn each_intake_refusal_has_its_own_status_and_leaks_nothing() {
    type Reply = fn() -> Result<Uploaded, IntakeError>;
    let cases: Vec<(&str, Reply, StatusCode, serde_json::Value)> = vec![
        (
            "unsupported type",
            || Err(IntakeError::UnsupportedMediaType),
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            serde_json::json!({
                "error": "unsupported media type",
                "accepted": [
                    "application/pdf", "image/png", "image/jpeg", "image/tiff", "application/xml"
                ],
            }),
        ),
        (
            "too large",
            || Err(IntakeError::TooLarge { max_bytes: 1000 }),
            StatusCode::PAYLOAD_TOO_LARGE,
            serde_json::json!({ "error": "file too large", "max_bytes": 1000 }),
        ),
        (
            "empty",
            || Err(IntakeError::Empty),
            StatusCode::BAD_REQUEST,
            serde_json::json!({ "error": "the file is empty" }),
        ),
        (
            "interrupted",
            || Err(IntakeError::Interrupted),
            StatusCode::BAD_REQUEST,
            serde_json::json!({ "error": "the upload was interrupted" }),
        ),
        (
            "intake says forbidden",
            || Err(IntakeError::Forbidden(Scope::DocumentsWrite)),
            StatusCode::FORBIDDEN,
            serde_json::json!({ "error": "forbidden", "required_scope": "documents:write" }),
        ),
        (
            "storage failure",
            || {
                Err(IntakeError::Storage(clearcut_objects::Error::Io(
                    std::io::Error::other("disk /srv/secret-path is full"),
                )))
            },
            StatusCode::INTERNAL_SERVER_ERROR,
            serde_json::json!({ "error": "internal error" }),
        ),
        (
            "database failure",
            || Err(IntakeError::Database(clearcut_store::Error::DocumentNotFound)),
            StatusCode::INTERNAL_SERVER_ERROR,
            serde_json::json!({ "error": "internal error" }),
        ),
    ];
    for (name, reply, status, expected) in cases {
        let ingest = FakeIngest::replying(reply);
        let (app, _) = app_with(&ingest);
        let (got, _, body) = send(&app, pdf_post(Some(WRITER))).await;
        assert_eq!(got, status, "{name}");
        assert_eq!(json(&body), expected, "{name}");
        assert!(!body.contains("secret-path"), "{name}: {body}");
    }
}

#[tokio::test]
async fn uploads_with_no_intake_configured_are_a_503() {
    let fake = FakeAuth::new();
    let app = finish(routes(), AppState::shared(fake));
    let (status, _, body) = send(&app, pdf_post(Some(WRITER))).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        json(&body),
        serde_json::json!({ "error": "uploads are not available" })
    );
}

#[tokio::test]
async fn the_route_only_answers_post() {
    let ingest = FakeIngest::replying(|| Ok(stored(false)));
    let (app, _) = app_with(&ingest);
    let request = Request::get("/v1/documents")
        .header(header::AUTHORIZATION, bearer(WRITER))
        .body(Body::empty())
        .unwrap();
    let (status, ..) = send(&app, request).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert!(ingest.calls().is_empty());
}

// ---------- the real intake, store and disk ----------

mod with_postgres {
    use super::*;
    use crate::tests::with_postgres::{first_key, service_store};
    use clearcut_intake::Intake;
    use clearcut_objects::LocalDisk;
    use sqlx::PgPool;

    const PDF: &[u8] = b"%PDF-1.7\n1 0 obj\n<<>>\nendobj\ntrailer\n%%EOF";

    fn files_under(dir: &std::path::Path) -> usize {
        std::fs::read_dir(dir).map_or(0, |entries| {
            entries
                .map(|entry| entry.unwrap().path())
                .map(|path| if path.is_dir() { files_under(&path) } else { 1 })
                .sum()
        })
    }

    async fn real_app(pool: &PgPool, disk: &tempfile::TempDir, max_bytes: u64) -> Router {
        let store = service_store(pool).await;
        let intake = Intake::new(store.clone(), LocalDisk::new(disk.path()), max_bytes);
        finish(routes(), AppState::new(store).with_ingest(intake))
    }

    fn upload(token: &str, bytes: &'static [u8], content_type: &str) -> Request<Body> {
        post(
            "/v1/documents?filename=a.pdf",
            Some(token),
            Some(content_type),
            Some(&bytes.len().to_string()),
            Body::from(bytes),
        )
    }

    #[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
    async fn a_real_upload_is_stored_recorded_repeated_and_kept_per_tenant(pool: PgPool) {
        let disk = tempfile::tempdir().unwrap();
        let app = real_app(&pool, &disk, 1_000_000).await;
        let (_, token, _) = first_key(&pool, &[Scope::DocumentsWrite]).await;
        let (_, other_token, _) = first_key(&pool, &[Scope::DocumentsWrite]).await;

        let (status, _, body) = send(&app, upload(&token, PDF, "application/pdf")).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let first = json(&body);
        assert_eq!(first["duplicate"], false);
        assert_eq!(first["byte_size"], PDF.len());
        assert_eq!(files_under(disk.path()), 1);

        // The same bytes again: the same document, not a second one.
        let (status, _, body) = send(&app, upload(&token, PDF, "application/pdf")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json(&body)["document_id"], first["document_id"]);
        assert_eq!(json(&body)["duplicate"], true);
        let documents: i64 = sqlx::query_scalar("select count(*) from documents")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(documents, 1);

        // Another tenant uploading the same bytes gets a document of its own.
        let (status, _, body) = send(&app, upload(&other_token, PDF, "application/pdf")).await;
        assert_eq!(status, StatusCode::CREATED);
        assert_ne!(json(&body)["document_id"], first["document_id"]);
        assert_eq!(files_under(disk.path()), 2);
    }

    #[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
    async fn refused_uploads_store_and_record_nothing(pool: PgPool) {
        let disk = tempfile::tempdir().unwrap();
        let app = real_app(&pool, &disk, 10).await;
        let (_, writer, _) = first_key(&pool, &[Scope::DocumentsWrite]).await;
        let (_, reader, _) = first_key(&pool, &[Scope::DocumentsRead]).await;

        let (status, ..) = send(&app, upload(&reader, PDF, "application/pdf")).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, ..) = send(&app, upload(&writer, PDF, "text/html")).await;
        assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
        let (status, _, body) = send(&app, upload(&writer, PDF, "application/pdf")).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{body}");
        assert_eq!(json(&body)["max_bytes"], 10);

        assert_eq!(files_under(disk.path()), 0);
        let documents: i64 = sqlx::query_scalar("select count(*) from documents")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(documents, 0);
    }
}
