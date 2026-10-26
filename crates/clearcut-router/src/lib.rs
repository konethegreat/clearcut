// Written by Kone Tshivhinda & Claude
//! # clearcut-router
//!
//! The driver that reads **one page** (ADR-0018 section 1): it asks the page state machine
//! (`clearcut_core::escalation::PageReader`) what to do next, does it, and says what happened.
//!
//! ```text
//! worker: the page's text, a picture of the size the rung wants, personal data painted out
//!   -> pack: the request (clearcut-pack)
//!   -> budget: would the worst case of this call break the document's or the month's limit?
//!   -> provider: one call (clearcut-provider)
//!   -> convert, validate, check against the page's own text, assess (clearcut-core)
//!   -> the state machine: accept, a stronger model, or a person
//! ```
//!
//! **What this crate is.** The glue between pieces that each have their own tests. Every rule
//! (when a reading is good enough, what a call costs, what the text can vouch for, what a pack
//! may hold) lives in `clearcut-core` or `clearcut-pack`; this crate only fetches, calls and
//! records. It has no database: the caller (the pipeline's read stage) keeps the money ledger
//! and stores what comes back.
//!
//! **Failures of the transport stop the page, they do not use up an attempt.** A provider that
//! is down or throttling, a bad key, a timeout, a worker that cannot be reached: the adapter
//! has already retried what is worth retrying, so the page returns [`PageRun::Interrupted`]
//! with a [`Carry`]: the state machine (which still holds the call it was about to make) and
//! the readings kept so far. A job stores the carry and resumes with it later, and the very
//! same call is made again. Nothing here ever tries a second model because the first one's
//! *service* failed; a stronger model is for a *reading* that failed its checks.
//!
//! **Nothing in a result holds a word of the document except the reading itself.** The call
//! log ([`CallRecord`]) has sizes, names, tokens, rand and the pack's manifest (which has no
//! text); the reading and the extraction are the one place the page's content goes, and the
//! caller stores them as events.

mod estimate;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_estimate;

use bytes::Bytes;
use clearcut_core::cost::{BudgetCheck, BudgetScope, Cost, ExchangeRate, PriceTable, TokenUsage};
use clearcut_core::escalation::{
    assess, Assessment, CallKind, CallPlan, ConfidenceBasis, EscalationReason, Outcome, PageOutcome,
    PageReader, Picture, ReadingEvidence, ReviewReason, RouterPolicy, Step, TextCheck, ThinkingEffort,
    Verdict,
};
use clearcut_core::event::{Confidence, DocumentId, Extractor};
use clearcut_core::extraction::{validate, Extraction, Finding};
use clearcut_core::jobs::FailureKind;
use clearcut_core::page_read::{PageTextSource, ReadRoute};
use clearcut_core::reading::{ConversionContext, ConversionError, PageReading, ProblemKind, ReadingProblem};
use clearcut_core::redact;
use clearcut_core::text_check::{check_against_text, TextCheckReport, TextTrust};
use clearcut_core::Currency;
use clearcut_docproc::{
    DocumentProcessor, ImageFormat, PageContent, PageRange, PageSource, ProcessError, RenderOptions,
    RenderedPage, WorkerMasker,
};
use clearcut_pack::{
    build_pack, prompt_version, DocumentContext, FailureReport, ImageMasker, NoMasker, PackError,
    PackManifest, PackSettings, PageImage, PageInput, PageText, RedactionPolicy, MAX_PAGE_TEXT_BYTES,
};
use clearcut_provider::{
    CacheTtl, CallContext, Effort, ModelProvider, ModelResponse, ProviderError, Purpose, Usage,
};
use estimate::{call_estimate, uncached_text_bytes, Sizes};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

// ---------------------------------------------------------------- configuration ----

/// Everything the router needs that is not about one page.
#[derive(Debug, Clone)]
pub struct RouterConfig {
    /// The ladder. A caller that learns a key cannot use a model, or that a model does not fit
    /// the document's budget, builds the next page's router with a policy without it.
    pub policy: RouterPolicy,
    pub prices: PriceTable,
    pub rate: ExchangeRate,
    /// The schema the model must follow (`reading_schema()` in production).
    pub output_schema: Value,
    pub cache_ttl: CacheTtl,
    pub redaction: RedactionPolicy,
    /// Conventions confirmed by this tenant's reviewers; `None` until that memory exists.
    pub tenant_rules: Option<String>,
}

