# Long invoices: reading every page and every part

_Written by Kone Tshivhinda & Claude, 3 November 2026. Proposed, not approved. This expands the reading rules of [ADR-0017](../adr/0017-document-analysis-stage.md), [ADR-0018](../adr/0018-model-router-and-escalation.md) and [ADR-0022](../adr/0022-carried-balances-across-pages.md); their rules on page-local evidence, budgets and deterministic decisions still bind._

## 1. The question

Four questions were asked about a long invoice:

1. Can the system make the API key read it completely, in chunks, until it is finished?
2. Models have a context window and a maximum output. Can the system guarantee that everything is readable?
3. What handles escalation, and is there an orchestrator that makes every part reach the right quality?
4. Are things grouped well?

Short answers:

1. **Today it chunks by page and resumes across restarts, but not within a page.** A page too dense for one answer ends in human review.
2. **The context window is not the limit. The output cap of 16k tokens and the 200-line conversion cap are.** A dense page cannot finish automatically today.
3. **Escalation is deterministic and per page. There is no orchestrator agent, deliberately.** What is missing is a bounded document-level repair round, not a model judge.
4. **Page, document, tenant and register grouping are sound.** Sub-page grouping and multi-invoice files are missing.

The rest of this document is the evidence, the guarantee, and the four slices that close the gap.

## 2. What happens today

### 2.1 The stages and their limits

| Stage | Unit | Limit | At the limit |
|---|---|---|---|
| Upload | one file | 100 MB (`DEFAULT_MAX_DOCUMENT_BYTES`); worker request 64 MB | refused before storage |
| Inspect | one file | 500 pages by default (`CLEARCUT_WORKER_MAX_PAGES`) | `too many pages`, no reading |
| Analyse text | 50 pages per worker call | 1 MB and 20 000 words per page | page refused as too large |
| Analyse OCR | page 1, then every page not `Good` | 50 pages, one second round | pages routed without OCR |
| Read | one page, sequential | 1 to 5 model calls per page; 16k answer; 200 lines | page `needs_review` |
| Assemble | one document | only when **every** page was read | one review page means no assembly and no events |
| Resume | one page | stored carry; finished pages skipped; calls logged | a restarted job continues at the page in progress |

### 2.2 The exact end of a dense page

Take a page with 300 line items.

1. The model's answer reaches its output cap and comes back `Truncated`.
2. The state machine retries on the same rung only if the current cap is below the 16k ceiling. Haiku's cap is 16k, so it does not retry; Sonnet (12k) and Opus (8k) retry once at 16k.
3. A second cut-off, or Haiku's first, ends the page `NeedsReview` with reason `TooLong`.
4. If the answer instead returns 250 lines within the cap, conversion keeps the first 200 and records `TooManyLines`, which the assessment turns into the same `TooLong` review.
5. The page is not `Read`, so `finish_reading` never calls `complete`, so no events are derived and the register shows nothing for the document.

Both paths end in the same place. Nothing splits the page and nothing re-reads it.

### 2.3 Why the context window is not the limit

One call carries one page: a picture of about 3 000 tokens, at most 200 KB of text (about 200 000 tokens), the rules prefix and the output schema. All four models in the ladder have a 1M-token window. Even the worst page in the 200 KB case fits with room to spare. The router already drops text above 200 KB and reads the picture instead, because sending it is expensive and rarely useful, not because it would not fit.

The limits that actually stop a dense page are the output cap (16k), the conversion cap (200 lines), the per-page picture resolution (150 dpi, 2000 px on the lower rungs), and the per-document budget. A 2000 px-tall picture of a 300-line page gives about 6 pixels per line, which is close to unreadable even before the answer cap applies. Splitting the page and rendering each band larger fixes both problems at once.

### 2.4 Escalation today

