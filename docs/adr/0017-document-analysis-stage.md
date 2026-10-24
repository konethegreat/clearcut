# ADR-0017: The document analysis stage (first stage of the pipeline handler)

Status: **Accepted** (approved by Kone Tshivhinda, 2026-10-24, with all four recommended answers below) · Date: 2026-10-24 · Deciders: Kone Tshivhinda, Claude
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

## Implementation notes

**Step 1, the routing rule and the page record in `clearcut-core`** (`page_read`):
1. **Names that are stored.** `ReadRoute` (`text_and_image`, `image_first`, `image_only`) and `PageTextSource` (`text_layer`, `ocr`, `none`) have fixed names with `ALL`, `as_str` and an exact `parse`; `TextVerdict` and `Reason` in `text_quality` gained `ALL` and `parse` for the same reason (a stored row is read back and checked like any other input).
2. **One verdict, one route.** `route_for`: `Good` is `text_and_image`, `NeedsCheck` is `image_first`, `Unusable` is `image_only`. The row "text layer, `NeedsCheck`, OCR agrees (850 or more)" of the table in decision 2 needs no rule of its own: `text_quality` already makes an agreeing page `Good`, so by the time the route is chosen it is in the first row.
3. **`PageRead::decide`** takes a page's text, fonts and what OCR found (`OcrRun { agreement }`, `None` if OCR was not run) and returns the page, the text source, the verdict and its reasons, the agreement (clamped to 1000), the route and the rules version `routing-v1`. The text source is the layer for `Good` and `NeedsCheck`, and for `Unusable` it is `ocr` if OCR was run and `none` if not. `text_layer_rejected()` is true for `Unusable` for any reason other than `no_text`: a scan never had a layer to reject. No text is in the record, and its `Debug` shows none.
4. **`PageRead::from_stored`** reads a row back (`StoredPageRead`, plain strings and numbers) and refuses one that does not follow from the rules: page 0, an unknown name, a route that is not the verdict's route, a text source that does not fit the verdict (a layer for an `Unusable` page, OCR or none for a `Good` one), an agreement over 1000, an empty or over-long (64) rules version, a repeated reason. A database that has been edited by hand cannot hand a stage a page that contradicts the rule.
5. **Which pages OCR reads, as pure functions.** `first_ocr_round(verdicts, cap)`: page 1 and every page whose text-only verdict is not `Good`, in page order, at most `cap`. `second_ocr_round(verdicts, first, page_one_agreement, cap)`: if page 1's agreement is below 850 it is the pages not yet read, in order, until `cap` pages in all; a page 1 that agrees, or has no text layer to compare (no agreement), adds nothing. `DEFAULT_OCR_PAGE_CAP` is 50.
   **Two points where the ADR text left room, settled as follows (tell me if you want them the other way):** (a) the cap applies to the first round too, not only to the whole-document round. A 300-page scan would otherwise be 300 seconds of OCR in one job; pages beyond the cap are `image_only` without an OCR hint, which is the safe route anyway. (b) Every page whose text layer is not `Good` is read in the first round, which includes scanned pages with no text layer at all, because the table says their OCR text is the hint; decision 3 named only page 1, doubtful pages and pictures, and a scanned PDF page is the same as a picture.
6. **Tests.** 32 new in core (410 in all). Mutation check of 41 breakages by hand: the route and source of each verdict, the clamp and its edge, the agreement and version edges (1000, 64), each check in `from_stored`, the rejected rule, page 1 in the first round, the cap in each round, skipping pages already read, and the 850 edge in the second round; all caught. The ones that did not compile or did not match were no-op shapes and are not counted.
7. **Step 2, the `page_reads` table and its store functions** (migration `0006_page_reads.sql`, shown to Kone and approved before it was applied):
   - **The table** is as decision 4 says: `tenant_id`, `document_id`, `page`, `text_source`, `verdict`, `reasons text[]`, `ocr_agreement smallint`, `route`, `analysed_with`, `created_at`; primary key `(document_id, page)`; a composite foreign key to `documents (tenant_id, id)`. The database repeats what is cheap to state, so a contradictory row cannot be stored even by a bug: the vocabulary of every column, the agreement between 0 and 1000, at most eight reasons from the fixed list, a rules version of 1 to 64 characters, and one combined check that the route and text source follow from the verdict (good: `text_and_image` + `text_layer`; needs_check: `image_first` + `text_layer`; unusable: `image_only` + `ocr` or `none`). The column checks for the three names are redundant with the combined check and are kept for clearer error messages.
   - **Access.** Row-level security like every other table. The application role may `select`, `insert` and `delete`, and may not `update` or `truncate`: a re-run of the stage deletes and re-inserts a document's rows in one transaction. This is looser than events and the audit log (append-only) because these rows are derived and can always be produced again from the file.
   - **Erasure.** The eraser role gets `select` and `delete` on the table, and `erase_document` is redefined with one more `delete` (the same `create or replace` pattern as migration 0005), so a document with page reads can still be erased for POPIA; a test checks that the rows go and that another tenant's rows stay.
   - **`Store::replace_page_reads(tenant, document, &[PageRead])`** checks that the pages are exactly 1 to n (any order, at most 10 000), then, in one transaction, requires the document, deletes its old rows, inserts the new ones and **sets `documents.page_count`** to n; any failure leaves the old rows as they were. **`Store::load_page_reads`** returns the rows in page order (an empty list if the stage has not run) and runs every row through `PageRead::from_stored`, so a row that passes the database's checks but not the core's (for instance the same reason twice) is `InvalidStoredPageRead` and never reaches a stage.
   - **Tests.** 25 new in the store (161 in all), against a real Postgres as the restricted role: round trip and order, page count, replace, the page-set rules and the 10 000 edge, tenant isolation (a foreign document looks absent; a row for another tenant is refused by the policy; a delete with no `where` clause removes only the caller's rows), every database check, the privileges, erasure. Hand mutation check of about 50 breakages across the Rust and the SQL: every survivor was a missing test (a second row for the same page with different values, a doubtful page with the wrong text source, the delete policy, the 10 000 edge) and is caught now, apart from the tenant filters in two queries, which are equivalent to the row-level security and the unique document id they sit beside, and the three redundant column checks above.
8. **Not done:** the crate and handler (step 3), the live test (step 4).

## Build order (after approval)
1. The routing rule and the `page_reads` record type in `clearcut-core`. 2. The migration for `page_reads` and its store functions (shown to you first). 3. `clearcut-pipeline` with the analyse stage and the handler. 4. The live test and docs; wiring into the server's job runner waits for P1-07, when the runner becomes its own process.

## Questions for Kone (all four answered: yes, as recommended)
1. A new crate `clearcut-pipeline` for the handler (decision 1)?
2. The routing table (decision 2): is it right that `NeedsCheck` with no OCR means "picture first, text as a labelled hint" rather than refusing the page?
3. OCR on page 1 of every PDF as a sentinel, plus whole-document OCR (capped at 50 pages) when it disagrees (decision 3)?
4. A `page_reads` table (decision 4), and page text not stored (decision 5)?
