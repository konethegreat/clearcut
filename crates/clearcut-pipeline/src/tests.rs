// Written by Kone Tshivhinda & Claude
//! The analysis stage and the job handler against a real Postgres, as the restricted
//! `clearcut_app` role the service runs as, a real object store on disk and a scripted
//! `FakeWorker`. `pool` (from `sqlx::test`) is the owner connection: tests use it to look
//! from outside and to move time.
//!
//! No test needs the real worker, a model or a key; none holds a real invoice.

use super::*;
use bytes::Bytes;
use clearcut_core::jobs::{JobKind, JobState, LeasePolicy, Progress};
use clearcut_core::page_read::{OcrRun, PageEvidence, PageRead, PageTextSource, ReadRoute, ROUTING_VERSION};
use clearcut_core::text_quality::{FontEncoding, FontFact, FontKind, Reason, TextVerdict};
use clearcut_docproc::{
    DocumentKind, DocumentProblem, DocumentProcessor, FakeWorker, ImageFormat, Inspection, MaskBox,
    PageContent, PageParts, PageRange, PageSource, PageWord, ProcessError, RenderOptions, RenderedPage,
    WordBox,
};
use clearcut_jobs::{Ended, RunReport, Runner, RunnerConfig};
use clearcut_objects::{LocalDisk, ObjectKey, ObjectStore, PutLimits, Sha256Digest};
use clearcut_store::{DocId, DocumentStatus, NewDocument, NewJob, Store, TenantId};
use serde_json::json;
use sqlx::PgPool;
use std::future::Future;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

// ---------- pages the fake worker answers with ----------

const GOOD: &str = "TAX INVOICE\nInvoice No: INV-2024-001\nDate: 2024-03-01\n\
                    Description Quantity Price Amount\nTotal R1,150.00 VAT R150.00";
const BROKEN: &str = "Invoice \u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}";
/// No invoice words and many words without a vowel: weak text.
const WEAK: &str = "xqz brtn plmk vnrt zdfg hjkl";

fn n(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).unwrap()
}

fn plain_font() -> FontFact {
    FontFact {
        kind: FontKind::Type1,
        has_to_unicode: false,
        encoding: FontEncoding::WinAnsi,
        embedded: true,
    }
}

fn type3_font() -> FontFact {
    FontFact {
        kind: FontKind::Type3,
        has_to_unicode: false,
        encoding: FontEncoding::None,
        embedded: true,
    }
}

fn page(
    page: u32,
    source: PageSource,
    text: &str,
    fonts: Vec<FontFact>,
    agreement: Option<u16>,
) -> PageContent {
    let words = (source == PageSource::Ocr).then(|| {
        vec![
            PageWord {
                start: 0,
                end: 5,
                bounds: WordBox {
                    x: 10,
                    y: 20,
                    w: 100,
                    h: 30,
                },
                confidence: Some(900),
            },
            PageWord {
                start: 6,
                end: 9,
                bounds: WordBox {
                    x: 120,
                    y: 20,
                    w: 100,
                    h: 30,
                },
                confidence: Some(900),
            },
        ]
    });
    PageContent::from_parts(PageParts {
        page: n(page),
        source,
        text: text.to_string(),
        width_pt: 595.0,
        height_pt: 842.0,
        fonts,
        words,
        agreement,
    })
    .unwrap()
}

fn text_page(number: u32, text: &str) -> PageContent {
    page(number, PageSource::TextLayer, text, vec![plain_font()], None)
}

fn weak_page(number: u32) -> PageContent {
    page(number, PageSource::TextLayer, WEAK, vec![type3_font()], None)
}

fn scan_page(number: u32) -> PageContent {
    page(number, PageSource::None, "", Vec::new(), None)
}

fn ocr_read(number: u32, agreement: Option<u16>) -> PageContent {
    page(number, PageSource::Ocr, "Total 100", Vec::new(), agreement)
}

fn pdf(pages: u32, with_text: u32) -> Inspection {
    Inspection::new(DocumentKind::Pdf, pages, with_text).unwrap()
}

fn good_pages(count: u32) -> Vec<PageContent> {
    (1..=count).map(|p| text_page(p, GOOD)).collect()
}

// ---------- the world ----------

fn actor() -> serde_json::Value {
    json!({ "type": "system" })
}

fn fast() -> RunnerConfig {
    RunnerConfig::new("test-worker")
        .unwrap()
        .with_heartbeat_every(Duration::from_millis(50))
        .unwrap()
        .with_poll_interval(Duration::from_millis(20))
        .unwrap()
}

struct World {
    pool: PgPool,
    store: Store,
    tenant: TenantId,
    objects: LocalDisk,
    _dir: tempfile::TempDir,
}

async fn world(pool: &PgPool) -> World {
    let tenant = Store::new(pool.clone())
        .create_tenant("Acme Advisory")
        .await
        .unwrap();
    let options = pool.connect_options().as_ref().clone().username("clearcut_app");
    let app_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(8)
        .connect_with(options)
        .await
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    World {
        pool: pool.clone(),
        store: Store::new(app_pool),
        tenant,
        objects: LocalDisk::new(dir.path()),
        _dir: dir,
    }
}

impl World {
    /// Stores `bytes` as a document's file and records the document.
    async fn document(&self, bytes: &[u8]) -> DocId {
        let stored = self
            .objects
            .put(
                self.tenant.0,
                bytes,
                PutLimits {
                    max_bytes: 10_000_000,
                },
            )
            .await
            .unwrap();
        self.record(NewDocument {
            original_filename: "invoice.pdf".into(),
            content_type: "application/pdf".into(),
            byte_size: i64::try_from(stored.byte_size).unwrap(),
            sha256: *stored.sha256.as_bytes(),
            storage_key: stored.key.as_str().to_string(),
            page_count: None,
        })
        .await
    }

    async fn record(&self, document: NewDocument) -> DocId {
        self.store.add_document(self.tenant, &document).await.unwrap().id
    }

