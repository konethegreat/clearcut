// Written by Kone Tshivhinda & Claude
//! The `process_document` job handler and its stages: the analysis (ADR-0017) and the
//! reading of the pages by the model router (ADR-0018).
//!
//! A document is a stored file. The analysis finds out **how each page should be read** and
//! records it, so every later stage knows whether to trust a page's text layer, read its
//! picture first, or read the picture alone. It calls no model, so it needs no key and costs
//! nothing:
//!
//! ```text
//! read the file from storage (size and hash checked against the record)
//!   worker.inspect        -> the page count
//!   worker.page_texts     -> every page's text and fonts, 50 pages at a time
//!   core::page_read       -> which pages OCR reads (page 1, and every page not already Good)
//!   worker.ocr_page       -> the OCR agreement (and the text of a page with no text layer)
//!   core::page_read       -> the verdict, the reasons and the route of every page
//!   store.replace_page_reads, audit, checkpoint   (all or nothing)
//! ```
//!
//! The rules are in `clearcut-core` (`page_read`, `text_quality`); this crate only fetches,
//! calls and records, and asks the core what to do. Nothing the document says is kept,
//! logged or put in an error: only counts, names from fixed lists and page numbers.
//!
//! **The read stage** ([`read`]) runs only when the handler is given a model provider
//! ([`PipelineHandler::with_models`]); without one the job ends after the analysis, as before,
//! and no model is ever asked. It reads the pages in order with the router, stores each page's
//! reading (or its best draft), logs every call and what it was charged, and resumes a page
//! that was stopped by a failure of the service.
//!
//! **What the handler returns.** The stages after the analysis (classify, extract, validate,
//! events) do not exist yet, and a document must never be marked `done` for work that was not
//! done (ADR-0005: every job ends `done` or `needs_review`). So, for now, the job ends
//! `needs_review` with [`AFTER_ANALYSIS`] as the reason. The ending is the one line to change
//! when the next stage is built.
//!
//! **Running twice.** The queue promises "at least once". The analysis writes all its rows in
//! one transaction and replaces the earlier ones, and after it a checkpoint is saved, so a
//! job taken over after the analysis does not do it again; one taken over before it does it
//! again from the start, with the same result.

mod call_log;
mod month;
mod read;

pub use month::{month_start, SOUTH_AFRICA_OFFSET_SECONDS};
pub use read::READ_STAGE;

#[cfg(test)]
mod tests;

use bytes::Bytes;
use clearcut_core::cost::BudgetPolicy;
use clearcut_core::jobs::Progress;
use clearcut_core::page_read::{
    first_ocr_round, second_ocr_round, OcrRun, PageEvidence, PageRead, ReadRoute, DEFAULT_OCR_PAGE_CAP,
    ROUTING_VERSION,
};
use clearcut_core::text_quality::TextVerdict;
use clearcut_docproc::{DocumentProcessor, PageContent, PageRange, PageSource, ProcessError};
use clearcut_jobs::{Failure, Finished, Handler, HandlerResult, JobContext, ReportError};
use clearcut_objects::{ObjectKey, ObjectStore};
use clearcut_provider::{ModelProvider, ModelRequest, ModelResponse, ProviderError};
use clearcut_router::RouterConfig;
use clearcut_store::{AuditEntry, DocId, Job, Store, StoredFile, TenantId};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::future::Future;
use std::num::NonZeroU32;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use time::UtcOffset;
use tokio::io::AsyncReadExt;

/// Why a job ends `needs_review` once the analysis is done: nothing comes after it yet.
pub const AFTER_ANALYSIS: &str = "analysed; the later stages are not built yet";

/// The name saved in the checkpoint once the analysis is complete.
pub const CHECKPOINT_STAGE: &str = "analysed";

/// The largest file the pipeline reads into memory, unless a setting says otherwise.
pub const DEFAULT_MAX_DOCUMENT_BYTES: u64 = 100 * 1024 * 1024;