The ladder is data: Haiku 5.5 (16k, medium, 150 dpi / 2000 px), Sonnet 5.5 (12k, medium), Opus 5.5 (8k, high, 200 dpi / 2576 px), Fable 5.1 (4k, off). Every page starts at rung 1. A reading is accepted only when the schema converts with no critical problem, line arithmetic passes, and the page's own text confirms every amount. Anything else escalates with a named reason. At the top rung, or when a cap or budget is reached, the page goes to a person.

This is the orchestrator, and it is deliberately not a model. A judge would read the page too, be as open to injection as the reader, cost money on every page, and make the outcome non-deterministic. The gap is not the absence of an agent. The gap is that the deterministic orchestrator stops at the page boundary:

- the provider is built without asking the Models API which models the key can use, so a rung the key cannot use is discovered by paying for the call;
- a cut-off answer cannot ask for a larger cap than 16k, even where the model and the budget allow it;
- a whole-document failure never re-reads the pages it names;
- a page that fails at the top rung is never retried with the document-level failure report, although ADR-0018 §3 describes exactly that `resume(from_rung, failure_report)` path.

### 2.5 Grouping today

| Group | What is grouped | Status |
|---|---|---|
| Page | one page, one reading, one carry | sound |
| Document | pages assembled in order; facts first-value-wins; conflicts block | sound |
| Tenant | one job per tenant at a time; per-tenant key and spend | sound |
| Call log | one row per call, no content, per page and rung | sound |
| Register | one row per completed document, one period | sound |
| Part of a page | not modelled | **missing** |
| File holding several invoices | not modelled; mixed facts become a conflict | **missing** |

The last row needs real samples and its own ADR. The first is what this spec adds.

## 3. The gaps that matter

| # | Gap | Evidence | What it costs today | Slice |
|---|---|---|---|---|
| 1 | A cut-off cannot retry above 16k | `reader.rs` truncation branch; Haiku cap equals the ceiling | dense page always to review | A |
| 2 | The ladder ignores the key's model list | `runner.rs` builds the provider without `check_key` | a wasted call per unusable rung | A |
| 3 | A context overflow looks like invalid output | response maps it to `InvalidOutput` | a pointless retry on the same input | A |
| 4 | Text above 200 KB is dropped silently | `fetch_material` filter | the reviewer cannot tell why a page was read from its picture | A |
| 5 | Intake accepts TIFF and XML that the worker cannot read | intake media types vs `inspect.py` | a document that fails after upload | A |
| 6 | A page is never split | no part concept in the pack, router or store | 300-line pages cannot complete | B |
| 7 | One review page blocks all events | `if all.review == 0` | a 300-page invoice yields nothing | B, C |
| 8 | A whole-document failure never re-reads named pages | no `resume` constructor; `complete` ends the job | a fixable misread goes to a person | C |
| 9 | Coverage is implied, not recorded | page state only | no evidence that every part was read | D |

## 4. What "complete" will mean

A document is **done** only when all of these hold:

1. every page was read;
2. every planned part of every page was read;
3. the parts of a page merged without a conflict;
4. the whole-document checks passed, or the one bounded repair round fixed what they named;
5. the events were derived and stored.

The system does **not** promise that the model is right. It promises that it cannot silently skip content: a part that fails makes its page `needs_review`, a page that fails makes the document `needs_review`, and the evidence says which part and why. That is the honest version of "reads it completely".

## 5. Slice A: honest limits and a capability-aware ladder

Small, independently shippable, and it removes the worst dead end before the larger change.

### 5.1 The ladder knows the key

`runner.rs` asks `check_key` once per tenant, caches the answer for one hour, and builds the policy with `without_models` for every model the key cannot use, or that does not support structured outputs or image input. The check is free. If the check itself fails, keep the full ladder: the provider's `ModelUnavailable` path already skips a rung and remembers it for an hour.

The rung is also clamped to what the model reports: `max_output_tokens <= min(policy cap, model max)`. The policy ceiling is already 16k, well below the 128k the models accept, so the clamp is a guard against a future configuration, not a change today.

### 5.2 A cut-off may ask for more room

