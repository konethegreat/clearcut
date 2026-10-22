// Written by Kone Tshivhinda & Claude
//! The contract test: this client against the **real** Python worker.
//!
//! The unit tests use a stub that answers however the test says; this one proves
//! the two sides still understand each other. It is `#[ignore]`d so that
//! `cargo test --workspace` stays self-contained, and CI runs it explicitly after
//! starting the worker (the `contract` job):
//!
//! ```text
//! CLEARCUT_WORKER_TOKEN=<a long secret> CLEARCUT_WORKER_MAX_PAGES=2 CLEARCUT_WORKER_MAX_BYTES=2000000 \
//!     PYTHONPATH=src python -m clearcut_worker              # from services/worker
//! CLEARCUT_LIVE_WORKER_URL=http://127.0.0.1:8090 CLEARCUT_LIVE_WORKER_TOKEN=<the same secret> \
//!     cargo test -p clearcut-docproc --test live -- --ignored
//! ```
//!
//! The worker must be started with a 2-page cap and a 2 000 000-byte size cap,
//! because two of these tests probe those limits.

use bytes::Bytes;
use clearcut_core::jobs::FailureKind;
use clearcut_docproc::{DocumentKind, DocumentProblem, DocumentProcessor, HttpWorker, ProcessError};

const MAX_PAGES: u32 = 2;
const MAX_BYTES: u64 = 2_000_000;

fn live_url() -> String {
    std::env::var("CLEARCUT_LIVE_WORKER_URL")
        .expect("set CLEARCUT_LIVE_WORKER_URL to the running worker, for example http://127.0.0.1:8090")
}

fn live_token() -> String {
    std::env::var("CLEARCUT_LIVE_WORKER_TOKEN").expect("set CLEARCUT_LIVE_WORKER_TOKEN to the worker's token")
}

fn worker() -> HttpWorker {
    HttpWorker::new(&live_url(), &live_token()).unwrap()
}

fn fixture(name: &str) -> Bytes {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    Bytes::from(std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}")))
}

async fn problem(document: Bytes) -> ProcessError {
    worker()
        .inspect(document)
        .await
        .expect_err("the worker should have refused this")
}

#[tokio::test]
#[ignore = "needs the real worker; run by the CI contract job"]
async fn a_pdf_with_some_text_is_inspected() {
    let found = worker().inspect(fixture("mixed_two_pages.pdf")).await.unwrap();
    assert_eq!(found.kind(), DocumentKind::Pdf);
    assert_eq!((found.pages(), found.pages_with_text()), (2, 1));
    assert!(found.has_text_layer());
}

#[tokio::test]
#[ignore = "needs the real worker; run by the CI contract job"]
async fn a_scan_has_pages_but_no_text_layer() {
    let found = worker().inspect(fixture("scan_two_pages.pdf")).await.unwrap();
    assert_eq!((found.pages(), found.pages_with_text()), (2, 0));
    assert!(!found.has_text_layer());
}

#[tokio::test]
#[ignore = "needs the real worker; run by the CI contract job"]
async fn each_way_a_document_can_be_refused_arrives_as_its_own_error() {
    assert_eq!(
        problem(fixture("three_pages.pdf")).await,
        ProcessError::Document(DocumentProblem::TooManyPages { max_pages: MAX_PAGES })
    );
    assert_eq!(
        problem(fixture("encrypted.pdf")).await,
        ProcessError::Document(DocumentProblem::Encrypted)
    );
    assert_eq!(
        problem(fixture("damaged.pdf")).await,
        ProcessError::Document(DocumentProblem::Unreadable)
    );
    assert_eq!(
        problem(Bytes::from_static(b"<html>not a pdf</html>")).await,
        ProcessError::Document(DocumentProblem::UnsupportedType)
    );
    assert_eq!(
        problem(Bytes::new()).await,
        ProcessError::Document(DocumentProblem::Empty)
    );
}

#[tokio::test]
#[ignore = "needs the real worker; run by the CI contract job"]
async fn a_document_over_the_size_cap_is_refused_with_the_cap() {
    let mut huge = b"%PDF-".to_vec();
    huge.resize(3_000_000, b'x');
    assert_eq!(
        problem(Bytes::from(huge)).await,
        ProcessError::Document(DocumentProblem::TooLarge { max_bytes: MAX_BYTES })
    );
}

#[tokio::test]
#[ignore = "needs the real worker; run by the CI contract job"]
async fn the_wrong_token_is_refused_and_is_the_platforms_problem_not_the_documents() {
    let wrong = HttpWorker::new(&live_url(), &"w".repeat(40)).unwrap();
    let error = wrong.inspect(fixture("mixed_two_pages.pdf")).await.unwrap_err();
    assert_eq!(error, ProcessError::Auth);
    assert_eq!(error.failure_kind(), FailureKind::ProviderUnavailable);
}

#[tokio::test]
#[ignore = "needs the real worker; run by the CI contract job"]
async fn what_the_queue_is_told_matches_what_happened() {
    let refused = problem(fixture("encrypted.pdf")).await;
    assert_eq!(refused.failure_kind(), FailureKind::NeedsReview);
    assert!(refused.is_about_the_document());
}

#[tokio::test]
#[ignore = "needs the real worker; run by the CI contract job"]
async fn several_inspections_at_once_all_get_their_own_answer() {
    let one = fixture("scan_two_pages.pdf");
    let two = fixture("mixed_two_pages.pdf");
    let w = worker();
    let (a, b, c, d) = tokio::join!(
        w.inspect(one.clone()),
        w.inspect(two.clone()),
        w.inspect(one),
        w.inspect(two),
    );
    assert_eq!(a.unwrap().pages_with_text(), 0);
    assert_eq!(b.unwrap().pages_with_text(), 1);
    assert_eq!(c.unwrap().pages_with_text(), 0);
    assert_eq!(d.unwrap().pages_with_text(), 1);
}