impl RouterConfig {
    /// The defaults of ADR-0018: the default ladder, the prices of research 06 at R18 to the
    /// dollar, a five-minute cache and best-effort redaction.
    pub fn new(output_schema: Value) -> RouterConfig {
        RouterConfig {
            policy: RouterPolicy::default_ladder(),
            prices: PriceTable::default_table(),
            rate: ExchangeRate::default(),
            output_schema,
            cache_ttl: CacheTtl::default(),
            redaction: RedactionPolicy::BestEffort,
            tenant_rules: None,
        }
    }
}

/// One page to read, and what is known about its document.
#[derive(Debug, Clone)]
pub struct PageTask {
    pub tenant: Uuid,
    pub job: Option<Uuid>,
    pub document_uuid: Uuid,
    /// The id the events cite.
    pub document: DocumentId,
    /// The stored file. Cheap to clone.
    pub file: Bytes,
    /// 1-based.
    pub page: NonZeroU32,
    /// How many pages the document has.
    pub of: NonZeroU32,
    /// From the analysis stage (ADR-0017).
    pub route: ReadRoute,
    pub text_source: PageTextSource,
    /// What earlier pages established.
    pub context: DocumentContext,
    /// A currency already established by **another page** of this document.
    pub currency_known: Option<Currency>,
}

// -------------------------------------------------------------------- the result ----

/// What a call came to, in fixed words (stored in the call log).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallLabel {
    /// The reading passed the checks and was kept.
    Accepted,
    /// The checks asked for a stronger model for this reason (at the top rung there is none,
    /// and the page goes to a person).
    Escalated(EscalationReason),
    /// The reading passed some checks and a person must look, whatever the model.
    Reviewed(ReviewReason),
    /// The answer was not the shape asked for.
    InvalidOutput,
    /// The answer was cut off at its cap.
    Truncated,
    /// The model declined.
    Refused,
    /// The provider rejected the request: a bug of ours.
    Rejected,
    /// The key cannot use the model.
    RungUnavailable,
    /// Nothing was sent, for this reason.
    NotSent(ReviewReason),
    /// The service failed before an answer came back; the page can resume.
    Interrupted(FailureKind),
}

/// One call the router made or considered, for the call log. **No content**: sizes, names,
/// tokens and rand only.
#[derive(Debug, Clone)]
pub struct CallRecord {
    pub attempt: u32,
    pub rung: usize,
    pub model: String,
    pub effort: ThinkingEffort,
    pub picture: Picture,
    pub kind: CallKind,
    pub prompt_version: String,
    /// What was sent, in names and sizes. `None` when no request was built.
    pub manifest: Option<PackManifest>,
    pub usage: TokenUsage,
    /// What the call cost: what was billed, never a guess. Zero for a call not made.
    pub cost: Cost,
    pub elapsed: Duration,
    pub request_id: Option<String>,
    pub label: CallLabel,
}

/// Why a budget stopped the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetStop {
    /// The worst case of the next call would have broken this limit.
    Scope(BudgetScope),
    /// The model has no price, so its cost cannot be bounded.
    NoPrice,
}

/// Where a reading came from: the rung and attempt that made it, and the model that answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadingOrigin {
    pub rung: usize,
    pub attempt: u32,
    pub model: String,
}

/// The highest rung number a ladder can have (the store keeps it in a column that holds 0 to 7).
const MAX_RUNG: usize = 7;

/// A page, finished: read, or handed to a person.
#[derive(Debug, Clone)]
pub struct PageResult {
    pub outcome: PageOutcome,
    /// The reading as facts, with the confidence the evidence gives. For a page that needs
    /// review, the best draft, with the lowest confidence. `None` if nothing was readable.
    pub extraction: Option<Extraction>,
    /// The reading as the model gave it (re-serialised from the checked shape), to store.
    pub reading: Option<Value>,
    /// Which rung, attempt and model gave `reading`.
    pub reading_from: Option<ReadingOrigin>,
    /// What could not be read exactly in `extraction`'s reading, and was left out.
    pub problems: Vec<ReadingProblem>,
    pub findings: Vec<Finding>,
    /// The places of the figures the page's own text did not confirm (never the figures).
    pub unconfirmed: Vec<String>,
    /// Every call of this run, in order.
    pub calls: Vec<CallRecord>,
    /// What this run spent. The budget given to the router already holds it.
    pub spent: Cost,
    /// Models the key cannot use. Later pages of the document should skip them.
    pub unavailable_models: Vec<String>,
    pub budget_stop: Option<BudgetStop>,
    /// Kinds of instruction-like text found on the page (never the text). Flagged, never obeyed.
    pub injection_suspected: Vec<&'static str>,
}

/// A reading kept between calls: enough to report on it, to send a stronger model what was
/// wrong with it, and to finish the page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Kept {
    pub attempt: u32,
    /// The rung that made it (a position in the policy's rungs).
    pub rung: usize,
    /// The model that gave it, as the provider named it.
    pub model: String,
    pub reading: Value,
}