`RouterPolicy` gains `truncation_ceiling`, default **32 000**, with `answer_ceiling <= truncation_ceiling <= 64 000`. The state machine's truncation branch becomes:

- if the current cap is below the truncation ceiling, retry once at `truncation_ceiling`;
- otherwise the state machine ends the page `too_long`, and the router splits it into parts where they are available (Slice B).

This lets Haiku retry at 32k (worst case about R0.29) instead of giving up. The budget check still runs first, so a Sonnet retry at 32k (worst case about R5.76) is refused on a short document and the page ends `budget`. That refusal is honest: the document's cap is R5 and the call would breach it.

The stored carry gains the field with a serde default, so a job that spans the deploy resumes instead of failing.

### 5.3 A context overflow is a split, not a schema error

The adapter will report a context-window stop as its own error instead of a generic invalid output. The router maps it to `Outcome::ContextExceeded`, which never retries the same input: it splits the page into parts when it can, otherwise ends `too_long`. A pre-flight check compares the estimated input plus the largest answer with the model's `max_input_tokens` and skips a rung that cannot fit, so the call is not made to discover it.

### 5.4 The text drop is visible

When a page's text is above `MAX_PAGE_TEXT_BYTES`, the router records a note `{ "type": "text_dropped", "bytes": n }` on the page and counts it in the audit. The reading still falls back to the picture. The reviewer can now see that the page was read from its picture because its text was too large, not because the text was bad.

### 5.5 Intake accepts what the worker can read

The worker reads PDF, PNG and JPEG. Intake currently also accepts TIFF and XML and the UI says PDF, PNG or JPEG. Until TIFF is implemented and tested with real scans, and until XML e-invoices have their own design, intake refuses both with a clear message. The accepted list and the copy match the worker exactly.

## 6. Slice B: page parts, the real chunk

### 6.1 When a page is split

A page is planned in parts when its text has at least **1 200 words**, which is roughly 200 lines at six words a line, and at least two rows. The plan aims for about **500 words** a part, with **2 to 6 parts**. A page over 3 000 words is capped at six parts; if a part then still truncates, the page ends `too_dense` rather than splitting further. A page below the trigger is read whole as it is today.

Word boxes come from the worker's text endpoint (`words=true`). The router now asks for words on every route, not only `text_and_image`, so OCR pages have them too. The planning itself is a pure function in `clearcut-core::escalation::parts`:

- group words into rows by vertical overlap;
- sort rows from top to bottom;
- choose boundaries between rows so each part is near the target and no two parts differ by more than one row;
- return each part's first and last row, its word count, its byte range in the page text, and its top and bottom in permille.

The coverage invariant is checked before any call: every row belongs to exactly one part, parts are in order, and the first starts at 0 and the last ends at 1 000 (blank margins belong to the adjacent part). A plan that breaks the invariant is not used; the page is read whole and, if it then truncates, ends `too_long`. The invariant is tested over generated word layouts.

### 6.2 What is sent for a part

- **The picture:** the page is rendered at the rung's dpi, cropped to the part's top and bottom in permille, then encoded with the rung's `max_edge` applied to the crop. Cropping lets the renderer use a higher dpi than a whole page could: a third of a page at 200 dpi keeps about 1.3 times the pixels per line of the whole page at 150 dpi, and about 1.6 times at 300 dpi. The quality gain comes from that, and the output gain comes from the smaller part.
- **The text:** the slice of the page text covered by the part's rows, with its source label unchanged (`text_layer`, `unverified_text_layer` or `ocr`).
- **The part label:** `PageInput` gains `part: Option<Part>` with `index` and `of`. The prompt gains one rule: when the pack says this is part k of n of page p, read only what is visible in this part, and never guess a line that is cut at the edge. `PROMPT_VERSION` becomes `prompt-v4` and the prompt golden is regenerated. The reading schema does not change, so `reading-v3` and every stored reading stay valid.

The pack manifest records the part for the call log, so "was context missing?" can be answered per part.

### 6.3 How parts are read and merged

