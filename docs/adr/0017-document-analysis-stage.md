# ADR-0017: The document analysis stage (first stage of the pipeline handler)

Status: **Proposed** (waiting for Kone Tshivhinda's answers below) · Date: 2026-10-24 · Deciders: Kone Tshivhinda, Claude
Task P2-02 (document intake analysis), and the first piece of the job code that P2-05 left open: the decision of when to run OCR and what to do with the `text_quality` verdict, which ADR-0016 (section 4 and the step 6 notes) hands to the pipeline. Builds on [ADR-0004](0004-ingestion-and-narration-pipeline.md), [ADR-0008](0008-job-queue.md) (the queue and its handler), [ADR-0009](0009-worker-and-provider-boundary.md) and [ADR-0016](0016-page-text-images-and-text-quality.md).

## What it is
Everything the worker and `clearcut-core` can now do (inspect a file, read each page's text, OCR a page, judge whether a text layer can be trusted) still has no caller. This ADR designs the caller for the **first stage**: given a stored document, find out *how each page should be read*, and record it. No model is called, so it needs no key and costs nothing to run.

```text
job process_document ─► stage 1: ANALYSE ─► (later stages: classify, extract, validate, events ...)
                         read the file from storage
                         worker.inspect        → kind, page count (documents.page_count)
                         worker.page_texts     → text and fonts of every page, in chunks of 50
                         core::text_quality    → Good / NeedsCheck / Unusable per page
                         worker.ocr_page       → OCR agreement for page 1 and every NeedsCheck page
                         core::text_quality    → the final verdict per page
                         core routing rule     → how each page will be read
                         ► one row per page, written in one transaction
```

## What already exists, so it is not redone
- **De-duplication by hash is done at intake** (`unique (tenant_id, sha256)`, and `AddedDocument.duplicate`). P2-02's "de-duplication by hash" needs nothing more. Near-duplicates (the same invoice scanned twice) are the content-level duplicate detection of P2-08 and need extracted fields, so they come after extraction.
- **Type sniffing** is done at intake (the media type from the bytes) and again by the worker's own reading.
- `documents.page_count` exists and nothing sets it; this stage does.

## Decisions to confirm

**1. Where the code lives: a new crate `clearcut-pipeline`.** (Recommended.) It implements `clearcut_jobs::Handler` for the `process_document` job kind and takes its collaborators as traits: the store, the object store and `DocumentProcessor`. Tests use `FakeWorker` and the test database, so nothing needs the real worker. `clearcut-core` keeps all the rules (`text_quality` and the routing rule below); the crate only fetches, calls and records. *Alternative:* put it in `clearcut-jobs`. Rejected: that crate is the generic runner and should not know about documents.

**2. The routing rule, in `clearcut-core` (pure and tested).** From the final verdict and the kind of page, each page gets one **reading route**:

| Page | Route | Meaning for later stages |
|---|---|---|
| Text layer, `Good` | `text_and_image` | the model gets the page picture and the text layer (the normal case, ADR-0014) |
| Text layer, `NeedsCheck`, OCR agrees (850 or more) | `text_and_image` | as above |
| Text layer, `NeedsCheck`, OCR not run or below 850 | `image_first` | the picture is the source; the text goes along only as a labelled hint |
| Text layer, `Unusable` | `image_only` | the text layer is rejected and recorded as `text_layer_rejected`; the model reads the picture, and the OCR text (if it was run) is the hint |
| No text layer, or a PNG/JPEG | `image_only` | the OCR text (if the worker has OCR) is the hint |

This is the "never trust one path" rule of P2-20: a page whose text and picture disagree is never read from the text alone. The thresholds are the ones already in `text_quality` (provisional until the gold set exists).

**3. When OCR runs.** Page 1 of every PDF (a sentinel: if the first page agrees with its text layer, the producer is probably honest, and if not we learn it for the price of one OCR), every page whose text-only verdict is `NeedsCheck`, and every page of a picture. Not every page of every PDF, because OCR is about a second a page. If the worker has no OCR (`NotAvailable`), nothing fails: the pages that needed it are routed `image_first` or `image_only` and the analysis records `ocr_unavailable`. *Proposed for the case where page 1 disagrees:* OCR the whole document, up to a cap (default 50 pages), and route the pages beyond the cap `image_first`. The cap is a setting.

**4. Where the result is kept: a new table `page_reads`.** (Recommended. This is a schema change, so I will show you the SQL before applying it, as agreed.) One row per document page: `tenant_id`, `document_id`, `page`, `text_source` (`text_layer`, `ocr` or `none`), `verdict`, `reasons text[]`, `ocr_agreement smallint null`, `route`, `analysed_with text` (the version of the rules, like `reading-v2`), `created_at`. Row-level security like the other tables; written by the job, read by later stages, the review screen and the audit. No text and no word of the document is in it, only facts about it. *Alternatives:* (a) only in the job checkpoint, which is lost when the job ends and cannot be queried; (b) as events, which are for facts *about the invoice* and would mix processing metadata into the source of truth. A re-run replaces the rows: the stage is idempotent (same file, same worker, same rows).

**5. Page text is not stored.** The worker is stateless and cheap to ask again, so each later stage that needs a page's text asks for it, and no second copy of possibly personal text sits in the database or the object store. The cost is one more worker call per stage. *Alternative:* store the text beside the file. Faster, but worse for POPIA (more copies, and a retention rule to write); I would only change this if a measured cost shows it matters.

**6. The job is checkpointed per stage.** After the analysis the handler saves a small checkpoint (`stage: "analysed"`, the page count and the rule version) through `JobContext::report`, so a retry, or a second worker that picks the job up after a lost lease, starts after it. A document that is encrypted, damaged or over the limits ends the job `needs_review` with the worker's problem as the reason (the existing `DocumentProblem` to failure kind mapping); a worker that is down is `ProviderUnavailable` (retry later).

**7. The document's status.** `received` becomes `processing` when the stage starts. It never becomes `done` here; later stages do that.

## Tests (no key, no real invoice)
- `FakeWorker` scripts for: a clean text PDF (all `Good`, OCR only on page 1, route `text_and_image`); a page of replacement characters (`Unusable`, so `image_only` and `text_layer_rejected`); a risky font with weak text and an OCR that agrees, and one that disagrees; a page-1 disagreement that triggers the whole-document OCR up to the cap; a worker without OCR; a picture; an encrypted file; a worker error half-way (nothing half-written: the rows are written in one transaction at the end); a 120-page document read in chunks of 50.
- The same job run twice gives the same rows; a cancelled job writes nothing.
- The routing rule: a table test over every combination in decision 2, in `clearcut-core`.
- One live test against the real worker in the CI contract job, with a generated scrambled-font PDF (ground truth by construction, as in ADR-0016 section 7).
- A hand mutation check as for the other crates.

## Build order (after approval)
1. The routing rule and the `page_reads` record type in `clearcut-core`. 2. The migration for `page_reads` and its store functions (shown to you first). 3. `clearcut-pipeline` with the analyse stage and the handler. 4. The live test and docs; wiring into the server's job runner waits for P1-07, when the runner becomes its own process.

## Questions for Kone
1. A new crate `clearcut-pipeline` for the handler (decision 1)?
2. The routing table (decision 2): is it right that `NeedsCheck` with no OCR means "picture first, text as a labelled hint" rather than refusing the page?
3. OCR on page 1 of every PDF as a sentinel, plus whole-document OCR (capped at 50 pages) when it disagrees (decision 3)?
4. A `page_reads` table (decision 4), and page text not stored (decision 5)?