/// A page stopped midway, to be resumed. **Read back from text, it is checked**: a stored
/// carry that has been damaged or edited is refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "UncheckedCarry")]
pub struct Carry {
    pub machine: PageReader,
    /// The reading with the fewest blocking issues so far, if any could be converted.
    pub best: Option<Kept>,
    /// The latest reading that could be converted: what the next model is told was wrong.
    pub last: Option<Kept>,
}

#[derive(Deserialize)]
struct UncheckedCarry {
    machine: PageReader,
    best: Option<Kept>,
    last: Option<Kept>,
}

impl TryFrom<UncheckedCarry> for Carry {
    type Error = &'static str;

    fn try_from(raw: UncheckedCarry) -> Result<Self, Self::Error> {
        for kept in [&raw.best, &raw.last].into_iter().flatten() {
            if kept.attempt == 0 {
                return Err("a kept reading has no attempt number");
            }
            if kept.rung > MAX_RUNG {
                return Err("a kept reading is from a rung outside the ladder");
            }
            if serde_json::from_value::<PageReading>(kept.reading.clone()).is_err() {
                return Err("a kept reading is not a reading");
            }
        }
        // The machine knows which attempt was best; the kept reading must be that one.
        if raw.machine.best_attempt() != raw.best.as_ref().map(|kept| kept.attempt) {
            return Err("the best reading is not the machine's best attempt");
        }
        if let (Some(best), Some(last)) = (&raw.best, &raw.last) {
            if best.attempt > last.attempt {
                return Err("the best reading is later than the last one");
            }
        }
        if raw.best.is_some() && raw.last.is_none() {
            return Err("there is a best reading but no last one");
        }
        if let (Some(plan), Some(last)) = (raw.machine.pending(), &raw.last) {
            if last.attempt >= plan.attempt {
                return Err("a kept reading is from a call that has not been made");
            }
        }
        Ok(Carry {
            machine: raw.machine,
            best: raw.best,
            last: raw.last,
        })
    }
}

/// The page stopped because the service failed, not because of the page.
#[derive(Debug)]
pub struct Interruption {
    /// What the job queue should do (ADR-0008).
    pub failure: FailureKind,
    /// Fixed words, never text from the document.
    pub message: String,
    /// When the provider said it is worth trying again.
    pub retry_after: Option<Duration>,
    /// The user's key itself was refused: it must be replaced (a tenant's stored key is marked so).
    pub key_rejected: bool,
    /// Store this and pass it back to `read_page` to carry on.
    pub carry: Carry,
    /// The calls of this run, including the one that failed.
    pub calls: Vec<CallRecord>,
    pub spent: Cost,
    pub unavailable_models: Vec<String>,
}

#[derive(Debug)]
pub enum PageRun {
    Finished(Box<PageResult>),
    Interrupted(Box<Interruption>),
}

// ------------------------------------------------------------------ the internals ----

/// Why the page could not go on: a service failed. No attempt is used up.
struct Trouble {
    failure: FailureKind,
    message: String,
    retry_after: Option<Duration>,
    key_rejected: bool,
}

impl Trouble {
    fn worker(error: ProcessError) -> Trouble {
        Trouble {
            failure: error.failure_kind(),
            message: error.to_string(),
            retry_after: None,
            key_rejected: false,
        }
    }

    fn worker_said(message: &str) -> Trouble {
        Trouble {
            failure: FailureKind::Transient,
            message: message.to_string(),
            retry_after: None,
            key_rejected: false,
        }
    }
}

/// The page's text as it goes to the model and how far it can be believed.
struct Material {
    /// What the pack gets (it redacts this itself).
    text: Option<PageText>,
    /// The worker's page, for the painter: its words say where the text is.
    content: Option<PageContent>,
    trust: TextTrust,
}

/// Everything worked out about one reading.
struct Evaluation {
    extraction: Extraction,
    problems: Vec<ReadingProblem>,
    findings: Vec<Finding>,
    report: TextCheckReport,
    assessment: Assessment,
}

/// A reading that cannot be turned into an extraction at all.
struct Unevaluable;

impl From<ConversionError> for Unevaluable {
    fn from(_: ConversionError) -> Self {
        Unevaluable
    }
}

fn effort_of(effort: ThinkingEffort) -> Effort {
    match effort {
        ThinkingEffort::Low => Effort::Low,
        ThinkingEffort::Medium => Effort::Medium,
        ThinkingEffort::High => Effort::High,
    }
}

fn token_usage(usage: Usage) -> TokenUsage {
    TokenUsage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cache_read_tokens: usage.cache_read_tokens,
        cache_write_tokens: usage.cache_write_tokens,
        cache_write_1h_tokens: usage.cache_write_1h_tokens,
    }
}