Each part runs its own `PageReader`, with its own escalation, its own calls and its own assessment. `whole_document` is false for every part, even on a one-page document, because a part is not the whole page; the assembled document's totals check remains the whole-document check.

`merge_parts` is a pure function in `clearcut-core`:

- **Lines** are concatenated in part order.
- **Document facts** (invoice number, date, supplier, recipient, tax basis, currency, totals, due, credit note) take the first non-empty value across parts. If two parts state different non-empty values for the same fact, the page is `needs_review` with `part_conflict`; two parts of one page must agree.
- **Carried figures** take the first non-empty value per role, with the same conflict rule.
- **Terms** take the first non-empty value.
- **Confidence** is the lowest part's basis. **Problems**, **findings**, **unconfirmed places**, **injection kinds** and **notes** are the union.
- **Outcome:** `Read` only if every part is `Read`. Otherwise `needs_review` with the first failing part's reason in a fixed order, and the merged best draft.

The merged reading is stored as one page reading, so the store, the review form and the exports do not change shape. `MAX_LINES_PER_PAGE` rises from 200 to **600**, which keeps a page of the longest permitted field values under the store's 250 KB reading limit; a test writes the longest permitted line 600 times to prove it. A page above 600 lines ends `too_dense`. The per-part plan keeps parts near 100 lines, so the higher limit is a safety net, not a target.

### 6.4 Caps, budgets and calls

- Calls per part: the existing 5.
- Document calls: the existing `3 x pages + 10` plus `3 x (parts - 1)` for each banded page. The extra is added when the plan is made and stored with the page's carry, so a resume counts it the same way.
- Budget: the existing document cap and month budget check every call before it happens, including each part. A densely banded document can reach its cap and then ends `budget` with the parts it read kept as a draft.
- No new price, no new model, no new provider call beyond the parts themselves.

### 6.5 Scans without OCR (B.2)

A scan with no text layer and no OCR has no words, so there are no row boundaries. The worker gains a way to find horizontal whitespace rows in the rendered page and return boundaries; the router uses it only after a whole-page read truncates, and at most once per page. If the worker cannot find a safe boundary, it returns equal bands and the page is marked `seam_uncertain`. A page read with an uncertain seam can still be `Read`, but only when no line appears in two parts with different values and the arithmetic passes; otherwise it goes to review with the part evidence. This is the honest fallback: the system says the seam is uncertain instead of pretending it is not.

### 6.6 Worker compatibility

`/v1/render` gains optional `top` and `bottom` in permille, and `/v1/bands` is added for the no-word case. The worker's `/health` capabilities list gains `render_bands` and `page_bands`. The server asks for bands only when the capability is present; otherwise every page is one part and nothing changes. This is capability negotiation, not a feature flag: an old worker and a new server are safe together, and the worker can be deployed first.

## 7. Slice C: one bounded repair round

### 7.1 What triggers it

A document gets **at most one** repair round, and only for findings a better reading could fix. The page-review triggers are checked at the end of the read stage, before it gives up; the finding triggers are checked after assembly. Both draw on the same one-round budget, in that order.

| Trigger | Pages to re-read |
|---|---|
| a page ended `TooLong`, `TopRungFailed` or `PartFailed` | that page, up to 3 pages, in page order |
| `LineTotalMismatch` | the page of the named line |
| `TotalsMismatch` | the page of the stated totals |
| `CarriedBalanceMismatch` | the page that carried forward and the page that brought forward |
| `CurrencyMismatch`, `NonPositiveQuantity`, `AmountOutOfRange` | the page of the named line |

It is never triggered by `DueBeforeIssue`, `Compliance`, `NoLines` or `TotalsOutOfRange`. A stronger model cannot fix a document that really has a zero quantity, a due date before the issue date, or no lines; re-reading would only cost money.

### 7.2 How it runs

`PageReader` gains `resume_at(policy, rung, failure_report, calls_left)`, the constructor ADR-0018 §3 already describes. It starts at the given rung, skips every lower rung, and labels its first call `Repair`. For a finding trigger, the failure report is built from the findings that named the page (fixed names and amounts, never document text), so the model is told what was wrong with the whole document, not only the page; for a page-review trigger, it is built from that page's own problems and findings.

