// Written by Kone Tshivhinda & Claude
//! Pure parts first, then the whole upload against a real Postgres
//! (`DATABASE_URL`, one throwaway database per test) and a temporary directory.

use super::*;
use clearcut_core::auth::{Principal, Scope};
use clearcut_objects::{LocalDisk, ObjectKey, ObjectStore, Sha256Digest};
use clearcut_store::{Store, TenantId};
use sqlx::PgPool;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use tempfile::TempDir;
use tokio::io::{AsyncRead, AsyncReadExt, ReadBuf};
use uuid::Uuid;

// ---------- media types ----------

#[test]
fn the_accepted_media_types_parse_whatever_their_case_or_parameters() {
    let cases = [
        ("application/pdf", MediaType::Pdf),
        ("Application/PDF", MediaType::Pdf),
        ("application/pdf; charset=binary", MediaType::Pdf),
        ("  application/pdf ; q=1", MediaType::Pdf),
        ("image/png", MediaType::Png),
        ("image/jpeg", MediaType::Jpeg),
        ("image/tiff", MediaType::Tiff),
        ("application/xml", MediaType::Xml),
        ("text/xml; charset=utf-8", MediaType::Xml),
    ];
    for (text, expected) in cases {
        assert_eq!(MediaType::parse(text), Some(expected), "{text}");
    }
}

#[test]
fn everything_else_is_refused_as_a_media_type() {
    for text in [
        "",
        " ",
        "application/octet-stream",
        "text/html",
        "text/plain",
        "image/svg+xml",
        "image/gif",
        "application/x-msdownload",
        "application/pdf2",
        "application/pdfx",
        "xapplication/pdf",
        "application/pdf, text/html",
        "application/pdf text/html",
        "pdf",
        "*/*",
        "application/*",
        "application/pdf\r\nX-Injected: 1",
    ] {
        assert_eq!(MediaType::parse(text), None, "{text:?}");
    }
}

#[test]
fn each_media_type_has_one_canonical_name() {
    assert_eq!(MediaType::Pdf.as_str(), "application/pdf");
    assert_eq!(MediaType::Png.as_str(), "image/png");
    assert_eq!(MediaType::Jpeg.as_str(), "image/jpeg");
    assert_eq!(MediaType::Tiff.as_str(), "image/tiff");
    assert_eq!(MediaType::Xml.as_str(), "application/xml");
    assert_eq!(MediaType::parse("text/xml"), Some(MediaType::Xml));
}

// ---------- file names ----------

#[test]
fn a_file_name_loses_any_directory_part() {
    for (raw, expected) in [
        ("invoice.pdf", "invoice.pdf"),
        ("../../etc/passwd", "passwd"),
        ("..\\..\\windows\\system32\\cmd.exe", "cmd.exe"),
        ("/var/tmp/x.pdf", "x.pdf"),
        ("C:\\Users\\kone\\scan.pdf", "scan.pdf"),
        ("a/b\\c/d.png", "d.png"),
        ("folder/", "upload"),
    ] {
        assert_eq!(clean_filename(Some(raw)), expected, "{raw:?}");
    }
}

#[test]
fn control_and_direction_override_characters_are_removed() {
    // The right-to-left override makes "invoice\u{202e}fdp.exe" display as "invoiceexe.pdf".
    assert_eq!(clean_filename(Some("invoice\u{202e}fdp.exe")), "invoicefdp.exe");
    assert_eq!(clean_filename(Some("a\u{0}b\u{7}c\r\nd.pdf")), "abcd.pdf");
    assert_eq!(clean_filename(Some("\u{200b}\u{feff}x.pdf")), "x.pdf");
    assert_eq!(clean_filename(Some("tab\there.pdf")), "tab here.pdf");
}

#[test]
fn whitespace_and_dots_at_the_ends_are_trimmed_and_runs_collapsed() {
    assert_eq!(clean_filename(Some("  my   invoice  .pdf  ")), "my invoice .pdf");
    assert_eq!(clean_filename(Some("..hidden.pdf..")), "hidden.pdf");
    assert_eq!(clean_filename(Some("...")), "upload");
}