/// Converts, validates, checks against the page's text and assesses one reading.
/// `confidence` is what the values are stamped with: the evidence of the *final* verdict, so
/// the caller evaluates twice, once to decide and once to stamp.
fn evaluate(
    task: &PageTask,
    material: &Material,
    reading: &PageReading,
    model: &str,
    confidence: ConfidenceBasis,
) -> Result<Evaluation, Unevaluable> {
    let context = ConversionContext {
        document: task.document.clone(),
        page: task.page,
        extractor: Extractor::Model {
            name: model.to_string(),
            prompt_version: prompt_version(),
        },
        confidence: Confidence::new(confidence.per_mille()).map_err(|_| Unevaluable)?,
        currency_known: task.currency_known,
    };
    let conversion = reading.into_extraction(&context)?;
    let findings = validate(&conversion.extraction).findings;
    let report = match &material.text {
        Some(text) => check_against_text(&conversion.extraction, &text.content, material.trust),
        None => TextCheckReport {
            result: TextCheck::NotAvailable,
            checked: 0,
            unconfirmed: Vec::new(),
        },
    };
    let assessment = assess(&ReadingEvidence {
        problems: &conversion.problems,
        findings: &findings,
        text_check: report.result,
        whole_document: task.of.get() == 1,
    });
    Ok(Evaluation {
        extraction: conversion.extraction,
        problems: conversion.problems,
        findings,
        report,
        assessment,
    })
}

/// What a stronger model is told about the reading that failed, in ClearCut's own words:
/// what could not be read exactly, which figures the page does not show (only where its text
/// can be believed), and the checks that stopped the reading. Facts about the document that
/// no model could change (a due date before the invoice date, a missing tax-invoice field) are
/// left out: telling a model about them only invites it to invent a fix.
fn failure_report(task: &PageTask, material: &Material, last: Option<&Kept>) -> Option<FailureReport> {
    let kept = last?;
    let reading: PageReading = serde_json::from_value(kept.reading.clone()).ok()?;
    let evaluation = evaluate(task, material, &reading, &kept.model, ConfidenceBasis::Unresolved).ok()?;
    let mut problems = evaluation.problems;
    if matches!(evaluation.report.result, TextCheck::Failed { .. }) {
        problems.extend(evaluation.report.unconfirmed.iter().map(|path| ReadingProblem {
            path: path.clone(),
            kind: ProblemKind::NotOnPage,
        }));
    }
    let whole = task.of.get() == 1;
    let findings = evaluation
        .findings
        .into_iter()
        .filter(|finding| match finding {
            Finding::LineTotalMismatch { .. }
            | Finding::NonPositiveQuantity { .. }
            | Finding::AmountOutOfRange { .. }
            | Finding::CurrencyMismatch { .. } => true,
            Finding::TotalsMismatch { .. } | Finding::TotalsOutOfRange => whole,
            Finding::NoLines | Finding::DueBeforeIssue | Finding::Compliance(_) => false,
        })
        .collect();
    Some(FailureReport {
        previous_model: kept.model.clone(),
        problems,
        findings,
    })
}

/// The state of one run of a page.
struct Run {
    machine: PageReader,
    best: Option<Kept>,
    last: Option<Kept>,
    material: Option<Material>,
    pictures: Vec<(Picture, RenderedPage)>,
    calls: Vec<CallRecord>,
    spent: Cost,
    unavailable: Vec<String>,
    budget_stop: Option<BudgetStop>,
    injection: Vec<&'static str>,
}

impl Run {
    fn note_injection(&mut self, kinds: &[&'static str]) {
        for kind in kinds {
            if !self.injection.contains(kind) {
                self.injection.push(kind);
            }
        }
    }

    /// A call that was not made, or not sent.
    fn record_unsent(&mut self, plan: &CallPlan, manifest: Option<PackManifest>, label: CallLabel) {
        self.calls.push(CallRecord {
            attempt: plan.attempt,
            rung: plan.rung,
            model: plan.model.clone(),
            effort: plan.effort,
            picture: plan.picture,
            kind: plan.kind,
            prompt_version: prompt_version(),
            manifest,
            usage: TokenUsage::default(),
            cost: Cost::ZERO,
            elapsed: Duration::ZERO,
            request_id: None,
            label,
        });
    }
}

/// A request that was sent: what it held, the most it could have cost, and how long it took.
struct Sent {
    manifest: PackManifest,
    worst: Cost,
    elapsed: Duration,
}

/// What one attempt came to.
struct Attempted {
    outcome: Outcome,
    /// A reading that could be converted, to keep.
    kept: Option<Kept>,
}

impl Attempted {
    fn just(outcome: Outcome) -> Result<Attempted, Trouble> {
        Ok(Attempted { outcome, kept: None })
    }
}

// ---------------------------------------------------------------------- the router ----

pub struct Router<P, W> {
    provider: Arc<P>,
    worker: Arc<W>,
    config: RouterConfig,
}

impl<P: ModelProvider, W: DocumentProcessor> Router<P, W> {
    pub fn new(provider: Arc<P>, worker: Arc<W>, config: RouterConfig) -> Router<P, W> {
        Router {
            provider,
            worker,
            config,
        }
    }

