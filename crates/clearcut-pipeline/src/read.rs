// Written by Kone Tshivhinda & Claude
//! The read stage (ADR-0018 step 7): the pages of a document are read by the model router,
//! one after the other, and what comes of each is stored.
//!
//! ```text
//! for each page, in order:
//!   already ended?  -> skip it (but learn from it what it established)
//!   stopped midway? -> resume it from its stored carry
//!   router.read_page(task, budget, calls left, carry)
//!     finished    -> log the calls, store the reading (or the best draft), learn from it
//!     interrupted -> log the calls, store the carry, stop the job (the queue retries later)
//! then the audit, the checkpoint, and the end of the job
//! ```
//!
//! **Money.** A document's spending so far is read from the call log (`model_calls`) when the
//! stage starts, so it is the same after a restart: the budget a resumed job sees is the one
//! the first run left. The month's spending is the tenant's calls since the start of the
//! calendar month in the tenant's time zone. Calls are logged **before** the page is stored:
//! if the second fails, the page is read again later and the first read is still on the
//! books. Known limits, recorded in ADR-0018: two documents running at once do not see each
//! other's spending until it is logged (P2-12's `SpendGate` closes that), and the calls of a
//! page whose job dies in the middle of it are not logged (a hook per call in the router
//! would close that).
//!
//! **What the next page is told.** Only what a page that was *read* established (its
//! currency, invoice number, supplier, whether totals were seen, how many lines): a draft
//! from a page that went to a person is not offered to the next page as fact.
//!
//! **How it ends.** Until the stages after reading exist the job still ends `needs_review`,
//! and says so: `read; the later stages are not built yet`, or how many pages need a person.

use crate::call_log::to_new_model_call;
use crate::month::month_start;
use crate::{needs_review, report_failure, store_failure, transient, Models, PipelineHandler, Step, Stop};
use bytes::Bytes;
use clearcut_core::assemble::assemble;
use clearcut_core::cost::{BudgetCheck, Cost};
use clearcut_core::escalation::{ConfidenceBasis, PageOutcome};
use clearcut_core::event::Confidence;
use clearcut_core::extraction::{derive_events, Extraction};
use clearcut_core::jobs::Progress;
use clearcut_core::page_read::{PageRead, ROUTING_VERSION};
use clearcut_core::reading::{ConversionContext, PageReading};
use clearcut_docproc::DocumentProcessor;
use clearcut_jobs::{Failure, Finished, JobContext};
use clearcut_objects::ObjectStore;
use clearcut_pack::DocumentContext;
use clearcut_provider::ModelProvider;
use clearcut_router::{Carry, PageResult, PageRun, PageTask, Router};
use clearcut_store::page_readings::{KeptReading, PageDetails, PageReading as StoredPage, PageState};
use clearcut_store::{AuditEntry, DocId, Job, TenantId};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::num::NonZeroU32;
use time::OffsetDateTime;

/// The name saved in the checkpoint once every page has ended.
pub const READ_STAGE: &str = "read";

/// What earlier pages of the document established.
#[derive(Default)]
struct Established {
    context: DocumentContext,
}

impl Established {
    /// Learns from a page that was read.
    fn absorb(&mut self, extraction: &Extraction) {
        let context = &mut self.context;
        context.currency.get_or_insert(extraction.currency);
        if context.invoice_number.is_none() {
            context.invoice_number = extraction
                .invoice_number
                .as_ref()
                .map(|number| number.value.clone());
        }
        if context.supplier_name.is_none() {
            context.supplier_name = extraction
                .supplier
                .as_ref()
                .map(|supplier| supplier.value.name.clone());
        }
        context.totals_seen |= extraction.stated_totals.is_some();
        let lines = u32::try_from(extraction.lines.len()).unwrap_or(u32::MAX);
        context.lines_read = context.lines_read.saturating_add(lines);
    }
}

/// How the stage went through the pages, for the audit and the end of the job.
#[derive(Default)]
struct Tally {
    read: u32,
    review: u32,
    review_reasons: BTreeMap<&'static str, u32>,
    rungs: BTreeMap<usize, u32>,
}