#[test]
fn a_missing_or_unusable_name_becomes_upload() {
    for raw in [
        None,
        Some(""),
        Some("   "),
        Some("."),
        Some(".."),
        Some("\u{0}\u{1}"),
        Some("\u{202e}"),
    ] {
        assert_eq!(clean_filename(raw), "upload", "{raw:?}");
    }
}

#[test]
fn a_long_name_is_cut_to_255_characters_without_splitting_a_character() {
    let long = "é".repeat(400);
    let cleaned = clean_filename(Some(&long));
    assert_eq!(cleaned.chars().count(), 255);
    assert!(cleaned.chars().all(|c| c == 'é'));
    let boundary = format!("{}{}", "a".repeat(254), "😀😀");
    assert_eq!(clean_filename(Some(&boundary)).chars().count(), 255);
}

#[test]
fn ordinary_unicode_names_survive() {
    assert_eq!(
        clean_filename(Some("Faktuur – Mei 2026 (Pty) Ltd.pdf")),
        "Faktuur – Mei 2026 (Pty) Ltd.pdf"
    );
    assert_eq!(clean_filename(Some("发票.pdf")), "发票.pdf");
}

// ---------- test helpers ----------

fn principal(tenant: TenantId, scopes: &[Scope]) -> Principal {
    Principal::new(tenant.0, Uuid::from_u128(77), scopes.iter().copied())
}

async fn app_store(pool: &PgPool) -> Store {
    let options = pool.connect_options().as_ref().clone().username("clearcut_app");
    let service_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
        .unwrap();
    Store::new(service_pool)
}

struct Fixture {
    intake: Intake<LocalDisk>,
    store: Store,
    dir: TempDir,
    tenant: TenantId,
    writer: Principal,
}

/// A restricted-role store, a temp-dir object store and one tenant.
async fn fixture(pool: &PgPool, max_bytes: u64) -> Fixture {
    let tenant = Store::new(pool.clone()).create_tenant("Acme").await.unwrap();
    let store = app_store(pool).await;
    let dir = tempfile::tempdir().unwrap();
    let intake = Intake::new(store.clone(), LocalDisk::new(dir.path()), max_bytes);
    Fixture {
        intake,
        store,
        dir,
        tenant,
        writer: principal(tenant, &[Scope::DocumentsWrite]),
    }
}

fn upload<'a>(bytes: &'a [u8], content_type: &str, name: Option<&str>) -> Upload<&'a [u8]> {
    Upload {
        filename: name.map(str::to_string),
        content_type: content_type.to_string(),
        declared_length: Some(bytes.len() as u64),
        body: bytes,
    }
}

const PDF: &[u8] = b"%PDF-1.7\n1 0 obj\n<<>>\nendobj\ntrailer\n%%EOF";

async fn count(pool: &PgPool, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
}

/// Number of stored files (not counting the temporary upload area).
fn stored_files(dir: &TempDir) -> usize {
    fn walk(path: &std::path::Path, out: &mut usize, skip_tmp: bool) {
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                if !(skip_tmp && path.file_name().is_some_and(|n| n == "tmp")) {
                    walk(&path, out, false);
                }
            } else {
                *out += 1;
            }
        }
    }
    let mut n = 0;
    walk(dir.path(), &mut n, true);
    n
}

/// A body that must never be read: proves a check happens before any reading.
struct MustNotBeRead;

impl AsyncRead for MustNotBeRead {
    fn poll_read(self: Pin<&mut Self>, _: &mut Context<'_>, _: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        panic!("the body was read before the request was checked");
    }
}

/// A body that delivers some bytes and then the connection drops.
struct DropsMidway(bool);

impl AsyncRead for DropsMidway {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.0 {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "client went away",
            )));
        }
        self.0 = true;
        buf.put_slice(b"%PDF-1.7 partial");
        Poll::Ready(Ok(()))
    }
}