    /// Reads one page, escalating up the ladder on evidence, or resumes a page that was
    /// interrupted (`resume` is the [`Carry`] the interruption gave).
    ///
    /// `budget` is the document's spending so far and is updated with every call's real cost.
    /// `document_calls_left` is what the document may still spend in calls
    /// (`RouterPolicy::document_call_cap` less what earlier pages used).
    pub async fn read_page(
        &self,
        task: &PageTask,
        budget: &mut BudgetCheck,
        document_calls_left: u32,
        resume: Option<Carry>,
    ) -> PageRun {
        let (machine, mut step, best, last) = match resume {
            Some(carry) => {
                let step = match (carry.machine.pending(), carry.machine.outcome()) {
                    (Some(plan), _) => Step::Call(plan.clone()),
                    (None, Some(done)) => Step::Done(done.clone()),
                    // Not reachable for a checked machine; if it ever were, a person looks.
                    (None, None) => Step::Done(PageOutcome::NeedsReview {
                        reason: ReviewReason::RequestRejected,
                        draft: None,
                    }),
                };
                (carry.machine, step, carry.best, carry.last)
            }
            None => {
                let (machine, step) = PageReader::start(&self.config.policy, document_calls_left);
                (machine, step, None, None)
            }
        };
        let mut run = Run {
            machine,
            best,
            last,
            material: None,
            pictures: Vec::new(),
            calls: Vec::new(),
            spent: Cost::ZERO,
            unavailable: Vec::new(),
            budget_stop: None,
            injection: Vec::new(),
        };
        loop {
            let plan = match step {
                Step::Done(outcome) => return PageRun::Finished(Box::new(self.finish(run, task, outcome))),
                Step::Call(plan) => plan,
            };
            match self.attempt(&mut run, task, &plan, budget).await {
                Ok(attempted) => {
                    step = run.machine.on_outcome(attempted.outcome);
                    if let Some(kept) = attempted.kept {
                        if run.machine.best_attempt() == Some(kept.attempt) {
                            run.best = Some(kept.clone());
                        }
                        run.last = Some(kept);
                    }
                }
                Err(trouble) => {
                    return PageRun::Interrupted(Box::new(Interruption {
                        failure: trouble.failure,
                        message: trouble.message,
                        retry_after: trouble.retry_after,
                        key_rejected: trouble.key_rejected,
                        carry: Carry {
                            machine: run.machine,
                            best: run.best,
                            last: run.last,
                        },
                        calls: run.calls,
                        spent: run.spent,
                        unavailable_models: run.unavailable,
                    }));
                }
            }
        }
    }