/// Settings of the pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipelineConfig {
    ocr_page_cap: u32,
    max_document_bytes: u64,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        PipelineConfig {
            ocr_page_cap: DEFAULT_OCR_PAGE_CAP,
            max_document_bytes: DEFAULT_MAX_DOCUMENT_BYTES,
        }
    }
}

impl PipelineConfig {
    /// The most pages of one document that OCR reads (OCR costs about a second a page).
    pub fn with_ocr_page_cap(mut self, pages: u32) -> Self {
        self.ocr_page_cap = pages;
        self
    }

    /// The largest file read into memory; a larger one goes to a person.
    pub fn with_max_document_bytes(mut self, bytes: u64) -> Self {
        self.max_document_bytes = bytes;
        self
    }

    pub fn ocr_page_cap(&self) -> u32 {
        self.ocr_page_cap
    }

    pub fn max_document_bytes(&self) -> u64 {
        self.max_document_bytes
    }
}

/// What the read stage needs beyond the router's own settings.
#[derive(Debug, Clone)]
pub struct ReadSettings {
    router: RouterConfig,
    budget: BudgetPolicy,
    month_offset: UtcOffset,
}

impl ReadSettings {
    /// The router's settings with the default budget (R5 a document, R0.50 a page for longer
    /// ones, R100 a month) and the South African time zone for the month.
    pub fn new(router: RouterConfig) -> Self {
        ReadSettings {
            router,
            budget: BudgetPolicy::default(),
            month_offset: UtcOffset::from_whole_seconds(SOUTH_AFRICA_OFFSET_SECONDS)
                .unwrap_or(UtcOffset::UTC),
        }
    }

    /// The defaults of ADR-0018 with the real reading schema.
    pub fn standard() -> Self {
        let schema =
            serde_json::to_value(clearcut_core::reading::reading_schema()).expect("a JSON schema is JSON");
        ReadSettings::new(RouterConfig::new(schema))
    }

    pub fn with_budget(mut self, budget: BudgetPolicy) -> Self {
        self.budget = budget;
        self
    }

    /// The time zone whose calendar month the monthly budget counts.
    pub fn with_month_offset(mut self, offset: UtcOffset) -> Self {
        self.month_offset = offset;
        self
    }

    pub fn router(&self) -> &RouterConfig {
        &self.router
    }

    pub fn budget(&self) -> &BudgetPolicy {
        &self.budget
    }

    pub fn month_offset(&self) -> UtcOffset {
        self.month_offset
    }
}

/// The provider of a handler that has none: it cannot be built, so it cannot be asked.
pub enum NoModel {}

impl ModelProvider for NoModel {
    async fn complete(&self, _request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        match *self {}
    }
}

/// A provider and the settings for reading with it.
struct Models<M> {
    provider: Arc<M>,
    settings: ReadSettings,
}

impl<M> Clone for Models<M> {
    fn clone(&self) -> Self {
        Models {
            provider: self.provider.clone(),
            settings: self.settings.clone(),
        }
    }
}

/// Why a tenant has no model provider just now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoProvider {
    /// No key this tenant may use: the job waits until one is set.
    NoKey,
    /// The tenant's own key was refused: it must be set again.
    Rejected,
}

pub type ProviderFuture<'a, M> = Pin<Box<dyn Future<Output = Result<Arc<M>, NoProvider>> + Send + 'a>>;
pub type NoticeFuture<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

/// Where a job gets its model provider: one per tenant, because each tenant brings its own Anthropic
/// key (ADR-0019). The pipeline knows nothing about keys; it asks, and waits if there is none.
pub trait ProviderSource<M>: Send + Sync + 'static {
    /// The provider for this tenant's job.
    fn provider<'a>(&'a self, tenant: TenantId) -> ProviderFuture<'a, M>;

    /// The provider refused the tenant's key during a job.
    fn key_rejected<'a>(&'a self, tenant: TenantId) -> NoticeFuture<'a>;
}