// ---------- a good upload ----------

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_pdf_is_stored_recorded_and_audited(pool: PgPool) {
    let f = fixture(&pool, 1_000_000).await;
    let done = f
        .intake
        .upload(
            &f.writer,
            upload(PDF, "application/pdf", Some("March invoice.pdf")),
        )
        .await
        .unwrap();

    assert!(!done.duplicate);
    assert_eq!(done.byte_size, PDF.len() as u64);
    assert_eq!(done.filename, "March invoice.pdf");
    assert_eq!(done.media_type, MediaType::Pdf);

    // The document row.
    let (status, name, content_type, size, key): (String, String, String, i64, String) = sqlx::query_as(
        "select status, original_filename, content_type, byte_size, storage_key from documents where id = $1",
    )
    .bind(done.document.0)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(status, "received");
    assert_eq!(name, "March invoice.pdf");
    assert_eq!(content_type, "application/pdf");
    assert_eq!(size, PDF.len() as i64);

    // The bytes are in the object store, under the row's key, intact, and the
    // recorded hash is the hash of exactly those bytes.
    let disk = LocalDisk::new(f.dir.path());
    let mut stored = Vec::new();
    disk.open(&ObjectKey::parse(&key).unwrap())
        .await
        .unwrap()
        .read_to_end(&mut stored)
        .await
        .unwrap();
    assert_eq!(stored, PDF);
    let hash: Vec<u8> = sqlx::query_scalar("select sha256 from documents where id = $1")
        .bind(done.document.0)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        Sha256Digest::from_hex(&done.sha256_hex)
            .unwrap()
            .as_bytes()
            .to_vec(),
        hash
    );
    assert_eq!(stored_files(&f.dir), 1);
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn the_upload_is_audited_without_any_file_content(pool: PgPool) {
    let f = fixture(&pool, 1_000_000).await;
    let done = f
        .intake
        .upload(&f.writer, upload(PDF, "application/pdf", Some("a.pdf")))
        .await
        .unwrap();
    let (actor, action, subject, detail): (serde_json::Value, String, String, serde_json::Value) =
        sqlx::query_as(
            "select actor, action, subject_id, detail from audit_log where subject_type = 'document'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(action, "document.uploaded");
    assert_eq!(
        actor,
        serde_json::json!({ "type": "api_key", "id": Uuid::from_u128(77).to_string() })
    );
    assert_eq!(subject, done.document.0.to_string());
    assert_eq!(detail["filename"], "a.pdf");
    assert_eq!(detail["byte_size"], PDF.len());
    assert_eq!(detail["duplicate"], false);
    assert!(!detail.to_string().contains("%PDF"));
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn the_stored_file_name_is_the_cleaned_one(pool: PgPool) {
    let f = fixture(&pool, 1_000_000).await;
    let done = f
        .intake
        .upload(
            &f.writer,
            upload(PDF, "application/pdf", Some("../../etc/passwd\u{202e}")),
        )
        .await
        .unwrap();
    assert_eq!(done.filename, "passwd");
    let name: String = sqlx::query_scalar("select original_filename from documents where id = $1")
        .bind(done.document.0)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(name, "passwd");
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_missing_file_name_is_recorded_as_upload(pool: PgPool) {
    let f = fixture(&pool, 1_000_000).await;
    let done = f
        .intake
        .upload(&f.writer, upload(PDF, "application/pdf", None))
        .await
        .unwrap();
    assert_eq!(done.filename, "upload");
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn every_accepted_media_type_can_be_uploaded(pool: PgPool) {
    let f = fixture(&pool, 1_000_000).await;
    for (n, content_type) in [
        "application/pdf",
        "image/png",
        "image/jpeg",
        "image/tiff",
        "application/xml",
        "text/xml",
    ]
    .into_iter()
    .enumerate()
    {
        let bytes = format!("file number {n}").into_bytes();
        let done = f
            .intake
            .upload(&f.writer, upload(&bytes, content_type, None))
            .await
            .unwrap();
        assert_eq!(done.media_type, MediaType::parse(content_type).unwrap());
    }
    assert_eq!(count(&pool, "select count(*) from documents").await, 6);
}

// ---------- the same file twice, and two tenants ----------

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn the_same_content_twice_is_one_document_and_one_file(pool: PgPool) {
    let f = fixture(&pool, 1_000_000).await;
    let first = f
        .intake
        .upload(&f.writer, upload(PDF, "application/pdf", Some("first.pdf")))
        .await
        .unwrap();
    let second = f
        .intake
        .upload(&f.writer, upload(PDF, "application/pdf", Some("second.pdf")))
        .await
        .unwrap();
    assert!(!first.duplicate && second.duplicate);
    assert_eq!(first.document, second.document);
    assert_eq!(count(&pool, "select count(*) from documents").await, 1);
    assert_eq!(stored_files(&f.dir), 1);
    // The attempt is still recorded.
    assert_eq!(
        count(
            &pool,
            "select count(*) from audit_log where action = 'document.uploaded'"
        )
        .await,
        2
    );
    let duplicates = count(
        &pool,
        "select count(*) from audit_log where detail ->> 'duplicate' = 'true'",
    )
    .await;
    assert_eq!(duplicates, 1);
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn two_tenants_uploading_the_same_file_get_separate_documents_and_files(pool: PgPool) {
    let f = fixture(&pool, 1_000_000).await;
    let other_tenant = Store::new(pool.clone()).create_tenant("Other").await.unwrap();
    let other = principal(other_tenant, &[Scope::DocumentsWrite]);
    let mine = f
        .intake
        .upload(&f.writer, upload(PDF, "application/pdf", None))
        .await
        .unwrap();
    let theirs = f
        .intake
        .upload(&other, upload(PDF, "application/pdf", None))
        .await
        .unwrap();
    assert_ne!(mine.document, theirs.document);
    assert!(!theirs.duplicate);
    assert_eq!(stored_files(&f.dir), 2);
    assert_eq!(
        count(&pool, "select count(distinct storage_key) from documents").await,
        2
    );
}

// ---------- refusals leave nothing behind ----------

async fn nothing_was_stored(pool: &PgPool, f: &Fixture) {
    assert_eq!(count(pool, "select count(*) from documents").await, 0);
    assert_eq!(
        count(
            pool,
            "select count(*) from audit_log where action = 'document.uploaded'"
        )
        .await,
        0
    );
    assert_eq!(stored_files(&f.dir), 0);
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn uploading_needs_documents_write_and_the_body_is_not_read_without_it(pool: PgPool) {
    let f = fixture(&pool, 1_000_000).await;
    for scopes in [
        vec![],
        vec![Scope::DocumentsRead],
        vec![Scope::EventsRead, Scope::ReviewWrite],
        vec![Scope::KeysAdmin],
    ] {
        let caller = principal(f.tenant, &scopes);
        let body = Upload {
            filename: None,
            content_type: "application/pdf".into(),
            declared_length: None,
            body: MustNotBeRead,
        };
        let result = f.intake.upload(&caller, body).await;
        assert!(
            matches!(&result, Err(IntakeError::Forbidden(Scope::DocumentsWrite))),
            "{scopes:?}: {result:?}"
        );
    }
    nothing_was_stored(&pool, &f).await;
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn an_unsupported_media_type_is_refused_before_the_body_is_read(pool: PgPool) {
    let f = fixture(&pool, 1_000_000).await;
    for content_type in [
        "text/html",
        "application/octet-stream",
        "",
        "application/pdf, text/html",
    ] {
        let body = Upload {
            filename: None,
            content_type: content_type.into(),
            declared_length: None,
            body: MustNotBeRead,
        };
        let result = f.intake.upload(&f.writer, body).await;
        assert!(
            matches!(result, Err(IntakeError::UnsupportedMediaType)),
            "{content_type:?}"
        );
    }
    nothing_was_stored(&pool, &f).await;
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_declared_length_over_the_limit_is_refused_before_the_body_is_read(pool: PgPool) {
    let f = fixture(&pool, 100).await;
    let body = Upload {
        filename: None,
        content_type: "application/pdf".into(),
        declared_length: Some(101),
        body: MustNotBeRead,
    };
    let result = f.intake.upload(&f.writer, body).await;
    assert!(
        matches!(result, Err(IntakeError::TooLarge { max_bytes: 100 })),
        "{result:?}"
    );
    nothing_was_stored(&pool, &f).await;
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn the_limit_holds_even_when_the_declared_length_lies(pool: PgPool) {
    let f = fixture(&pool, 100).await;
    let big = vec![b'x'; 5_000];
    let liar = Upload {
        filename: None,
        content_type: "application/pdf".into(),
        declared_length: Some(10), // claims to be small
        body: &big[..],
    };
    let result = f.intake.upload(&f.writer, liar).await;
    assert!(
        matches!(result, Err(IntakeError::TooLarge { max_bytes: 100 })),
        "{result:?}"
    );
    nothing_was_stored(&pool, &f).await;

    let no_length = Upload {
        filename: None,
        content_type: "application/pdf".into(),
        declared_length: None,
        body: &big[..],
    };
    assert!(matches!(
        f.intake.upload(&f.writer, no_length).await,
        Err(IntakeError::TooLarge { .. })
    ));
    nothing_was_stored(&pool, &f).await;
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_file_exactly_at_the_limit_is_accepted(pool: PgPool) {
    let f = fixture(&pool, PDF.len() as u64).await;
    f.intake
        .upload(&f.writer, upload(PDF, "application/pdf", None))
        .await
        .unwrap();
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn an_empty_file_is_refused(pool: PgPool) {
    let f = fixture(&pool, 1_000_000).await;
    let result = f
        .intake
        .upload(&f.writer, upload(b"", "application/pdf", None))
        .await;
    assert!(matches!(result, Err(IntakeError::Empty)));
    nothing_was_stored(&pool, &f).await;
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_dropped_connection_is_an_interrupted_upload_and_stores_nothing(pool: PgPool) {
    let f = fixture(&pool, 1_000_000).await;
    let body = Upload {
        filename: None,
        content_type: "application/pdf".into(),
        declared_length: None,
        body: DropsMidway(false),
    };
    let result = f.intake.upload(&f.writer, body).await;
    assert!(matches!(result, Err(IntakeError::Interrupted)), "{result:?}");
    nothing_was_stored(&pool, &f).await;
}

// ---------- when the database fails after the file is stored ----------

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_database_failure_keeps_the_file_and_a_retry_reuses_it(pool: PgPool) {
    // The file is stored first, then the row is written. If the row cannot be
    // written the file stays (it is harmless, identified by its content and
    // owned by one tenant, and deleting it could race with an identical upload),
    // and a retry finds it already there.
    let f = fixture(&pool, 1_000_000).await;
    let broken = app_store(&pool).await;
    broken.pool().close().await;
    let failing = Intake::new(broken, LocalDisk::new(f.dir.path()), 1_000_000);

    let result = failing
        .upload(&f.writer, upload(PDF, "application/pdf", None))
        .await;
    assert!(matches!(result, Err(IntakeError::Database(_))), "{result:?}");
    assert_eq!(count(&pool, "select count(*) from documents").await, 0);
    assert_eq!(stored_files(&f.dir), 1, "the file is kept for the retry");

    let done = f
        .intake
        .upload(&f.writer, upload(PDF, "application/pdf", None))
        .await
        .unwrap();
    assert!(!done.duplicate);
    assert_eq!(stored_files(&f.dir), 1, "the retry reused it");
    assert_eq!(count(&pool, "select count(*) from documents").await, 1);
    let _ = &f.store;
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_storage_failure_is_reported_and_nothing_is_recorded(pool: PgPool) {
    let f = fixture(&pool, 1_000_000).await;
    // Point the object store at a path that is a file, so it cannot create folders.
    let blocker = f.dir.path().join("blocker");
    std::fs::write(&blocker, b"x").unwrap();
    let broken = Intake::new(f.store.clone(), LocalDisk::new(&blocker), 1_000_000);
    let result = broken
        .upload(&f.writer, upload(PDF, "application/pdf", None))
        .await;
    assert!(matches!(result, Err(IntakeError::Storage(_))), "{result:?}");
    assert_eq!(count(&pool, "select count(*) from documents").await, 0);
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn the_recorded_type_is_the_canonical_one_and_the_page_count_is_unknown(pool: PgPool) {
    let f = fixture(&pool, 1_000_000).await;
    let done = f
        .intake
        .upload(&f.writer, upload(PDF, "Application/PDF; charset=binary", None))
        .await
        .unwrap();
    let (content_type, pages): (String, Option<i32>) =
        sqlx::query_as("select content_type, page_count from documents where id = $1")
            .bind(done.document.0)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(content_type, "application/pdf");
    // Counting pages means opening the file, which is the extractor's job later.
    assert_eq!(pages, None);
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_new_document_is_queued_to_be_read_once(pool: PgPool) {
    let f = fixture(&pool, 1_000_000).await;
    let first = f
        .intake
        .upload(&f.writer, upload(PDF, "application/pdf", Some("a.pdf")))
        .await
        .unwrap();
    assert!(first.job.is_some());
    // The same file again while its job waits is the same job, not a second one.
    let second = f
        .intake
        .upload(&f.writer, upload(PDF, "application/pdf", Some("b.pdf")))
        .await
        .unwrap();
    assert!(second.duplicate);
    assert_eq!(second.job, first.job);
    assert_eq!(count(&pool, "select count(*) from jobs").await, 1);
    assert_eq!(
        count(&pool, "select count(*) from jobs where kind = 'process_document'").await,
        1
    );

    // Once the document has been worked on, a repeat upload leaves it alone.
    sqlx::query("update jobs set state = 'succeeded', finished_at = now()")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("update documents set status = 'done'")
        .execute(&pool)
        .await
        .unwrap();
    let third = f
        .intake
        .upload(&f.writer, upload(PDF, "application/pdf", Some("c.pdf")))
        .await
        .unwrap();
    assert!(third.duplicate);
    assert_eq!(third.job, None);
    assert_eq!(count(&pool, "select count(*) from jobs").await, 1);
}

// ---------- erasing ----------

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_document_is_erased_with_its_file_but_not_while_a_job_works_on_it(pool: PgPool) {
    let f = fixture(&pool, 1_000_000).await;
    let other = principal(
        Store::new(pool.clone()).create_tenant("Other").await.unwrap(),
        &[Scope::DocumentsWrite],
    );
    let done = f
        .intake
        .upload(&f.writer, upload(PDF, "application/pdf", Some("a.pdf")))
        .await
        .unwrap();
    assert_eq!(stored_files(&f.dir), 1);

    // Running: refused, nothing removed. Another tenant cannot see it exists.
    sqlx::query(
        "update jobs set state = 'running', started_at = now(), attempts = 1, lease_token = gen_random_uuid(),          leased_by = 'test', lease_expires_at = now() + interval '1 minute'",
    )
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        f.intake.erase(&f.writer, done.document).await,
        Err(IntakeError::Busy)
    ));
    assert_eq!(stored_files(&f.dir), 1);
    assert!(matches!(
        f.intake.erase(&other, done.document).await,
        Err(IntakeError::Database(clearcut_store::Error::DocumentNotFound))
    ));
    // A key without the scope is refused before anything is looked up.
    let reader = principal(f.tenant, &[Scope::DocumentsRead]);
    assert!(matches!(
        f.intake.erase(&reader, done.document).await,
        Err(IntakeError::Forbidden(_))
    ));

    // Merely waiting (queued): the job is cancelled and the document erased, so a document that
    // waits for a key can be deleted. The row, the job and the file go; the audit entry that
    // names it (and holds no content) stays.
    sqlx::query(
        "update jobs set state = 'queued', started_at = null, lease_token = null, leased_by = null,          lease_expires_at = null",
    )
        .execute(&pool)
        .await
        .unwrap();
    f.intake.erase(&f.writer, done.document).await.unwrap();
    assert_eq!(count(&pool, "select count(*) from documents").await, 0);
    assert_eq!(count(&pool, "select count(*) from jobs").await, 0);
    assert_eq!(stored_files(&f.dir), 0);
    assert_eq!(
        count(
            &pool,
            "select count(*) from audit_log where action = 'document.erased'"
        )
        .await,
        1
    );
    // Gone is gone.
    assert!(matches!(
        f.intake.erase(&f.writer, done.document).await,
        Err(IntakeError::Database(clearcut_store::Error::DocumentNotFound))
    ));
}