    /// Makes one planned call and says what came of it.
    async fn attempt(
        &self,
        run: &mut Run,
        task: &PageTask,
        plan: &CallPlan,
        budget: &mut BudgetCheck,
    ) -> Result<Attempted, Trouble> {
        if run.material.is_none() {
            run.material = Some(self.fetch_material(task).await?);
        }
        let rendered = self.picture(run, task, plan.picture).await?;
        let image = PageImage {
            media: rendered.media(),
            bytes: rendered.bytes().to_vec(),
        };

        // Personal data in the picture is painted over by the worker, from the redaction that
        // the pack will compute for the same text. A text that cannot vouch for the picture
        // (an unverified layer) is not painted from, and the pack records the picture as
        // unchecked. A painter that fails is a masker that says so, and the policy decides.
        let painter = match run.material.as_ref() {
            Some(Material {
                text: Some(text),
                content: Some(content),
                ..
            }) => {
                // (The painter itself refuses an unverified layer, without asking the worker.)
                let spans = redact::redact(&text.content).spans;
                WorkerMasker::prepare(&*self.worker, &image, content, text, &spans)
                    .await
                    .ok()
            }
            _ => None,
        };
        let no_painter = NoMasker;
        let masker: &dyn ImageMasker = match &painter {
            Some(painter) => painter,
            None => &no_painter,
        };

        let Some(material) = run.material.as_ref() else {
            return Err(Trouble::worker_said("the page's material was not fetched"));
        };
        let settings = PackSettings {
            model: plan.model.clone(),
            call: CallContext {
                tenant: task.tenant,
                job: task.job,
                document: Some(task.document_uuid),
                page: Some(task.page.get()),
                purpose: Purpose::Extraction,
            },
            effort: effort_of(plan.effort),
            max_output_tokens: plan.max_output_tokens,
            cache_ttl: self.config.cache_ttl,
            output_schema: self.config.output_schema.clone(),
            tenant_rules: self.config.tenant_rules.clone(),
            redaction: self.config.redaction,
        };
        let input = PageInput {
            page: task.page,
            of: task.of,
            image: Some(image),
            text: material.text.clone(),
        };
        let report = failure_report(task, material, run.last.as_ref());
        let pack = match build_pack(&settings, &input, &task.context, report.as_ref(), masker) {
            Ok(pack) => pack,
            Err(PackError::CannotBeSentSafely(_)) => {
                run.record_unsent(plan, None, CallLabel::NotSent(ReviewReason::CannotBeSentSafely));
                return Attempted::just(Outcome::CannotBeSentSafely);
            }
            Err(_) => {
                run.record_unsent(plan, None, CallLabel::Rejected);
                return Attempted::just(Outcome::Rejected);
            }
        };
        let request = pack.request;
        let manifest = pack.manifest;
        run.note_injection(&manifest.injection_suspected);

        // The worst this call could cost must fit what is left, or it is not made.
        let estimate = call_estimate(&Sizes {
            prefix_bytes: manifest.prefix_bytes,
            page_text_bytes: uncached_text_bytes(&manifest.sections),
            schema_bytes: serde_json::to_string(&self.config.output_schema)
                .unwrap_or_default()
                .len(),
            width: rendered.width(),
            height: rendered.height(),
            max_output_tokens: plan.max_output_tokens,
        });
        let worst = match self
            .config
            .prices
            .worst_case(&plan.model, &estimate, self.config.rate)
        {
            Ok(worst) => worst,
            Err(_) => {
                run.budget_stop = Some(BudgetStop::NoPrice);
                run.record_unsent(plan, Some(manifest), CallLabel::NotSent(ReviewReason::Budget));
                return Attempted::just(Outcome::BudgetRefused);
            }
        };
        if let Err(scope) = budget.check(worst) {
            run.budget_stop = Some(BudgetStop::Scope(scope));
            run.record_unsent(plan, Some(manifest), CallLabel::NotSent(ReviewReason::Budget));
            return Attempted::just(Outcome::BudgetRefused);
        }

        let sent = Sent {
            manifest,
            worst,
            elapsed: Duration::ZERO,
        };
        let started = Instant::now();
        let result = self.provider.complete(request).await;
        let sent = Sent {
            elapsed: started.elapsed(),
            ..sent
        };

        match result {
            Ok(response) => self.on_response(run, task, plan, budget, sent, response),
            Err(error) => self.on_error(run, plan, budget, sent, error),
        }
    }

    /// An answer came back: charge it, and read it.
    fn on_response(
        &self,
        run: &mut Run,
        task: &PageTask,
        plan: &CallPlan,
        budget: &mut BudgetCheck,
        sent: Sent,
        response: ModelResponse,
    ) -> Result<Attempted, Trouble> {
        let (usage, cost) = self.charge(run, budget, plan, response.usage, sent.worst);
        let model = if response.model.is_empty() {
            plan.model.clone()
        } else {
            response.model.clone()
        };
        let mut record = CallRecord {
            attempt: plan.attempt,
            rung: plan.rung,
            model: plan.model.clone(),
            effort: plan.effort,
            picture: plan.picture,
            kind: plan.kind,
            prompt_version: prompt_version(),
            manifest: Some(sent.manifest),
            usage,
            cost,
            elapsed: sent.elapsed,
            request_id: response.request_id.clone(),
            label: CallLabel::InvalidOutput,
        };
        let reading: PageReading = match serde_json::from_value(response.output) {
            Ok(reading) => reading,
            Err(_) => {
                run.calls.push(record);
                return Attempted::just(Outcome::InvalidOutput);
            }
        };
        let Some(material) = run.material.as_ref() else {
            return Err(Trouble::worker_said("the page's material was not fetched"));
        };
        let evaluation = match evaluate(task, material, &reading, &model, ConfidenceBasis::Unresolved) {
            Ok(evaluation) => evaluation,
            Err(Unevaluable) => {
                record.label = CallLabel::Escalated(EscalationReason::ReadingProblems);
                run.calls.push(record);
                return Attempted::just(Outcome::NotConverted);
            }
        };
        record.label = match evaluation.assessment.verdict {
            Verdict::Accept => CallLabel::Accepted,
            Verdict::Escalate(reason) => CallLabel::Escalated(reason),
            Verdict::Review(reason) => CallLabel::Reviewed(reason),
        };
        run.calls.push(record);
        Ok(Attempted {
            outcome: Outcome::Read(evaluation.assessment),
            kept: Some(Kept {
                attempt: plan.attempt,
                rung: plan.rung,
                model,
                reading: serde_json::to_value(&reading).unwrap_or(Value::Null),
            }),
        })
    }