/// How a handler gets its provider.
enum ModelSupply<M> {
    /// One provider for every job (tests, and a single-owner server).
    Fixed(Models<M>),
    /// One per tenant.
    PerTenant {
        source: Arc<dyn ProviderSource<M>>,
        settings: ReadSettings,
    },
}

/// How long a job waits for a key to be set before looking again. Setting a key starts it within
/// about this long.
const KEY_WAIT: Duration = Duration::from_secs(60);

/// Why a run stopped before the end: cancelled, or failed in a way the queue decides on.
enum Stop {
    Cancelled,
    Failed(Failure),
}

impl From<Failure> for Stop {
    fn from(failure: Failure) -> Self {
        Stop::Failed(failure)
    }
}

type Step<T> = Result<T, Stop>;

fn needs_review(message: &str) -> Stop {
    Stop::Failed(Failure::needs_review(message))
}

fn transient(message: &str) -> Stop {
    Stop::Failed(Failure::transient(message))
}

/// A worker's error, with what the queue should do about it. The message is the error's own
/// fixed text and never repeats anything the worker said.
fn worker_failure(error: &ProcessError) -> Stop {
    Stop::Failed(Failure {
        kind: error.failure_kind(),
        message: error.to_string(),
        retry_after: None,
    })
}

fn store_failure(error: &clearcut_store::Error) -> Stop {
    use clearcut_store::Error;
    match error {
        Error::DocumentNotFound => needs_review("the document was not found"),
        Error::InvalidPageReads(_) | Error::InvalidStoredPageRead(_) => {
            needs_review("the page records are not valid")
        }
        other => {
            tracing::warn!(error = %other, "the database refused a request of the analysis");
            transient("the database could not complete the request")
        }
    }
}

fn report_failure(error: &ReportError) -> Stop {
    match error {
        ReportError::LeaseLost => transient("the lease on the job was lost"),
        ReportError::Store(_) => transient("progress could not be saved"),
    }
}

/// Reads the document's file, analyses it and, if it has a model provider, reads its pages,
/// as the `process_document` job.
pub struct PipelineHandler<O, P, M = NoModel> {
    store: Store,
    objects: O,
    worker: Arc<P>,
    config: PipelineConfig,
    models: Option<ModelSupply<M>>,
}

impl<O, P> PipelineHandler<O, P, NoModel> {
    /// A handler that analyses documents and asks no model.
    pub fn new(store: Store, objects: O, worker: Arc<P>, config: PipelineConfig) -> Self {
        PipelineHandler {
            store,
            objects,
            worker,
            config,
            models: None,
        }
    }
}

impl<O, P, M> PipelineHandler<O, P, M> {
    /// Gives the handler a model provider: it then reads the pages after the analysis.
    ///
    /// **Nothing may run against a real key until the monthly ledger and the spend gate of
    /// P2-12 exist** (two documents running at once do not yet see each other's spending).
    pub fn with_models<M2: ModelProvider>(
        self,
        provider: Arc<M2>,
        settings: ReadSettings,
    ) -> PipelineHandler<O, P, M2> {
        PipelineHandler {
            store: self.store,
            objects: self.objects,
            worker: self.worker,
            config: self.config,
            models: Some(ModelSupply::Fixed(Models { provider, settings })),
        }
    }

    /// Gives the handler a provider **per tenant** (each tenant brings its own key). A job whose
    /// tenant has no usable key is analysed, then waits (without using up attempts) until one is set.
    pub fn with_provider_source<M2: ModelProvider>(
        self,
        source: Arc<dyn ProviderSource<M2>>,
        settings: ReadSettings,
    ) -> PipelineHandler<O, P, M2> {
        PipelineHandler {
            store: self.store,
            objects: self.objects,
            worker: self.worker,
            config: self.config,
            models: Some(ModelSupply::PerTenant { source, settings }),
        }
    }
}