impl Tally {
    fn add(&mut self, state: &PageState) {
        match state {
            PageState::Read(kept) => {
                self.read += 1;
                *self.rungs.entry(usize::from(kept.rung)).or_default() += 1;
            }
            PageState::NeedsReview { reason, .. } => {
                self.review += 1;
                *self.review_reasons.entry(reason.as_str()).or_default() += 1;
            }
            PageState::InProgress { .. } => {}
        }
    }
}

impl<O: ObjectStore + 'static, P: DocumentProcessor, M: ModelProvider> PipelineHandler<O, P, M> {
    /// Reads every page of the analysed document with the model router.
    pub(crate) async fn read_pages(
        &self,
        ctx: &JobContext,
        job: &Job,
        bytes: &Bytes,
        models: &Models<M>,
    ) -> Step<Finished> {
        let (tenant, document) = (job.tenant, job.document);
        let reads = self
            .store
            .load_page_reads(tenant, document)
            .await
            .map_err(|e| store_failure(&e))?;
        let pages = u32::try_from(reads.len()).unwrap_or(u32::MAX);
        if pages == 0 {
            return Err(needs_review("the document has no pages"));
        }
        let stored = self
            .store
            .load_page_readings(tenant, document)
            .await
            .map_err(|e| store_failure(&e))?;

        let mut established = Established::default();
        self.replay(document, &reads, &stored, &mut established);

        let mut budget = self.budget_for(tenant, document, pages, models).await?;
        let mut calls_left = self.calls_left(tenant, document, pages, models).await?;
        let mut unavailable: Vec<String> = Vec::new();
        self.progress(ctx, 0, pages, "reading the pages").await?;

        for (index, read) in reads.iter().enumerate() {
            let page = read.page();
            let existing = stored.iter().find(|stored| stored.page == page);
            if existing.is_some_and(|stored| !matches!(stored.state, PageState::InProgress { .. })) {
                continue;
            }
            let carry = existing.and_then(|stored| match &stored.state {
                PageState::InProgress { carry } => serde_json::from_value::<Carry>(carry.clone()).ok(),
                _ => None,
            });
            let task = self.task(job, bytes, read, pages, &established)?;
            let mut config = models.settings.router.clone();
            if !unavailable.is_empty() {
                let names: Vec<&str> = unavailable.iter().map(String::as_str).collect();
                config.policy = config.policy.without_models(&names);
            }
            let router = Router::new(models.provider.clone(), self.worker.clone(), config);
            match router.read_page(&task, &mut budget, calls_left, carry).await {
                PageRun::Finished(result) => {
                    let state = self.save_finished(tenant, document, job, page, &result).await?;
                    if let (PageState::Read(_), Some(extraction)) = (&state, &result.extraction) {
                        established.absorb(extraction);
                    }
                    calls_left = calls_left.saturating_sub(counted_calls(&result));
                    for model in &result.unavailable_models {
                        if !unavailable.contains(model) {
                            unavailable.push(model.clone());
                        }
                    }
                }
                PageRun::Interrupted(interruption) => {
                    let calls: Vec<_> = interruption
                        .calls
                        .iter()
                        .map(|record| to_new_model_call(Some(job.id), document, page, record))
                        .collect();
                    self.store
                        .record_model_calls(tenant, &calls)
                        .await
                        .map_err(|e| store_failure(&e))?;
                    // Keeps the page where it stopped. If the carry cannot be stored the page
                    // is simply read again from the start next time.
                    if let Ok(carry) = serde_json::to_value(&interruption.carry) {
                        let saved = self
                            .store
                            .save_page_reading(
                                tenant,
                                document,
                                page,
                                &PageState::InProgress { carry },
                                &PageDetails::default(),
                            )
                            .await;
                        if let Err(error) = saved {
                            tracing::warn!(%error, "the carry of an interrupted page could not be stored");
                        }
                    }
                    return Err(Stop::Failed(Failure {
                        kind: interruption.failure,
                        message: interruption.message,
                    }));
                }
            }
            let done = u32::try_from(index + 1).unwrap_or(pages);
            self.progress(ctx, done, pages, "reading the pages").await?;
        }

        self.finish_reading(ctx, tenant, document, pages).await
    }

    /// Learns from the pages that ended in an earlier run, in page order.
    fn replay(
        &self,
        document: DocId,
        reads: &[PageRead],
        stored: &[StoredPage],
        established: &mut Established,
    ) {
        for read in reads {
            let Some(page) = stored.iter().find(|stored| stored.page == read.page()) else {
                continue;
            };
            if let PageState::Read(kept) = &page.state {
                if let Some(extraction) = convert_stored(document, read.page(), kept, established) {
                    established.absorb(&extraction);
                }
            }
        }
    }

    /// The document's budget as it stands: its cap, what the month has spent and what this
    /// document has spent already.
    async fn budget_for(
        &self,
        tenant: TenantId,
        document: DocId,
        pages: u32,
        models: &Models<M>,
    ) -> Step<BudgetCheck> {
        let settings = &models.settings;
        let since = month_start(OffsetDateTime::now_utc(), settings.month_offset);
        let month = self
            .store
            .spend_since(tenant, since)
            .await
            .map_err(|e| store_failure(&e))?;
        let document_spend = self
            .store
            .document_spend(tenant, document)
            .await
            .map_err(|e| store_failure(&e))?;
        // The month's total already holds this document's earlier calls, which the check adds
        // again as the document's own: take them out of the month first. (Calls of a month
        // ago would make this take out too much, which only makes the check stricter.)
        let mut budget = BudgetCheck::new(&settings.budget, pages, month.saturating_sub(document_spend.cost));
        budget.record(document_spend.cost);
        Ok(budget)
    }

    /// How many calls the document may still make.
    async fn calls_left(
        &self,
        tenant: TenantId,
        document: DocId,
        pages: u32,
        models: &Models<M>,
    ) -> Step<u32> {
        let used = self
            .store
            .document_spend(tenant, document)
            .await
            .map_err(|e| store_failure(&e))?
            .calls;
        Ok(models
            .settings
            .router
            .policy
            .document_call_cap(pages)
            .saturating_sub(used))
    }

    fn task(
        &self,
        job: &Job,
        bytes: &Bytes,
        read: &PageRead,
        pages: u32,
        established: &Established,
    ) -> Step<PageTask> {
        let number = |value: u32| NonZeroU32::new(value).ok_or_else(|| transient("a page number was zero"));
        Ok(PageTask {
            tenant: job.tenant.0,
            job: Some(job.id),
            document_uuid: job.document.0,
            document: job.document.as_core_id(),
            file: bytes.clone(),
            page: number(read.page())?,
            of: number(pages)?,
            route: read.route(),
            text_source: read.text_source(),
            context: established.context.clone(),
            currency_known: established.context.currency,
        })
    }

    /// Logs a finished page's calls, then stores how it ended. Returns what was stored.
    async fn save_finished(
        &self,
        tenant: TenantId,
        document: DocId,
        job: &Job,
        page: u32,
        result: &PageResult,
    ) -> Step<PageState> {
        let calls: Vec<_> = result
            .calls
            .iter()
            .map(|record| to_new_model_call(Some(job.id), document, page, record))
            .collect();
        self.store
            .record_model_calls(tenant, &calls)
            .await
            .map_err(|e| store_failure(&e))?;

        let kept = |basis: ConfidenceBasis| -> Option<KeptReading> {
            let (reading, origin) = (result.reading.clone()?, result.reading_from.clone()?);
            Some(KeptReading {
                rung: u8::try_from(origin.rung).unwrap_or(u8::MAX),
                attempt: origin.attempt,
                confidence: basis,
                model: origin.model,
                reading,
            })
        };
        let state = match &result.outcome {
            PageOutcome::Read { confidence, .. } => match kept(*confidence) {
                Some(kept) => PageState::Read(kept),
                None => return Err(needs_review("the router's result for a page was incomplete")),
            },
            PageOutcome::NeedsReview { reason, .. } => PageState::NeedsReview {
                reason: *reason,
                draft: kept(ConfidenceBasis::Unresolved),
            },
        };
        let notes = match &result.outcome {
            PageOutcome::Read { notes, .. } => serde_json::to_value(notes),
            PageOutcome::NeedsReview { .. } => Ok(json!([])),
        };
        let details = PageDetails {
            problems: serde_json::to_value(&result.problems).unwrap_or_else(|_| json!([])),
            findings: serde_json::to_value(&result.findings).unwrap_or_else(|_| json!([])),
            notes: notes.unwrap_or_else(|_| json!([])),
            unconfirmed: result.unconfirmed.clone(),
            injection_suspected: result
                .injection_suspected
                .iter()
                .map(|kind| (*kind).to_string())
                .collect(),
        };
        self.store
            .save_page_reading(tenant, document, page, &state, &details)
            .await
            .map_err(|e| store_failure(&e))?;
        Ok(state)
    }

    /// Audits the reading, saves the checkpoint and ends the job.
    async fn finish_reading(
        &self,
        ctx: &JobContext,
        tenant: TenantId,
        document: DocId,
        pages: u32,
    ) -> Step<Finished> {
        // Every page has ended; this run may have ended some of them, so count them all again.
        let ended = self
            .store
            .load_page_readings(tenant, document)
            .await
            .map_err(|e| store_failure(&e))?;
        let mut all = Tally::default();
        for page in &ended {
            all.add(&page.state);
        }
        if u32::try_from(ended.len()) != Ok(pages) || all.read + all.review != pages {
            return Err(transient("not every page has ended"));
        }

        // A document whose pages were all read is put together, checked and turned into events.
        let completion = if all.review == 0 {
            Some(self.complete(tenant, document, &ended).await?)
        } else {
            None
        };

        let first_end = ctx
            .checkpoint()
            .and_then(|saved| (saved.get("stage").and_then(Value::as_str) == Some(READ_STAGE)).then_some(()));
        if first_end.is_none() {
            let spend = self
                .store
                .document_spend(tenant, document)
                .await
                .map_err(|e| store_failure(&e))?;
            self.store
                .record_audit(
                    tenant,
                    &AuditEntry {
                        actor: json!({ "type": "system" }),
                        action: "document.read".to_string(),
                        subject_type: "document".to_string(),
                        subject_id: document.0.to_string(),
                        detail: audit_detail(pages, &all, spend.calls, spend.cost, completion.as_ref()),
                    },
                )
                .await
                .map_err(|e| store_failure(&e))?;
            let done =
                Progress::new(pages, Some(pages)).map_err(|_| transient("progress was out of range"))?;
            let checkpoint = json!({ "stage": READ_STAGE, "pages": pages, "rules": ROUTING_VERSION });
            ctx.report(done, Some("read"), Some(checkpoint))
                .await
                .map_err(|e| report_failure(&e))?;
        }
        Ok(match completion {
            None => Finished::NeedsReview(format!("{} of {} pages need review", all.review, pages)),
            Some(completion) if completion.is_done() => Finished::Succeeded,
            Some(completion) => Finished::NeedsReview(completion.reason()),
        })
    }

    /// Puts the pages of a document that was read together, checks the whole, and stores its
    /// events. The events are stored even when the checks found something (the events say
    /// what), and the job then ends for review instead of done.
    async fn complete(&self, tenant: TenantId, document: DocId, ended: &[StoredPage]) -> Step<Completion> {
        let mut established = Established::default();
        let mut extractions = Vec::new();
        for page in ended {
            let PageState::Read(kept) = &page.state else {
                return Err(transient("a page that was read is missing"));
            };
            let extraction = convert_stored(document, page.page, kept, &established)
                .ok_or_else(|| needs_review("a stored reading could not be read back"))?;
            established.absorb(&extraction);
            extractions.push(extraction);
        }
        let assembled =
            assemble(&extractions).map_err(|_| needs_review("the pages could not be put together"))?;
        let earlier = self
            .store
            .load_events(tenant, document)
            .await
            .map_err(|e| store_failure(&e))?;
        let derived = derive_events(&assembled.extraction, NonZeroU32::MIN, &earlier)
            .map_err(|_| needs_review("events could not be made from the reading"))?;
        match self.store.append_events(tenant, document, &derived.events).await {
            Ok(()) => {}
            Err(clearcut_store::Error::EventIdConflict(_)) => {
                return Err(needs_review(
                    "events from an earlier reading of this document already exist",
                ));
            }
            Err(error) => return Err(store_failure(&error)),
        }
        Ok(Completion {
            events: derived.events.len(),
            blocking: derived
                .validation
                .findings
                .iter()
                .filter(|f| f.is_blocking())
                .count(),
            conflicts: assembled.conflicts.iter().map(|c| c.as_str()).collect(),
        })
    }
}