- The repair starts at the rung **above** the one that produced the current reading, or above the last rung tried when the page ended in review with no reading. At the top rung, it retries the same rung once with the failure report, which is a different input.
- A rung the key cannot use is skipped.
- Accepted repairs are stored through a new `replace_page_reading` query that may overwrite a finished row, and only that query may do so. The normal save path keeps refusing to overwrite a finished page.
- The round is bounded by the existing budget and call caps; a repair that would breach either does not happen.
- After the repair, assembly and validation run **once more**. If they still fail, the document ends `needs_review` with the new evidence.

### 7.3 What it records

An audit entry `document.repaired` with the pages, the rungs before and after, the cost in rand, and the outcome (`fixed`, `still_blocking`, `refused_by_budget`, `no_rung`). No content. The page's notes gain `repair_attempted` and the rung used, so the reviewer sees that the system tried before asking for a person.

## 8. Slice D: coverage evidence

### 8.1 What is recorded

Per page, in the existing `notes` list (additive, no migration):

```json
{ "type": "parts", "planned": 3, "read": 3, "seams": [333, 667], "seam_uncertain": false }
```

Per document, in the `document.read` audit detail: pages read, pages banded, parts planned, parts read, pages whose text was dropped, repair rounds and the repair outcome. Counts and geometry only, never content.

### 8.2 Where it is shown

- The review screen shows a per-page badge: "Read in 3 parts", "Part 2 of 3 failed: too long", or "Read from the picture: text too large". The existing page details already travel with the review data; this adds an additive `coverage` object to the document detail payload and to its OpenAPI and SDK type.
- The document summary says how many pages were read in parts and how many parts failed.
- The audit report shows the counts above.
- The register and the five downloads are unchanged: they only ever contain documents that are done.

## 9. Non-goals

- **A file holding several invoices.** Splitting one file into several documents needs real samples and its own ADR.
- **Statement-shaped documents.** ADR-0022 defers opening and closing balances, statement rows and debit/credit conventions.
- **Reading pages in parallel.** ADR-0018 keeps pages sequential; parts are sequential too, so the carried context stays correct.
- **The Batch API.** A later cost optimisation, not a completeness fix.
- **A model judge or a second-opinion vote.** The deterministic checks remain the second opinion.
- **Raising the 1M context.** It is not the limit.
- **TIFF and XML support.** Slice A refuses them honestly; support needs its own design and samples.
- **A partial register entry for a document with a review page.** The repair round recovers the fixable case; the unfixable case still produces no events. Whether to show a draft assembly for review is question 3 below.

## 10. Compatibility, rollout and cost

**No database migration.** The page state, details and carry are JSON; the new fields are additive and validated. The reading schema stays `reading-v3`. The prompt becomes `prompt-v4`, so the prompt golden is regenerated and gold-set scores before and after are not directly comparable, as ADR-0013 requires.

**Correction (5 November 2026):** migration `0013_review_reason_parts.sql` widens the stored review-reason check so `part_conflict`, `part_failed` and `too_dense` can be stored; it changes no data and no column.

**New review reasons:** `part_conflict`, `part_failed` and `too_dense`. The SDK and OpenAPI enums and the review copy gain them. An uncertain seam is a note, not a reason, because a page with one can still be accepted. The fixed order of reasons is documented in one place.

**Deployment order:** worker first (new endpoints and parameters are additive), verify `/health` lists the band capabilities, then the server. A new server on an old worker reads whole pages and behaves exactly as today.

**Cost, worst case per call at the default prices:**

| Call | Worst case |
|---|---|
| one page, Haiku, 16k answer | about R0.16 |
| one page in 3 parts, Haiku, 16k each | about R0.48 |
| one truncation retry at 32k, Haiku | about R0.29 |
| one truncation retry at 32k, Sonnet | about R5.76, refused by a short document's R5 cap |
| a repair on one page, Sonnet, 12k | about R2.16; up to 3 pages, budget-checked |