    /// The provider did not give an answer.
    fn on_error(
        &self,
        run: &mut Run,
        plan: &CallPlan,
        budget: &mut BudgetCheck,
        sent: Sent,
        error: ProviderError,
    ) -> Result<Attempted, Trouble> {
        // Billed failures are charged what they were billed.
        let billed = error.usage();
        let (usage, cost) = match billed {
            // A billed failure that reports no tokens at all (an answer too large to read) cannot
            // be costed from them, so it counts at the most the call could have cost.
            Some(usage) if usage == Usage::default() => {
                Self::book(run, budget, sent.worst);
                (TokenUsage::default(), sent.worst)
            }
            Some(usage) => self.charge(run, budget, plan, usage, sent.worst),
            None => (TokenUsage::default(), Cost::ZERO),
        };
        let mut record = CallRecord {
            attempt: plan.attempt,
            rung: plan.rung,
            model: plan.model.clone(),
            effort: plan.effort,
            picture: plan.picture,
            kind: plan.kind,
            prompt_version: prompt_version(),
            manifest: Some(sent.manifest),
            usage,
            cost,
            elapsed: sent.elapsed,
            request_id: None,
            label: CallLabel::Rejected,
        };
        let outcome = match &error {
            ProviderError::Truncated { .. } => {
                record.label = CallLabel::Truncated;
                Outcome::Truncated
            }
            ProviderError::InvalidOutput { .. } => {
                record.label = CallLabel::InvalidOutput;
                Outcome::InvalidOutput
            }
            ProviderError::Refused { .. } => {
                record.label = CallLabel::Refused;
                Outcome::Refused
            }
            ProviderError::Rejected(_) => Outcome::Rejected,
            ProviderError::ModelUnavailable => {
                record.label = CallLabel::RungUnavailable;
                // (A rung is left behind once it is unavailable, so this is never a repeat.)
                run.unavailable.push(plan.model.clone());
                Outcome::RungUnavailable
            }
            // The service failed: no answer, no attempt used. The page can resume.
            ProviderError::Unavailable { .. }
            | ProviderError::Timeout
            | ProviderError::Auth
            | ProviderError::KeyProblem { .. } => {
                let failure = error.failure_kind();
                record.label = CallLabel::Interrupted(failure);
                // A refusal before any work (overloaded, bad key, billing) is not billed. A
                // timeout may have been processed, and the provider cannot say, so the budget
                // is charged the most the call could have cost: a loop of timeouts cannot
                // spend past the cap unseen.
                if matches!(error, ProviderError::Timeout) {
                    Self::book(run, budget, sent.worst);
                    record.cost = sent.worst;
                }
                run.calls.push(record);
                return Err(Trouble {
                    failure,
                    message: error.to_string(),
                    retry_after: error.retry_after(),
                    key_rejected: error.key_rejected(),
                });
            }
        };
        run.calls.push(record);
        Attempted::just(outcome)
    }

    /// Works out what a billed call cost and adds it to the budget and the run.
    fn charge(
        &self,
        run: &mut Run,
        budget: &mut BudgetCheck,
        plan: &CallPlan,
        usage: Usage,
        worst: Cost,
    ) -> (TokenUsage, Cost) {
        let usage = token_usage(usage);
        // A model that was priced a moment ago still is; if it somehow were not, the budget is
        // charged the most the call could have cost rather than nothing.
        let cost = self
            .config
            .prices
            .cost(&plan.model, &usage, self.config.rate)
            .unwrap_or(worst);
        Self::book(run, budget, cost);
        (usage, cost)
    }

    /// Adds an amount to the document's budget and to this run's spending.
    fn book(run: &mut Run, budget: &mut BudgetCheck, cost: Cost) {
        budget.record(cost);
        run.spent = run.spent.saturating_add(cost);
    }