/// What came of putting a document's pages together.
struct Completion {
    events: usize,
    /// How many of the checks' findings stop the document being called done.
    blocking: usize,
    /// What the pages printed differently, by fixed name.
    conflicts: Vec<&'static str>,
}

impl Completion {
    fn is_done(&self) -> bool {
        self.blocking == 0 && self.conflicts.is_empty()
    }

    /// Why the document goes to a person. Counts and fixed names only.
    fn reason(&self) -> String {
        if self.conflicts.is_empty() {
            format!(
                "the checks found {} problem(s) in the whole document; the events say which",
                self.blocking
            )
        } else {
            format!("the pages disagree about: {}", self.conflicts.join(", "))
        }
    }
}

/// Counts and fixed names only: nothing from the document.
fn audit_detail(pages: u32, tally: &Tally, calls: u32, cost: Cost, completion: Option<&Completion>) -> Value {
    let reasons: BTreeMap<&str, u32> = tally.review_reasons.clone();
    let rungs: BTreeMap<String, u32> = tally
        .rungs
        .iter()
        .map(|(rung, count)| (rung.to_string(), *count))
        .collect();
    json!({
        "pages": pages,
        "read": tally.read,
        "needs_review": tally.review,
        "review_reasons": reasons,
        "calls": calls,
        "cost_rand_cents": cost.rand_cents_ceil(),
        "rungs": rungs,
        "events": completion.map_or(0, |c| c.events),
        "blocking_findings": completion.map_or(0, |c| c.blocking),
        "conflicts": completion.map(|c| c.conflicts.clone()).unwrap_or_default(),
        "outcome": match completion {
            Some(c) if c.is_done() => "done",
            _ => "needs_review",
        },
        "rules": ROUTING_VERSION,
    })
}

/// The calls of a page that count against the document's allowance: the ones the state machine
/// counted (not one refused before it was sent, nor one the service did not answer).
fn counted_calls(result: &PageResult) -> u32 {
    use clearcut_router::CallLabel;
    let counted = result
        .calls
        .iter()
        .filter(|record| !matches!(record.label, CallLabel::NotSent(_) | CallLabel::Interrupted(_)))
        .count();
    u32::try_from(counted).unwrap_or(u32::MAX)
}

/// A stored reading converted again, to learn what it established.
fn convert_stored(
    document: DocId,
    page: u32,
    kept: &KeptReading,
    established: &Established,
) -> Option<Extraction> {
    let reading: PageReading = serde_json::from_value(kept.reading.clone()).ok()?;
    let context = ConversionContext {
        document: document.as_core_id(),
        page: NonZeroU32::new(page)?,
        extractor: clearcut_core::event::Extractor::Model {
            name: kept.model.clone(),
            prompt_version: clearcut_pack::prompt_version(),
        },
        confidence: Confidence::new(kept.confidence.per_mille()).ok()?,
        currency_known: established.context.currency,
    };
    reading
        .into_extraction(&context)
        .ok()
        .map(|conversion| conversion.extraction)
}