impl<O: ObjectStore + 'static, P: DocumentProcessor, M: ModelProvider> PipelineHandler<O, P, M> {
    async fn run(&self, job: &Job, ctx: &JobContext) -> Step<Finished> {
        let (tenant, document) = (job.tenant, job.document);
        let analysed = self.already_analysed(ctx, tenant, document).await?;
        let Some(supply) = &self.models else {
            if !analysed {
                let bytes = self.load_file(tenant, document).await?;
                self.analyse(ctx, tenant, document, &bytes).await?;
            }
            return Ok(Finished::NeedsReview(AFTER_ANALYSIS.to_string()));
        };
        let bytes = self.load_file(tenant, document).await?;
        if !analysed {
            self.analyse(ctx, tenant, document, &bytes).await?;
        }
        let models = match supply {
            ModelSupply::Fixed(models) => models.clone(),
            ModelSupply::PerTenant { source, settings } => match source.provider(tenant).await {
                Ok(provider) => Models {
                    provider,
                    settings: settings.clone(),
                },
                // Not a failure of the document: it waits, and starts soon after a key is set.
                Err(NoProvider::NoKey) => {
                    return Err(Stop::Failed(
                        Failure::provider_unavailable("waiting for an Anthropic key: set one in Settings")
                            .with_retry_after(KEY_WAIT),
                    ));
                }
                Err(NoProvider::Rejected) => {
                    return Err(Stop::Failed(
                        Failure::provider_unavailable(
                            "the Anthropic key was refused: set a new one in Settings",
                        )
                        .with_retry_after(KEY_WAIT),
                    ));
                }
            },
        };
        self.read_pages(ctx, job, &bytes, &models).await
    }

    /// Tells the provider source the tenant's own key was refused during a job.
    async fn key_was_rejected(&self, tenant: TenantId) {
        if let Some(ModelSupply::PerTenant { source, .. }) = &self.models {
            source.key_rejected(tenant).await;
        }
    }

    /// The document's file, read out of storage and checked against its record.
    async fn load_file(&self, tenant: TenantId, document: DocId) -> Step<Bytes> {
        let file = self
            .store
            .document_file(tenant, document)
            .await
            .map_err(|e| store_failure(&e))?;
        self.read_file(&file).await
    }

    /// Whether an earlier run saved the end of the analysis and its rows are still there.
    async fn already_analysed(&self, ctx: &JobContext, tenant: TenantId, document: DocId) -> Step<bool> {
        let Some(saved) = ctx.checkpoint() else {
            return Ok(false);
        };
        let pages = saved.get("pages").and_then(Value::as_u64);
        // The analysis is done once its own stage, or a later one, was saved.
        let stage = saved.get("stage").and_then(Value::as_str);
        let current = matches!(stage, Some(CHECKPOINT_STAGE | READ_STAGE))
            && saved.get("rules").and_then(Value::as_str) == Some(ROUTING_VERSION);
        let Some(pages) = pages.filter(|pages| current && *pages > 0) else {
            return Ok(false);
        };
        let rows = self
            .store
            .load_page_reads(tenant, document)
            .await
            .map_err(|e| store_failure(&e))?;
        Ok(u64::try_from(rows.len()) == Ok(pages)
            && rows.iter().all(|r| r.analysed_with() == ROUTING_VERSION))
    }

    /// Reads the file out of storage and checks it is the file that was recorded.
    async fn read_file(&self, file: &StoredFile) -> Step<Bytes> {
        if file.byte_size > self.config.max_document_bytes {
            return Err(needs_review("the file is larger than the pipeline reads"));
        }
        let key = ObjectKey::parse(&file.storage_key)
            .map_err(|_| needs_review("the stored file has no valid location"))?;
        let reader = match self.objects.open(&key).await {
            Ok(reader) => reader,
            Err(clearcut_objects::Error::NotFound) => return Err(needs_review("the stored file is missing")),
            Err(_) => return Err(transient("the stored file could not be opened")),
        };
        // One byte more than recorded, so a file that grew is noticed without reading it all.
        let mut content = Vec::new();
        reader
            .take(file.byte_size.saturating_add(1))
            .read_to_end(&mut content)
            .await
            .map_err(|_| transient("the stored file could not be read"))?;
        let digest: [u8; 32] = Sha256::digest(&content).into();
        if u64::try_from(content.len()) != Ok(file.byte_size) || digest != file.sha256 {
            return Err(needs_review("the stored file does not match its record"));
        }
        Ok(Bytes::from(content))
    }

    /// Saves progress and notices a request to stop.
    async fn progress(&self, ctx: &JobContext, done: u32, total: u32, note: &str) -> Step<()> {
        let progress =
            Progress::new(done, Some(total)).map_err(|_| transient("progress was out of range"))?;
        ctx.report(progress, Some(note), None)
            .await
            .map_err(|e| report_failure(&e))?;
        if ctx.is_cancelled() {
            return Err(Stop::Cancelled);
        }
        Ok(())
    }

    /// The text of every page, 50 pages at a time, each answer checked to be the pages asked for.
    async fn read_texts(&self, ctx: &JobContext, bytes: &Bytes, pages: u32) -> Step<Vec<PageContent>> {
        let mut contents = Vec::new();
        let mut first = 1u32;
        while first <= pages {
            let last = pages.min(first.saturating_add(PageRange::MAX_PAGES - 1));
            let range = PageRange::new(page_number(first)?, Some(page_number(last)?))
                .map_err(|_| transient("the page range was not valid"))?;
            let answer = self
                .worker
                .page_texts(bytes.clone(), range, false)
                .await
                .map_err(|e| worker_failure(&e))?;
            let right_pages = answer.len() as u64 == u64::from(last - first + 1)
                && answer
                    .iter()
                    .zip(first..)
                    .all(|(content, expected)| content.page().get() == expected);
            if !right_pages {
                return Err(transient("the document worker answered with the wrong pages"));
            }
            contents.extend(answer);
            self.progress(ctx, last, pages, "reading the text").await?;
            first = last + 1;
        }
        Ok(contents)
    }

    /// Analyses the document: reads it, decides how each page is read, saves all of it.
    async fn analyse(&self, ctx: &JobContext, tenant: TenantId, document: DocId, bytes: &Bytes) -> Step<()> {
        let inspection = self
            .worker
            .inspect(bytes.clone())
            .await
            .map_err(|e| worker_failure(&e))?;
        let pages = inspection.pages();
        if pages == 0 {
            return Err(needs_review("the document has no pages"));
        }
        self.progress(ctx, 0, pages, "reading the text").await?;
        let contents = self.read_texts(ctx, bytes, pages).await?;

        // The verdict from the text alone says which pages OCR is asked about.
        let verdicts: Vec<TextVerdict> = contents
            .iter()
            .map(|content| decide(content, None).verdict())
            .collect();
        let mut ocr = Ocr::default();
        let cap = self.config.ocr_page_cap;
        let first_round = first_ocr_round(&verdicts, cap);
        self.ocr_pages(ctx, bytes, &contents, &first_round, &mut ocr)
            .await?;
        let page_one = ocr.read.get(&1).copied().flatten();
        let second_round = second_ocr_round(&verdicts, &first_round, page_one, cap);
        self.ocr_pages(ctx, bytes, &contents, &second_round, &mut ocr)
            .await?;

        let reads: Vec<PageRead> = contents
            .iter()
            .map(|content| {
                let run = ocr.read.get(&content.page().get()).map(|agreement| OcrRun {
                    agreement: *agreement,
                });
                decide(content, run)
            })
            .collect();

        self.store
            .replace_page_reads(tenant, document, &reads)
            .await
            .map_err(|e| store_failure(&e))?;
        self.store
            .record_audit(
                tenant,
                &AuditEntry {
                    actor: json!({ "type": "system" }),
                    action: "document.analysed".to_string(),
                    subject_type: "document".to_string(),
                    subject_id: document.0.to_string(),
                    detail: audit_detail(&reads, &ocr),
                },
            )
            .await
            .map_err(|e| store_failure(&e))?;

        let done = Progress::new(pages, Some(pages)).map_err(|_| transient("progress was out of range"))?;
        let checkpoint = json!({ "stage": CHECKPOINT_STAGE, "pages": pages, "rules": ROUTING_VERSION });
        ctx.report(done, Some("analysed"), Some(checkpoint))
            .await
            .map_err(|e| report_failure(&e))?;
        Ok(())
    }

    /// Has OCR read each of `pages`, noting what it found. A worker without OCR is not an
    /// error: nothing more is asked of it and the pages are routed without OCR.
    async fn ocr_pages(
        &self,
        ctx: &JobContext,
        bytes: &Bytes,
        contents: &[PageContent],
        pages: &[u32],
        ocr: &mut Ocr,
    ) -> Step<()> {
        let total = u32::try_from(contents.len()).unwrap_or(u32::MAX);
        for &page in pages {
            if ocr.unavailable {
                break;
            }
            let Some(content) = page
                .checked_sub(1)
                .and_then(|i| contents.get(usize::try_from(i).ok()?))
            else {
                return Err(transient("a page to read by OCR does not exist"));
            };
            // Only a page with a text layer has something for OCR to be compared with.
            let compare = content.source() == PageSource::TextLayer;
            match self
                .worker
                .ocr_page(bytes.clone(), page_number(page)?, compare)
                .await
            {
                Ok(read) if read.page().get() == page && read.source() == PageSource::Ocr => {
                    ocr.read.insert(page, read.agreement());
                }
                Ok(_) => return Err(transient("the document worker answered with the wrong page")),
                Err(ProcessError::NotAvailable) => ocr.unavailable = true,
                Err(error) => return Err(worker_failure(&error)),
            }
            self.progress(ctx, total, total, "reading pages by OCR").await?;
        }
        Ok(())
    }
}