    async fn enqueue(&self, doc: DocId) -> Uuid {
        self.store
            .enqueue_job(self.tenant, &NewJob::new(doc, JobKind::ProcessDocument), &actor())
            .await
            .unwrap()
            .id
    }

    fn handler<P: DocumentProcessor>(
        &self,
        worker: &Arc<P>,
        config: PipelineConfig,
    ) -> PipelineHandler<LocalDisk, P> {
        PipelineHandler::new(self.store.clone(), self.objects.clone(), worker.clone(), config)
    }

    async fn run<P: DocumentProcessor, M: clearcut_provider::ModelProvider>(
        &self,
        handler: PipelineHandler<LocalDisk, P, M>,
    ) -> RunReport {
        Runner::new(self.store.clone(), handler, fast())
            .run_once()
            .await
            .unwrap()
            .expect("a job was due")
    }

    /// A document with a queued job, run once with `worker` and the default settings.
    async fn process(&self, bytes: &[u8], worker: &Arc<FakeWorker>) -> (DocId, Uuid, RunReport) {
        self.process_with(bytes, worker, PipelineConfig::default()).await
    }

    async fn process_with(
        &self,
        bytes: &[u8],
        worker: &Arc<FakeWorker>,
        config: PipelineConfig,
    ) -> (DocId, Uuid, RunReport) {
        let doc = self.document(bytes).await;
        let job = self.enqueue(doc).await;
        let report = self.run(self.handler(worker, config)).await;
        (doc, job, report)
    }

    async fn reads(&self, doc: DocId) -> Vec<PageRead> {
        self.store.load_page_reads(self.tenant, doc).await.unwrap()
    }

    async fn job(&self, job: Uuid) -> clearcut_store::Job {
        self.store.get_job(self.tenant, job).await.unwrap()
    }

    async fn status(&self, doc: DocId) -> DocumentStatus {
        self.store.document_status(self.tenant, doc).await.unwrap()
    }

    async fn page_count(&self, doc: DocId) -> Option<i32> {
        sqlx::query_scalar("select page_count from documents where id = $1")
            .bind(doc.0)
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }
}

fn routes(reads: &[PageRead]) -> Vec<ReadRoute> {
    reads.iter().map(PageRead::route).collect()
}

fn range(first: u32, last: u32) -> PageRange {
    PageRange::new(n(first), Some(n(last))).unwrap()
}

const FILE: &[u8] = b"%PDF-1.7 a stand-in for a file; the fake worker never parses it";

mod read;

