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

use base64::Engine as _;
use bytes::Bytes;
use clearcut_core::jobs::FailureKind;
use clearcut_docproc::{
    DocumentKind, DocumentProblem, DocumentProcessor, HttpWorker, ImageFormat, MaskBox, PageRange,
    PageSource, ProcessError, RenderOptions,
};
use clearcut_provider::ImageType;
use std::num::NonZeroU32;

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

// ---------- the newer calls (ADR-0016) ----------

/// A real 1 by 1 pixel PNG, so the picture calls have a picture to work on.
fn tiny_png() -> Bytes {
    let encoded =
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg==";
    Bytes::from(base64::engine::general_purpose::STANDARD.decode(encoded).unwrap())
}

fn n(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).unwrap()
}

#[tokio::test]
#[ignore = "needs the real worker; run by the CI contract job"]
async fn a_picture_is_a_one_page_document() {
    let found = worker().inspect(tiny_png()).await.unwrap();
    assert_eq!(
        (found.kind(), found.pages(), found.has_text_layer()),
        (DocumentKind::Png, 1, false)
    );
}

#[tokio::test]
#[ignore = "needs the real worker; run by the CI contract job"]
async fn the_text_of_pages_comes_back_checked_with_words_and_a_page_without_text_says_so() {
    let range = PageRange::new(n(1), Some(n(2))).unwrap();
    let pages = worker()
        .page_texts(fixture("mixed_two_pages.pdf"), range, true)
        .await
        .unwrap();
    assert_eq!(pages.len(), 2);
    assert_eq!(
        pages
            .iter()
            .filter(|p| p.source() == PageSource::TextLayer)
            .count(),
        1
    );
    assert_eq!(pages.iter().filter(|p| p.source() == PageSource::None).count(), 1);
    for page in &pages {
        assert!(page.words().is_some());
        assert!(page.width_pt() > 0.0 && page.height_pt() > 0.0);
        if page.source() == PageSource::TextLayer {
            assert!(!page.words().unwrap().is_empty());
            assert!(!page.fonts().is_empty());
        }
    }
    // Without words, none are sent.
    let plain = worker()
        .page_texts(fixture("mixed_two_pages.pdf"), PageRange::single(n(1)), false)
        .await
        .unwrap();
    assert!(plain[0].words().is_none());
}

#[tokio::test]
#[ignore = "needs the real worker; run by the CI contract job"]
async fn asking_for_pages_past_the_end_says_how_many_there_are() {
    let range = PageRange::new(n(2), Some(n(5))).unwrap();
    let error = worker()
        .page_texts(fixture("mixed_two_pages.pdf"), range, false)
        .await
        .unwrap_err();
    assert_eq!(
        error,
        ProcessError::Document(DocumentProblem::PageOutOfRange { pages: 2 })
    );
}

#[tokio::test]
#[ignore = "needs the real worker; run by the CI contract job"]
async fn a_page_is_drawn_and_then_painted_over() {
    let drawn = worker()
        .render_page(fixture("mixed_two_pages.pdf"), n(1), RenderOptions::default())
        .await
        .unwrap();
    assert_eq!(drawn.media(), ImageType::Png);
    assert!(drawn.width() > 0 && drawn.height() > 0);
    assert!(drawn.bytes().starts_with(b"\x89PNG"));

    let boxes = vec![MaskBox::new(100, 100, 300, 100).unwrap()];
    let painted = worker()
        .mask_image(drawn.bytes().clone(), boxes, ImageFormat::Png, 2000)
        .await
        .unwrap();
    assert_eq!(
        (painted.width(), painted.height()),
        (drawn.width(), drawn.height())
    );
    assert!(painted.bytes().starts_with(b"\x89PNG"));
}

#[tokio::test]
#[ignore = "needs the real worker; run by the CI contract job"]
async fn a_picture_can_be_drawn_and_painted_as_a_document() {
    let drawn = worker()
        .render_page(tiny_png(), n(1), RenderOptions::default())
        .await
        .unwrap();
    assert_eq!((drawn.width(), drawn.height()), (1, 1));
    let boxes = vec![MaskBox::new(0, 0, 1000, 1000).unwrap()];
    assert!(worker()
        .mask_image(tiny_png(), boxes, ImageFormat::Jpeg, 2000)
        .await
        .is_ok());
}

#[tokio::test]
#[ignore = "needs the real worker; run by the CI contract job"]
async fn drawing_a_page_that_is_not_there_or_a_file_that_is_not_supported_is_refused() {
    let past = worker()
        .render_page(fixture("mixed_two_pages.pdf"), n(3), RenderOptions::default())
        .await
        .unwrap_err();
    assert_eq!(
        past,
        ProcessError::Document(DocumentProblem::PageOutOfRange { pages: 2 })
    );
    let html = Bytes::from_static(b"<html>not a picture</html>");
    let masked = worker()
        .mask_image(html, Vec::new(), ImageFormat::Png, 2000)
        .await
        .unwrap_err();
    assert_eq!(masked, ProcessError::Document(DocumentProblem::UnsupportedType));
}

#[tokio::test]
#[ignore = "needs the real worker; run by the CI contract job"]
async fn ocr_is_read_when_the_worker_has_it_and_is_not_available_when_it_does_not() {
    // The contract job has no Tesseract, so the worker answers 503; a worker that has it
    // gives a page read by OCR. Either is understood.
    match worker().ocr_page(fixture("scan_two_pages.pdf"), n(1), true).await {
        Ok(page) => {
            assert_eq!(page.source(), PageSource::Ocr);
            assert!(page.words().is_some());
        }
        Err(error) => {
            assert_eq!(error, ProcessError::NotAvailable);
            assert_eq!(error.failure_kind(), FailureKind::ProviderUnavailable);
        }
    }
}