impl<O: ObjectStore + 'static, P: DocumentProcessor, M: ModelProvider> Handler for PipelineHandler<O, P, M> {
    async fn handle(&self, job: Job, ctx: JobContext) -> HandlerResult {
        match self.run(&job, &ctx).await {
            Ok(finished) => Ok(finished),
            Err(Stop::Cancelled) => Ok(Finished::Cancelled),
            Err(Stop::Failed(failure)) => Err(failure),
        }
    }
}

/// What OCR has been asked and answered so far.
#[derive(Default)]
struct Ocr {
    /// The pages OCR read, with the agreement for each (None when there was no text layer).
    read: BTreeMap<u32, Option<u16>>,
    /// The worker said it has no OCR.
    unavailable: bool,
}

fn page_number(page: u32) -> Step<NonZeroU32> {
    NonZeroU32::new(page).ok_or_else(|| transient("a page number was zero"))
}

fn decide(content: &PageContent, ocr: Option<OcrRun>) -> PageRead {
    PageRead::decide(&PageEvidence {
        page: content.page().get(),
        text: content.text(),
        fonts: content.fonts(),
        ocr,
    })
}

/// Counts and fixed names only: nothing from the document.
fn audit_detail(reads: &[PageRead], ocr: &Ocr) -> Value {
    let count = |route: ReadRoute| reads.iter().filter(|read| read.route() == route).count();
    let rejected = reads.iter().filter(|read| read.text_layer_rejected()).count();
    json!({
        "pages": reads.len(),
        "ocr": if ocr.unavailable { "unavailable" } else { "ran" },
        "ocr_pages": ocr.read.len(),
        "routes": {
            ReadRoute::TextAndImage.as_str(): count(ReadRoute::TextAndImage),
            ReadRoute::ImageFirst.as_str(): count(ReadRoute::ImageFirst),
            ReadRoute::ImageOnly.as_str(): count(ReadRoute::ImageOnly),
        },
        "text_layers_rejected": rejected,
        "rules": ROUTING_VERSION,
    })
}