// ---------- the normal case ----------

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_clean_text_pdf_is_read_with_its_pictures_and_only_page_one_goes_to_ocr(pool: PgPool) {
    let w = world(&pool).await;
    let fake = Arc::new(
        FakeWorker::scripted(vec![Ok(pdf(3, 3))])
            .with_texts(vec![Ok(good_pages(3))])
            .with_ocr(vec![Ok(ocr_read(1, Some(1000)))]),
    );
    let (doc, job, report) = w.process(FILE, &fake).await;

    // The worker was asked the right things, once each, with the stored bytes.
    assert_eq!(fake.documents(), vec![Bytes::from_static(FILE)]);
    let texts = fake.text_calls();
    assert_eq!(texts.len(), 1);
    assert_eq!((texts[0].range, texts[0].words), (range(1, 3), false));
    let ocr = fake.ocr_calls();
    assert_eq!(ocr.len(), 1);
    assert_eq!((ocr[0].page, ocr[0].agreement), (n(1), true));
    assert_eq!(ocr[0].document, Bytes::from_static(FILE));

    // Every page is read with its picture and its text; page one carries the OCR agreement.
    let reads = w.reads(doc).await;
    assert_eq!(reads.len(), 3);
    assert_eq!(routes(&reads), [ReadRoute::TextAndImage; 3]);
    assert_eq!(reads[0].ocr_agreement(), Some(1000));
    assert_eq!(reads[0].reasons(), &[Reason::OcrAgrees]);
    assert_eq!(reads[1].ocr_agreement(), None);
    assert!(reads.iter().all(|r| r.text_source() == PageTextSource::TextLayer));
    assert!(reads.iter().all(|r| r.analysed_with() == ROUTING_VERSION));
    assert_eq!(w.page_count(doc).await, Some(3));

    // The job ends for review, honestly: nothing after the analysis exists yet.
    assert_eq!(report.ended, Ended::NeedsReview);
    let job = w.job(job).await;
    assert_eq!(job.state, JobState::NeedsReview);
    assert_eq!(job.last_error.as_deref(), Some(AFTER_ANALYSIS));
    assert_eq!(w.status(doc).await, DocumentStatus::NeedsReview);
    assert_eq!(job.progress, Progress::new(3, Some(3)).unwrap());
    assert_eq!(
        job.checkpoint,
        Some(json!({ "stage": CHECKPOINT_STAGE, "pages": 3, "rules": ROUTING_VERSION }))
    );
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_page_whose_text_is_broken_is_read_by_ocr_and_its_layer_is_rejected(pool: PgPool) {
    let w = world(&pool).await;
    let fake = Arc::new(
        FakeWorker::scripted(vec![Ok(pdf(3, 3))])
            .with_texts(vec![Ok(vec![
                text_page(1, GOOD),
                text_page(2, BROKEN),
                text_page(3, GOOD),
            ])])
            .with_ocr(vec![Ok(ocr_read(1, Some(1000))), Ok(ocr_read(2, Some(100)))]),
    );
    let (doc, _, _) = w.process(FILE, &fake).await;

    let pages: Vec<u32> = fake.ocr_calls().iter().map(|c| c.page.get()).collect();
    assert_eq!(pages, [1, 2]);
    let reads = w.reads(doc).await;
    assert_eq!(
        routes(&reads),
        [
            ReadRoute::TextAndImage,
            ReadRoute::ImageOnly,
            ReadRoute::TextAndImage
        ]
    );
    assert_eq!(reads[1].verdict(), TextVerdict::Unusable);
    assert_eq!(reads[1].text_source(), PageTextSource::Ocr);
    assert!(reads[1].text_layer_rejected());
    // The third page was never compared with OCR and is taken as it is.
    assert_eq!(reads[2].verdict(), TextVerdict::Good);
    assert_eq!(reads[2].ocr_agreement(), None);
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_first_page_that_disagrees_sends_the_rest_of_the_document_to_ocr(pool: PgPool) {
    let w = world(&pool).await;
    let fake = Arc::new(
        FakeWorker::scripted(vec![Ok(pdf(3, 3))])
            .with_texts(vec![Ok(good_pages(3))])
            .with_ocr(vec![
                Ok(ocr_read(1, Some(300))),
                Ok(ocr_read(2, Some(1000))),
                Ok(ocr_read(3, Some(1000))),
            ]),
    );
    let (doc, _, _) = w.process(FILE, &fake).await;

    let pages: Vec<u32> = fake.ocr_calls().iter().map(|c| c.page.get()).collect();
    assert_eq!(pages, [1, 2, 3]);
    let reads = w.reads(doc).await;
    assert_eq!(
        routes(&reads),
        [
            ReadRoute::ImageOnly,
            ReadRoute::TextAndImage,
            ReadRoute::TextAndImage
        ]
    );
    assert_eq!(reads[0].reasons(), &[Reason::OcrDisagrees]);
    assert_eq!(reads[2].ocr_agreement(), Some(1000));
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_scan_is_read_by_ocr_page_by_page_with_nothing_to_compare(pool: PgPool) {
    let w = world(&pool).await;
    let fake = Arc::new(
        FakeWorker::scripted(vec![Ok(pdf(3, 0))])
            .with_texts(vec![Ok((1..=3).map(scan_page).collect())])
            .with_ocr((1..=3).map(|p| Ok(ocr_read(p, None))).collect()),
    );
    let (doc, _, _) = w.process(FILE, &fake).await;

    let calls = fake.ocr_calls();
    assert_eq!(calls.iter().map(|c| c.page.get()).collect::<Vec<_>>(), [1, 2, 3]);
    // No text layer, so no agreement is asked for.
    assert!(calls.iter().all(|c| !c.agreement));
    let reads = w.reads(doc).await;
    assert_eq!(routes(&reads), [ReadRoute::ImageOnly; 3]);
    assert!(reads.iter().all(|r| r.text_source() == PageTextSource::Ocr));
    assert!(reads.iter().all(|r| r.reasons() == [Reason::NoText]));
    assert!(reads.iter().all(|r| !r.text_layer_rejected()));
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_picture_is_a_one_page_scan(pool: PgPool) {
    let w = world(&pool).await;
    let fake = Arc::new(
        FakeWorker::scripted(vec![Ok(Inspection::new(DocumentKind::Png, 1, 0).unwrap())])
            .with_texts(vec![Ok(vec![scan_page(1)])])
            .with_ocr(vec![Ok(ocr_read(1, None))]),
    );
    let (doc, _, _) = w.process(FILE, &fake).await;
    let reads = w.reads(doc).await;
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0].route(), ReadRoute::ImageOnly);
    assert_eq!(w.page_count(doc).await, Some(1));
}

// ---------- no OCR on this worker ----------

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_worker_without_ocr_is_not_a_failure_and_is_asked_only_once(pool: PgPool) {
    let w = world(&pool).await;
    let fake = Arc::new(
        FakeWorker::scripted(vec![Ok(pdf(3, 2))])
            .with_texts(vec![Ok(vec![text_page(1, GOOD), weak_page(2), scan_page(3)])])
            .with_ocr(vec![Err(ProcessError::NotAvailable)]),
    );
    let (doc, job, report) = w.process(FILE, &fake).await;

    assert_eq!(fake.ocr_calls().len(), 1, "it is not asked again");
    let reads = w.reads(doc).await;
    assert_eq!(
        routes(&reads),
        [
            ReadRoute::TextAndImage,
            ReadRoute::ImageFirst,
            ReadRoute::ImageOnly
        ]
    );
    assert_eq!(reads[0].ocr_agreement(), None);
    assert_eq!(reads[1].text_source(), PageTextSource::TextLayer);
    assert_eq!(reads[2].text_source(), PageTextSource::None);
    assert_eq!(report.ended, Ended::NeedsReview);
    assert_eq!(w.job(job).await.last_error.as_deref(), Some(AFTER_ANALYSIS));

    let detail = analysis_detail(&w, doc).await;
    assert_eq!(detail["ocr"], "unavailable");
    assert_eq!(detail["ocr_pages"], 0);
}

// ---------- long documents ----------

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn text_is_read_fifty_pages_at_a_time(pool: PgPool) {
    let w = world(&pool).await;
    let chunk = |from: u32, to: u32| Ok((from..=to).map(|p| text_page(p, GOOD)).collect());
    let fake = Arc::new(
        FakeWorker::scripted(vec![Ok(pdf(120, 120))])
            .with_texts(vec![chunk(1, 50), chunk(51, 100), chunk(101, 120)])
            .with_ocr(vec![Ok(ocr_read(1, Some(1000)))]),
    );
    let (doc, _, _) = w.process(FILE, &fake).await;

    let ranges: Vec<PageRange> = fake.text_calls().iter().map(|c| c.range).collect();
    assert_eq!(ranges, [range(1, 50), range(51, 100), range(101, 120)]);
    let reads = w.reads(doc).await;
    assert_eq!(reads.len(), 120);
    assert_eq!(reads[119].page(), 120);
    assert_eq!(w.page_count(doc).await, Some(120));
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn ocr_never_reads_more_pages_than_the_cap(pool: PgPool) {
    let w = world(&pool).await;
    let chunk = |from: u32, to: u32| Ok((from..=to).map(scan_page).collect());
    let fake = Arc::new(
        FakeWorker::scripted(vec![Ok(pdf(120, 0))])
            .with_texts(vec![chunk(1, 50), chunk(51, 100), chunk(101, 120)])
            .with_ocr((1..=50).map(|p| Ok(ocr_read(p, None))).collect()),
    );
    let (doc, _, _) = w.process(FILE, &fake).await;

    assert_eq!(fake.ocr_calls().len(), 50);
    let reads = w.reads(doc).await;
    assert_eq!(reads.len(), 120);
    assert!(reads[..50].iter().all(|r| r.text_source() == PageTextSource::Ocr));
    assert!(reads[50..]
        .iter()
        .all(|r| r.text_source() == PageTextSource::None));
    assert!(reads.iter().all(|r| r.route() == ReadRoute::ImageOnly));
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn the_cap_is_a_setting(pool: PgPool) {
    let w = world(&pool).await;
    let fake = Arc::new(
        FakeWorker::scripted(vec![Ok(pdf(5, 0))])
            .with_texts(vec![Ok((1..=5).map(scan_page).collect())])
            .with_ocr((1..=2).map(|p| Ok(ocr_read(p, None))).collect()),
    );
    let (doc, _, _) = w
        .process_with(FILE, &fake, PipelineConfig::default().with_ocr_page_cap(2))
        .await;
    assert_eq!(fake.ocr_calls().len(), 2);
    assert_eq!(w.reads(doc).await.len(), 5);
}

// ---------- what goes wrong, and what the queue does about it ----------

async fn nothing_was_written(w: &World, doc: DocId) {
    assert_eq!(w.reads(doc).await, Vec::new());
    assert_eq!(w.page_count(doc).await, None);
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_worker_that_times_out_is_retried_and_counts_against_the_attempts(pool: PgPool) {
    let w = world(&pool).await;
    let fake = Arc::new(FakeWorker::scripted(vec![Err(ProcessError::Timeout)]));
    let (doc, job, report) = w.process(FILE, &fake).await;
    assert!(
        matches!(report.ended, Ended::Retrying { counts: true, .. }),
        "{:?}",
        report.ended
    );
    assert_eq!(w.job(job).await.state, JobState::Queued);
    nothing_was_written(&w, doc).await;
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_worker_that_is_down_is_retried_without_using_up_an_attempt(pool: PgPool) {
    let w = world(&pool).await;
    let fake = Arc::new(FakeWorker::scripted(vec![Err(ProcessError::Unavailable)]));
    let (doc, _, report) = w.process(FILE, &fake).await;
    assert!(
        matches!(report.ended, Ended::Retrying { counts: false, .. }),
        "{:?}",
        report.ended
    );
    nothing_was_written(&w, doc).await;
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_document_the_worker_refuses_goes_to_review_with_the_reason(pool: PgPool) {
    let w = world(&pool).await;
    let refusal = ProcessError::Document(DocumentProblem::Encrypted);
    let fake = Arc::new(FakeWorker::scripted(vec![Err(refusal)]));
    let (doc, job, report) = w.process(FILE, &fake).await;
    assert_eq!(report.ended, Ended::NeedsReview);
    let job = w.job(job).await;
    assert_eq!(job.state, JobState::NeedsReview);
    assert_eq!(job.last_error.as_deref(), Some(refusal.to_string().as_str()));
    assert_eq!(w.status(doc).await, DocumentStatus::NeedsReview);
    nothing_was_written(&w, doc).await;
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_document_with_no_pages_goes_to_review(pool: PgPool) {
    let w = world(&pool).await;
    let fake = Arc::new(FakeWorker::scripted(vec![Ok(pdf(0, 0))]));
    let (doc, job, report) = w.process(FILE, &fake).await;
    assert_eq!(report.ended, Ended::NeedsReview);
    assert_eq!(
        w.job(job).await.last_error.as_deref(),
        Some("the document has no pages")
    );
    nothing_was_written(&w, doc).await;
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_failure_half_way_through_the_text_leaves_nothing_behind(pool: PgPool) {
    let w = world(&pool).await;
    let first: Vec<PageContent> = good_pages(50);
    let fake = Arc::new(
        FakeWorker::scripted(vec![Ok(pdf(120, 120))]).with_texts(vec![Ok(first), Err(ProcessError::Timeout)]),
    );
    let (doc, _, report) = w.process(FILE, &fake).await;
    assert!(
        matches!(report.ended, Ended::Retrying { counts: true, .. }),
        "{:?}",
        report.ended
    );
    nothing_was_written(&w, doc).await;
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_failure_in_ocr_leaves_nothing_behind(pool: PgPool) {
    let w = world(&pool).await;
    let fake = Arc::new(
        FakeWorker::scripted(vec![Ok(pdf(2, 2))])
            .with_texts(vec![Ok(good_pages(2))])
            .with_ocr(vec![Err(ProcessError::Timeout)]),
    );
    let (doc, _, report) = w.process(FILE, &fake).await;
    assert!(
        matches!(report.ended, Ended::Retrying { counts: true, .. }),
        "{:?}",
        report.ended
    );
    nothing_was_written(&w, doc).await;
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_worker_that_answers_with_the_wrong_pages_is_a_transient_failure(pool: PgPool) {
    let w = world(&pool).await;
    // Pages 2 and 3 were sent for a request for 1 to 3; and too few.
    for (i, answer) in [
        vec![text_page(2, GOOD), text_page(3, GOOD), text_page(4, GOOD)],
        good_pages(2),
    ]
    .into_iter()
    .enumerate()
    {
        let fake = Arc::new(FakeWorker::scripted(vec![Ok(pdf(3, 3))]).with_texts(vec![Ok(answer)]));
        let bytes = format!("file number {i}");
        let (doc, _, report) = w.process(bytes.as_bytes(), &fake).await;
        assert!(
            matches!(report.ended, Ended::Retrying { counts: true, .. }),
            "{:?}",
            report.ended
        );
        nothing_was_written(&w, doc).await;
    }
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn an_ocr_answer_for_the_wrong_page_is_a_transient_failure(pool: PgPool) {
    let w = world(&pool).await;
    let fake = Arc::new(
        FakeWorker::scripted(vec![Ok(pdf(2, 2))])
            .with_texts(vec![Ok(good_pages(2))])
            .with_ocr(vec![Ok(ocr_read(2, Some(1000)))]),
    );
    let (doc, _, report) = w.process(FILE, &fake).await;
    assert!(
        matches!(report.ended, Ended::Retrying { counts: true, .. }),
        "{:?}",
        report.ended
    );
    nothing_was_written(&w, doc).await;
}

// ---------- the stored file ----------

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_file_that_is_missing_from_storage_goes_to_review_and_the_worker_is_not_asked(pool: PgPool) {
    let w = world(&pool).await;
    let digest = Sha256Digest::from_hex(&"ab".repeat(32)).unwrap();
    let doc = w
        .record(NewDocument {
            original_filename: "gone.pdf".into(),
            content_type: "application/pdf".into(),
            byte_size: 10,
            sha256: *digest.as_bytes(),
            storage_key: ObjectKey::for_document(w.tenant.0, &digest).as_str().to_string(),
            page_count: None,
        })
        .await;
    let job = w.enqueue(doc).await;
    let fake = Arc::new(FakeWorker::scripted(vec![]));
    let report = w.run(w.handler(&fake, PipelineConfig::default())).await;
    assert_eq!(report.ended, Ended::NeedsReview);
    assert_eq!(
        w.job(job).await.last_error.as_deref(),
        Some("the stored file is missing")
    );
    assert_eq!(fake.call_count(), 0);
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_file_that_does_not_match_its_record_goes_to_review(pool: PgPool) {
    let w = world(&pool).await;
    // For each case the file is stored properly and the record then disagrees with it.
    type Change = fn(&mut NewDocument);
    let cases: [(&str, Change); 3] = [
        ("another hash", |d| d.sha256 = [7; 32]),
        ("a larger size", |d| d.byte_size += 1),
        ("a smaller size", |d| d.byte_size -= 1),
    ];
    for (case, change) in cases {
        let bytes = format!("{case} {}", String::from_utf8_lossy(FILE));
        let stored = w
            .objects
            .put(
                w.tenant.0,
                bytes.as_bytes(),
                PutLimits {
                    max_bytes: 10_000_000,
                },
            )
            .await
            .unwrap();
        let mut record = NewDocument {
            original_filename: "x.pdf".into(),
            content_type: "application/pdf".into(),
            byte_size: i64::try_from(stored.byte_size).unwrap(),
            sha256: *stored.sha256.as_bytes(),
            storage_key: stored.key.as_str().to_string(),
            page_count: None,
        };
        change(&mut record);
        let doc = w.record(record).await;
        let job = w.enqueue(doc).await;
        let fake = Arc::new(FakeWorker::scripted(vec![]));
        let report = w.run(w.handler(&fake, PipelineConfig::default())).await;
        assert_eq!(report.ended, Ended::NeedsReview, "{case}");
        assert_eq!(
            w.job(job).await.last_error.as_deref(),
            Some("the stored file does not match its record"),
            "{case}"
        );
        assert_eq!(fake.call_count(), 0, "{case}");
    }
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_location_that_is_not_a_stored_key_goes_to_review(pool: PgPool) {
    let w = world(&pool).await;
    let doc = w
        .record(NewDocument {
            original_filename: "x.pdf".into(),
            content_type: "application/pdf".into(),
            byte_size: 10,
            sha256: [5; 32],
            storage_key: "../../etc/passwd".into(),
            page_count: None,
        })
        .await;
    let job = w.enqueue(doc).await;
    let fake = Arc::new(FakeWorker::scripted(vec![]));
    let report = w.run(w.handler(&fake, PipelineConfig::default())).await;
    assert_eq!(report.ended, Ended::NeedsReview);
    assert_eq!(
        w.job(job).await.last_error.as_deref(),
        Some("the stored file has no valid location")
    );
    assert_eq!(fake.call_count(), 0);
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_file_larger_than_the_pipeline_reads_goes_to_review(pool: PgPool) {
    let w = world(&pool).await;
    let fake = Arc::new(FakeWorker::scripted(vec![]));
    let (doc, job, report) = w
        .process_with(FILE, &fake, PipelineConfig::default().with_max_document_bytes(10))
        .await;
    assert_eq!(report.ended, Ended::NeedsReview);
    assert_eq!(
        w.job(job).await.last_error.as_deref(),
        Some("the file is larger than the pipeline reads")
    );
    assert_eq!(fake.call_count(), 0);
    nothing_was_written(&w, doc).await;
}

// ---------- cancelling ----------

/// A worker that asks for the job to be cancelled while it is inspecting the document.
struct CancelsDuringInspect {
    inner: Arc<FakeWorker>,
    store: Store,
    tenant: TenantId,
    job: Uuid,
}

impl DocumentProcessor for CancelsDuringInspect {
    fn inspect(&self, document: Bytes) -> impl Future<Output = Result<Inspection, ProcessError>> + Send {
        let (store, tenant, job, inner) = (self.store.clone(), self.tenant, self.job, self.inner.clone());
        async move {
            store.request_cancel(tenant, job, &actor()).await.unwrap();
            inner.inspect(document).await
        }
    }

    fn page_texts(
        &self,
        document: Bytes,
        range: PageRange,
        words: bool,
    ) -> impl Future<Output = Result<Vec<PageContent>, ProcessError>> + Send {
        self.inner.page_texts(document, range, words)
    }

    fn ocr_page(
        &self,
        document: Bytes,
        page: NonZeroU32,
        agreement: bool,
    ) -> impl Future<Output = Result<PageContent, ProcessError>> + Send {
        self.inner.ocr_page(document, page, agreement)
    }

    fn render_page(
        &self,
        document: Bytes,
        page: NonZeroU32,
        options: RenderOptions,
    ) -> impl Future<Output = Result<RenderedPage, ProcessError>> + Send {
        self.inner.render_page(document, page, options)
    }

    fn mask_image(
        &self,
        image: Bytes,
        boxes: Vec<MaskBox>,
        format: ImageFormat,
        max_edge: u16,
    ) -> impl Future<Output = Result<RenderedPage, ProcessError>> + Send {
        self.inner.mask_image(image, boxes, format, max_edge)
    }
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_cancelled_job_stops_before_anything_is_written(pool: PgPool) {
    let w = world(&pool).await;
    let doc = w.document(FILE).await;
    let job = w.enqueue(doc).await;
    let inner = Arc::new(FakeWorker::scripted(vec![Ok(pdf(3, 3))]).with_texts(vec![Ok(good_pages(3))]));
    let cancelling = Arc::new(CancelsDuringInspect {
        inner: inner.clone(),
        store: w.store.clone(),
        tenant: w.tenant,
        job,
    });
    let report = w.run(w.handler(&cancelling, PipelineConfig::default())).await;

    assert_eq!(report.ended, Ended::Cancelled);
    assert_eq!(w.job(job).await.state, JobState::Cancelled);
    assert!(
        inner.text_calls().is_empty(),
        "it stopped before reading the text"
    );
    nothing_was_written(&w, doc).await;
}

// ---------- resuming ----------

/// Claims the job as an earlier worker that saved `checkpoint` and then stopped heart-beating.
async fn abandoned_with(w: &World, job: Uuid, checkpoint: serde_json::Value) {
    let claimed = w
        .store
        .claim_job("earlier-worker", LeasePolicy::default(), &JobKind::ALL, None)
        .await
        .unwrap()
        .expect("the job is due");
    assert_eq!(claimed.job.id, job);
    w.store
        .heartbeat_job(
            &claimed,
            LeasePolicy::default(),
            Progress::new(3, Some(3)).unwrap(),
            Some("analysed"),
            Some(&checkpoint),
        )
        .await
        .unwrap();
    sqlx::query("update jobs set lease_expires_at = now() - interval '1 second' where id = $1")
        .bind(job)
        .execute(&w.pool)
        .await
        .unwrap();
}

fn analysed_checkpoint(rules: &str) -> serde_json::Value {
    json!({ "stage": CHECKPOINT_STAGE, "pages": 3, "rules": rules })
}

fn three_reads() -> Vec<PageRead> {
    (1..=3)
        .map(|page| {
            PageRead::decide(&PageEvidence {
                page,
                text: GOOD,
                fonts: &[],
                ocr: Some(OcrRun {
                    agreement: Some(1000),
                }),
            })
        })
        .collect()
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_job_taken_over_after_the_analysis_does_not_do_it_again(pool: PgPool) {
    let w = world(&pool).await;
    let doc = w.document(FILE).await;
    let job = w.enqueue(doc).await;
    w.store
        .replace_page_reads(w.tenant, doc, &three_reads())
        .await
        .unwrap();
    abandoned_with(&w, job, analysed_checkpoint(ROUTING_VERSION)).await;

    let fake = Arc::new(FakeWorker::scripted(vec![]));
    let report = w.run(w.handler(&fake, PipelineConfig::default())).await;

    assert_eq!(fake.call_count(), 0, "the worker is not asked again");
    assert_eq!(report.ended, Ended::NeedsReview);
    assert_eq!(w.job(job).await.last_error.as_deref(), Some(AFTER_ANALYSIS));
    assert_eq!(w.reads(doc).await, three_reads());
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_checkpoint_from_other_rules_or_without_its_rows_is_not_trusted(pool: PgPool) {
    let w = world(&pool).await;
    for (case, rules, rows_present) in [
        ("old rules", "routing-v0", true),
        ("no rows", ROUTING_VERSION, false),
    ] {
        let bytes = format!("{case}{FILE:?}");
        let doc = w.document(bytes.as_bytes()).await;
        let job = w.enqueue(doc).await;
        if rows_present {
            w.store
                .replace_page_reads(w.tenant, doc, &three_reads())
                .await
                .unwrap();
        }
        abandoned_with(&w, job, analysed_checkpoint(rules)).await;

        let fake = Arc::new(
            FakeWorker::scripted(vec![Ok(pdf(3, 3))])
                .with_texts(vec![Ok(good_pages(3))])
                .with_ocr(vec![Ok(ocr_read(1, Some(1000)))]),
        );
        let report = w.run(w.handler(&fake, PipelineConfig::default())).await;
        assert_eq!(report.ended, Ended::NeedsReview, "{case}");
        assert_eq!(fake.documents().len(), 1, "{case}: the analysis was done again");
        assert_eq!(w.reads(doc).await.len(), 3, "{case}");
    }
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn running_the_stage_again_replaces_the_rows(pool: PgPool) {
    let w = world(&pool).await;
    let scripts = || {
        Arc::new(
            FakeWorker::scripted(vec![Ok(pdf(3, 3))])
                .with_texts(vec![Ok(good_pages(3))])
                .with_ocr(vec![Ok(ocr_read(1, Some(1000)))]),
        )
    };
    let (doc, first_job, _) = w.process(FILE, &scripts()).await;
    let before = w.reads(doc).await;
    // The same document is processed again (a person asked for it after the review).
    let second_job = w.enqueue(doc).await;
    assert_ne!(first_job, second_job);
    let report = w.run(w.handler(&scripts(), PipelineConfig::default())).await;
    assert_eq!(report.ended, Ended::NeedsReview);
    assert_eq!(w.reads(doc).await, before);
    let rows: i64 = sqlx::query_scalar("select count(*) from page_reads where document_id = $1")
        .bind(doc.0)
        .fetch_one(&w.pool)
        .await
        .unwrap();
    assert_eq!(rows, 3);
}

// ---------- what is recorded ----------

async fn analysis_detail(w: &World, doc: DocId) -> serde_json::Value {
    let rows: Vec<(serde_json::Value, serde_json::Value)> = sqlx::query_as(
        "select actor, detail from audit_log where action = 'document.analysed' and subject_id = $1",
    )
    .bind(doc.0.to_string())
    .fetch_all(&w.pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 1, "one audit entry");
    assert_eq!(rows[0].0, json!({ "type": "system" }));
    rows[0].1.clone()
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn the_analysis_is_audited_with_counts_and_no_content(pool: PgPool) {
    let w = world(&pool).await;
    let marker = "MARKER-INV-777";
    let secret = format!("{GOOD}\n{marker}");
    let fake = Arc::new(
        FakeWorker::scripted(vec![Ok(pdf(3, 3))])
            .with_texts(vec![Ok(vec![
                text_page(1, &secret),
                text_page(2, BROKEN),
                text_page(3, &secret),
            ])])
            .with_ocr(vec![Ok(ocr_read(1, Some(1000))), Ok(ocr_read(2, Some(0)))]),
    );
    let (doc, job, _) = w.process(FILE, &fake).await;

    let detail = analysis_detail(&w, doc).await;
    assert_eq!(
        detail,
        json!({
            "pages": 3,
            "ocr": "ran",
            "ocr_pages": 2,
            "routes": { "text_and_image": 2, "image_first": 0, "image_only": 1 },
            "text_layers_rejected": 1,
            "rules": ROUTING_VERSION,
        })
    );

    // Nothing the document said is anywhere that is logged or shown.
    let job = w.job(job).await;
    let everything = format!(
        "{detail} {} {:?} {:?} {:?}",
        job.last_error.unwrap_or_default(),
        job.checkpoint,
        job.progress_note,
        w.reads(doc).await
    );
    assert!(!everything.contains(marker), "{everything}");
    assert!(!everything.contains("Invoice No"), "{everything}");
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn progress_is_reported_as_the_pages_are_read(pool: PgPool) {
    let w = world(&pool).await;
    let chunk = |from: u32, to: u32| Ok((from..=to).map(|p| text_page(p, GOOD)).collect());
    let fake = Arc::new(
        FakeWorker::scripted(vec![Ok(pdf(70, 70))])
            .with_texts(vec![chunk(1, 50), chunk(51, 70)])
            .with_ocr(vec![Ok(ocr_read(1, Some(1000)))]),
    );
    let (_, job, _) = w.process(FILE, &fake).await;
    let job = w.job(job).await;
    assert_eq!(job.progress, Progress::new(70, Some(70)).unwrap());
    assert!(job.progress_note.is_some());
}

// ---------- settings and wording ----------

#[test]
fn the_defaults_are_fifty_ocr_pages_and_a_hundred_mebibytes() {
    let config = PipelineConfig::default();
    assert_eq!(config.ocr_page_cap(), 50);
    assert_eq!(config.max_document_bytes(), 100 * 1024 * 1024);
    assert_eq!(DEFAULT_MAX_DOCUMENT_BYTES, 100 * 1024 * 1024);
    assert_eq!(config.with_ocr_page_cap(7).ocr_page_cap(), 7);
    assert_eq!(config.with_max_document_bytes(9).max_document_bytes(), 9);
}

#[test]
fn the_reason_a_job_ends_for_review_says_that_nothing_was_read() {
    assert_eq!(
        AFTER_ANALYSIS,
        "analysed; no model provider was given, so nothing was read"
    );
    assert_eq!(CHECKPOINT_STAGE, "analysed");
}

fn worker_for(inspection: Inspection, pages: Vec<PageContent>) -> Arc<FakeWorker> {
    Arc::new(
        FakeWorker::scripted(vec![Ok(inspection)])
            .with_texts(vec![Ok(pages)])
            .with_ocr(vec![Ok(ocr_read(1, Some(1000)))]),
    )
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_file_exactly_as_large_as_the_limit_is_read(pool: PgPool) {
    let w = world(&pool).await;
    let size = FILE.len() as u64;
    let fake = worker_for(pdf(1, 1), vec![text_page(1, GOOD)]);
    let (doc, _, report) = w
        .process_with(
            FILE,
            &fake,
            PipelineConfig::default().with_max_document_bytes(size),
        )
        .await;
    assert_eq!(report.ended, Ended::NeedsReview);
    assert_eq!(w.reads(doc).await.len(), 1, "it was analysed");

    // One byte over the limit is not read.
    let other = b"another file";
    let fake = worker_for(pdf(1, 1), vec![text_page(1, GOOD)]);
    let limit = other.len() as u64 - 1;
    let (doc, job, _) = w
        .process_with(
            other,
            &fake,
            PipelineConfig::default().with_max_document_bytes(limit),
        )
        .await;
    assert_eq!(
        w.job(job).await.last_error.as_deref(),
        Some("the file is larger than the pipeline reads")
    );
    assert_eq!(w.reads(doc).await, Vec::new());
}

// ---------- the cap reaches into the second round ----------

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn the_cap_also_limits_the_pages_read_after_a_first_page_that_disagrees(pool: PgPool) {
    let w = world(&pool).await;
    let fake = Arc::new(
        FakeWorker::scripted(vec![Ok(pdf(3, 3))])
            .with_texts(vec![Ok(good_pages(3))])
            .with_ocr(vec![Ok(ocr_read(1, Some(300))), Ok(ocr_read(2, Some(1000)))]),
    );
    let (doc, _, _) = w
        .process_with(FILE, &fake, PipelineConfig::default().with_ocr_page_cap(2))
        .await;
    let pages: Vec<u32> = fake.ocr_calls().iter().map(|c| c.page.get()).collect();
    assert_eq!(pages, [1, 2]);
    let reads = w.reads(doc).await;
    assert_eq!(reads[2].ocr_agreement(), None);
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn an_ocr_answer_that_was_not_read_by_ocr_is_a_transient_failure(pool: PgPool) {
    let w = world(&pool).await;
    let fake = Arc::new(
        FakeWorker::scripted(vec![Ok(pdf(1, 1))])
            .with_texts(vec![Ok(good_pages(1))])
            .with_ocr(vec![Ok(text_page(1, GOOD))]),
    );
    let (doc, _, report) = w.process(FILE, &fake).await;
    assert!(
        matches!(report.ended, Ended::Retrying { counts: true, .. }),
        "{:?}",
        report.ended
    );
    nothing_was_written(&w, doc).await;
}

// ---------- a checkpoint is trusted only when everything it says is true ----------

/// The same reads, stored as if they had been analysed under other rules.
fn under_old_rules(reads: &[PageRead]) -> Vec<PageRead> {
    reads
        .iter()
        .map(|r| {
            let names: Vec<&str> = r.reasons().iter().map(|reason| reason.as_str()).collect();
            PageRead::from_stored(clearcut_core::page_read::StoredPageRead {
                page: r.page(),
                text_source: r.text_source().as_str(),
                verdict: r.verdict().as_str(),
                reasons: &names,
                ocr_agreement: r.ocr_agreement(),
                route: r.route().as_str(),
                analysed_with: "routing-v0",
            })
            .unwrap()
        })
        .collect()
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_checkpoint_is_not_trusted_unless_stage_rules_pages_and_rows_all_agree(pool: PgPool) {
    let w = world(&pool).await;
    let cases: Vec<(&str, serde_json::Value, Vec<PageRead>)> = vec![
        (
            "another stage",
            json!({ "stage": "something-else", "pages": 3, "rules": ROUTING_VERSION }),
            three_reads(),
        ),
        (
            "no pages",
            json!({ "stage": CHECKPOINT_STAGE, "pages": 0, "rules": ROUTING_VERSION }),
            Vec::new(),
        ),
        (
            "fewer rows than pages",
            analysed_checkpoint(ROUTING_VERSION),
            three_reads()[..2].to_vec(),
        ),
        (
            "rows from other rules",
            analysed_checkpoint(ROUTING_VERSION),
            under_old_rules(&three_reads()),
        ),
    ];
    for (case, checkpoint, rows) in cases {
        let bytes = format!("checkpoint case: {case}");
        let doc = w.document(bytes.as_bytes()).await;
        let job = w.enqueue(doc).await;
        if !rows.is_empty() {
            w.store.replace_page_reads(w.tenant, doc, &rows).await.unwrap();
        }
        abandoned_with(&w, job, checkpoint).await;

        let fake = Arc::new(
            FakeWorker::scripted(vec![Ok(pdf(3, 3))])
                .with_texts(vec![Ok(good_pages(3))])
                .with_ocr(vec![Ok(ocr_read(1, Some(1000)))]),
        );
        let report = w.run(w.handler(&fake, PipelineConfig::default())).await;
        assert_eq!(report.ended, Ended::NeedsReview, "{case}");
        assert_eq!(fake.documents().len(), 1, "{case}: the analysis was done again");
        let reads = w.reads(doc).await;
        assert_eq!(reads.len(), 3, "{case}");
        assert!(
            reads.iter().all(|r| r.analysed_with() == ROUTING_VERSION),
            "{case}"
        );
    }
}

// ---------- progress is saved as the work goes ----------

/// A worker that notes what the job's saved progress is each time it is asked for text.
struct WatchesProgress {
    inner: Arc<FakeWorker>,
    store: Store,
    tenant: TenantId,
    job: Uuid,
    seen: std::sync::Mutex<Vec<u32>>,
}

impl DocumentProcessor for WatchesProgress {
    fn inspect(&self, document: Bytes) -> impl Future<Output = Result<Inspection, ProcessError>> + Send {
        self.inner.inspect(document)
    }

    async fn page_texts(
        &self,
        document: Bytes,
        range: PageRange,
        words: bool,
    ) -> Result<Vec<PageContent>, ProcessError> {
        let job = self.store.get_job(self.tenant, self.job).await.unwrap();
        self.seen.lock().unwrap().push(job.progress.done());
        self.inner.page_texts(document, range, words).await
    }

    fn ocr_page(
        &self,
        document: Bytes,
        page: NonZeroU32,
        agreement: bool,
    ) -> impl Future<Output = Result<PageContent, ProcessError>> + Send {
        self.inner.ocr_page(document, page, agreement)
    }

    fn render_page(
        &self,
        document: Bytes,
        page: NonZeroU32,
        options: RenderOptions,
    ) -> impl Future<Output = Result<RenderedPage, ProcessError>> + Send {
        self.inner.render_page(document, page, options)
    }

    fn mask_image(
        &self,
        image: Bytes,
        boxes: Vec<MaskBox>,
        format: ImageFormat,
        max_edge: u16,
    ) -> impl Future<Output = Result<RenderedPage, ProcessError>> + Send {
        self.inner.mask_image(image, boxes, format, max_edge)
    }
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn the_pages_read_so_far_are_saved_after_each_chunk(pool: PgPool) {
    let w = world(&pool).await;
    let doc = w.document(FILE).await;
    let job = w.enqueue(doc).await;
    let chunk = |from: u32, to: u32| Ok((from..=to).map(|p| text_page(p, GOOD)).collect());
    let watching = Arc::new(WatchesProgress {
        inner: Arc::new(
            FakeWorker::scripted(vec![Ok(pdf(70, 70))])
                .with_texts(vec![chunk(1, 50), chunk(51, 70)])
                .with_ocr(vec![Ok(ocr_read(1, Some(1000)))]),
        ),
        store: w.store.clone(),
        tenant: w.tenant,
        job,
        seen: std::sync::Mutex::new(Vec::new()),
    });
    w.run(w.handler(&watching, PipelineConfig::default())).await;
    // Before the first chunk nothing was read; before the second, fifty pages were.
    assert_eq!(*watching.seen.lock().unwrap(), [0, 50]);
}