    /// The page's text as it will be sent and how far it can be believed.
    async fn fetch_material(&self, task: &PageTask) -> Result<Material, Trouble> {
        let (content, trust) = match (task.route, task.text_source) {
            // A text layer that passed its checks: the words are needed to paint over data.
            (ReadRoute::TextAndImage, PageTextSource::TextLayer) => {
                (Some(self.layer(task, true).await?), TextTrust::Trusted)
            }
            // Only a hint: its words would not be painted from.
            (ReadRoute::ImageFirst, PageTextSource::TextLayer) => {
                (Some(self.layer(task, false).await?), TextTrust::Unverified)
            }
            (ReadRoute::ImageOnly, PageTextSource::Ocr) => {
                let page = self
                    .worker
                    .ocr_page(task.file.clone(), task.page, false)
                    .await
                    .map_err(Trouble::worker)?;
                if page.page() != task.page || page.source() != PageSource::Ocr {
                    return Err(Trouble::worker_said(
                        "the document worker answered for another page",
                    ));
                }
                (Some(page), TextTrust::Unverified)
            }
            _ => (None, TextTrust::Unverified),
        };
        // A page with more text than is ever sent is read from its picture, never cut.
        let text = content
            .as_ref()
            .filter(|page| page.text().len() <= MAX_PAGE_TEXT_BYTES)
            .and_then(|page| PageText::for_route(task.route, task.text_source, page.text().to_string()));
        let content = if text.is_some() { content } else { None };
        Ok(Material { text, content, trust })
    }

    async fn layer(&self, task: &PageTask, words: bool) -> Result<PageContent, Trouble> {
        let mut pages = self
            .worker
            .page_texts(task.file.clone(), PageRange::single(task.page), words)
            .await
            .map_err(Trouble::worker)?;
        match (pages.pop(), pages.is_empty()) {
            (Some(page), true)
                if page.page() == task.page
                    && matches!(page.source(), PageSource::TextLayer | PageSource::None) =>
            {
                Ok(page)
            }
            _ => Err(Trouble::worker_said(
                "the document worker answered for another page",
            )),
        }
    }

    /// The picture of the page at the size the rung wants, drawn once per size.
    async fn picture(
        &self,
        run: &mut Run,
        task: &PageTask,
        picture: Picture,
    ) -> Result<RenderedPage, Trouble> {
        if let Some((_, rendered)) = run.pictures.iter().find(|(drawn, _)| *drawn == picture) {
            return Ok(rendered.clone());
        }
        let options =
            RenderOptions::new(picture.dpi, ImageFormat::Png, picture.max_edge).map_err(|_| Trouble {
                failure: FailureKind::NeedsReview,
                message: "a rung asks for a picture size the worker does not draw".to_string(),
                retry_after: None,
                key_rejected: false,
            })?;
        let rendered = self
            .worker
            .render_page(task.file.clone(), task.page, options)
            .await
            .map_err(Trouble::worker)?;
        run.pictures.push((picture, rendered.clone()));
        Ok(rendered)
    }

    /// Builds the result of a page that has ended: the reading that counts, stamped with the
    /// confidence its evidence gives.
    fn finish(&self, run: Run, task: &PageTask, outcome: PageOutcome) -> PageResult {
        let (kept, basis) = match &outcome {
            PageOutcome::Read { confidence, .. } => (run.last.clone(), *confidence),
            PageOutcome::NeedsReview {
                draft: Some(attempt), ..
            } => (
                [&run.best, &run.last]
                    .into_iter()
                    .flatten()
                    .find(|kept| kept.attempt == *attempt)
                    .cloned(),
                ConfidenceBasis::Unresolved,
            ),
            PageOutcome::NeedsReview { draft: None, .. } => (None, ConfidenceBasis::Unresolved),
        };
        let evaluated = match (&kept, &run.material) {
            (Some(kept), Some(material)) => serde_json::from_value::<PageReading>(kept.reading.clone())
                .ok()
                .and_then(|reading| evaluate(task, material, &reading, &kept.model, basis).ok()),
            _ => None,
        };
        let (reading, reading_from) = match (evaluated.as_ref(), kept) {
            (Some(_), Some(kept)) => (
                Some(kept.reading),
                Some(ReadingOrigin {
                    rung: kept.rung,
                    attempt: kept.attempt,
                    model: kept.model,
                }),
            ),
            _ => (None, None),
        };
        let (extraction, problems, findings, unconfirmed) = match evaluated {
            Some(e) => (Some(e.extraction), e.problems, e.findings, e.report.unconfirmed),
            None => (None, Vec::new(), Vec::new(), Vec::new()),
        };
        let mut injection = run.injection;
        injection.sort_unstable();
        PageResult {
            outcome,
            extraction,
            reading,
            reading_from,
            problems,
            findings,
            unconfirmed,
            calls: run.calls,
            spent: run.spent,
            unavailable_models: run.unavailable,
            budget_stop: run.budget_stop,
            injection_suspected: injection,
        }
    }
}