Realistic answers are about a tenth of the caps, so a three-part page is nearer R0.36 than R0.48. The R5 document cap, R0.50 per page for longer documents and R100 monthly budget do not change. A fully dense 300-page document can still exhaust the monthly budget; it then ends `budget` with everything it read kept as a draft, which is the honest outcome.

## 11. Tests

**Slice A:** a model the key cannot use is skipped and recorded; a model without structured outputs is skipped; a cut-off retries at 32k and a second cut-off ends `too_long`; the budget refuses a retry that would breach the cap; a context overflow splits or ends `too_long` and never retries the same input; the text drop is noted and counted; TIFF and XML are refused at intake with the right message; a stored carry without `truncation_ceiling` resumes.

**Slice B:** generated word layouts produce plans that cover every row exactly once, in order, within 2 to 6 parts; a page below the trigger is one part; a page above 3 000 words is capped at six; boundaries are at row edges; the part picture is the crop of the right rows and is rendered at the rung's size; the part text is the right slice; a merged page concatenates lines in order and takes first non-empty facts; two parts disagreeing on one fact make `part_conflict`; a page is `Read` only when every part is; a failed part keeps the merged draft with the part named; a merged page of 601 lines is `too_dense`; an old worker without the capability reads whole pages; a scan with no words falls back to equal bands with `seam_uncertain`, and a seam contradiction goes to review.

**Slice C:** a misread line total is repaired at the next rung and the document ends done; a repair at the top rung uses the failure report and the same rung once; a repair that still fails ends review with the new evidence; document facts (`DueBeforeIssue`, `Compliance`, `NoLines`) never trigger a repair; more than three implicated pages ends review without spending; a repair refused by the budget is audited as `refused_by_budget`; the normal save path still refuses to overwrite a finished page.

**Slice D:** the parts evidence is stored and loaded; the audit counts match the pages; the review payload carries `coverage`; no content appears in any note, audit or log.

**End to end, with fakes:** a 3-page invoice whose middle page has 320 lines ends `done`, with the middle page read in 3 parts and the events derived; one part fails and the document ends `needs_review` with the part named and every other page's reading kept; a 3-page invoice with one misread total is repaired once and ends `done`; a repair that fails ends `needs_review`; a budget that runs out mid-document keeps every finished part as a draft.

## 12. Delivery

Five small PRs, each tested first and merged only with every gate green:

1. **A**, capability and honesty: `runner.rs`, policy and reader, response mapping, intake and copy.
2. **B.1**, parts for pages with words: worker render crop, pack part and prompt-v4, planner and merge in core, router loops, store evidence.
3. **B.2**, scans without OCR: whitespace bands, capability negotiation, the uncertain seam.
4. **C**, the bounded repair round: `resume_at`, the repair selector, the replace query, the audit.
5. **D**, coverage: audit detail, review payload, review copy and docs.

Slices A and C are useful without B, but the full answer to "reads it completely" needs B. Each slice updates `docs/WORKLOG.md`, the relevant ADR status or a new ADR recording the decision, and the public docs that describe what the system can read.

## 13. Questions for Kone

1. **Page parts:** build them now (recommended, it is the only way a 300-line page can finish automatically), or ship only Slice A and see whether a 32k retry is enough first?
2. **Automatic repair:** allow one bounded round per document (recommended) or never re-read a page without a person asking?
3. **A partial draft for review:** when a page cannot be read, should the document show the draft assembled from the pages that were read (recommended as a separate follow-up, it costs no model calls but needs a review-screen design), or stay as it is, with page drafts only?
4. **The truncation ceiling:** is 32 000 the right ceiling, or should it be 24 000 to keep a Sonnet retry under the R5 document cap?

Recommended answers: 1 yes, 2 one round, 3 separate follow-up, 4 32 000 with the budget refusing what does not fit.
