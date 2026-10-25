# ADR-0018: The model router and the escalation ladder

Status: **Accepted** (approved by Kone Tshivhinda, 2026-10-25, with all eight recommended answers below; for question 6 the per-document cap grows with the pages) · Date: 2026-10-25 · Deciders: Kone Tshivhinda, Claude
Task P2-14, with the page-reading half of P2-06 and the per-call log of P2-15. Builds on [ADR-0005](0005-model-strategy-escalation.md) (the ladder), [ADR-0011](0011-bring-your-own-anthropic-key.md) (the user's key, budgets, errors), [ADR-0012](0012-anthropic-adapter.md) (the adapter and its failures), [ADR-0013](0013-model-facing-reading-shape.md) (what the model returns), [ADR-0014](0014-context-pack-builder.md) (what is sent), [ADR-0017](0017-document-analysis-stage.md) (how each page is read) and [research 06](../research/06-anthropic-api.md) (the API's facts). Design only; nothing is built until this is approved. **No model has been called in the whole project yet**, so every number below that depends on real model behaviour is a starting value for the gold set to correct.

## What it is
Every piece the router needs exists and is tested, and nothing yet **decides what to do with them**:

```text
page_reads (ADR-0017)  ──►  render, text, mask (worker)  ──►  context pack (ADR-0014)  ──►  provider (ADR-0012)
   route per page                                                                                   │
                                                                                                    ▼
 next rung ◄── escalate ◄── assess: conversion problems + validate + text check ◄── PageReading (ADR-0013)
     │                                         │ accept
     └── top rung fails, budget or caps hit ───┴──► needs_review (with the best draft)
```

The router reads **one page at a time** and decides, from deterministic evidence only, whether the reading is good enough to keep, should be tried again by a stronger model with a report of what was wrong, or should go to a person. It ends every page in one of two states, `read` or `needs_review`, never a third, and it cannot loop for ever or spend more than it was allowed.

## 1. Where the code goes
| Part | Where | Why |
|---|---|---|
| The ladder as data (`RouterPolicy`: rungs, effort, output caps, image sizes, call caps), the **assessment** (a reading is accepted, or escalated for a named reason), the **page state machine**, the evidence-based confidence | `clearcut-core::escalation` | These are business rules: pure, no I/O, no clock, tested exhaustively, and tuned on the gold set without touching an async runtime |
| Prices and the cost of a call in rand (`Cost`, a price table keyed by model id with the date it was read) | `clearcut-core::cost` (the first half of P2-12) | The router must check budgets, so it needs the arithmetic; integer units finer than a cent, rounded up when charged to a budget |
| The async driver for one page: gets the picture and text from the worker, builds the pack, calls the provider, feeds the answer to the state machine | new crate `clearcut-router` | Generic over `ModelProvider` and `DocumentProcessor`, so it is tested with `FakeProvider` and `FakeWorker`. **No database.** |
| The read stage of the job: pages in order, saving each result, budgets, resuming | `clearcut-pipeline` | Persistence and the job stay where they are |

**The state machine is the design's centre.** It is a plain value: `PageReader::start(policy, route, page_of_n) -> Step`, and `step.on_outcome(outcome) -> Step`, where a `Step` is `Call(CallPlan)` (use this rung, this effort, this report) or `Done(PageOutcome)`. The async driver only loops "do what the plan says, report what happened". Three things follow: it is testable without any async code or network; **termination is a property we can test** (every possible sequence of outcomes ends within the caps, checked over generated sequences); and a later **Batch driver** (cheaper, results within 24 hours) can run the same machine in rounds, all pages at the first rung in one batch, then the failures at the next, without a second copy of the rules.

## 2. The ladder
Haiku 5.5, Sonnet 5.5, Opus 5.5, Fable 5.1, then a person (ADR-0005). As data:

| Rung | Model id | Effort | Largest answer | Page picture | Default |
|---|---|---|---|---|---|
| 1 | `claude-haiku-5-5` | medium | 16 000 tokens | 150 dpi, 2 000 px | on |
| 2 | `claude-sonnet-5-5` | medium | 12 000 | 150 dpi, 2 000 px | on |
| 3 | `claude-opus-5-5` | high | 8 000 | 200 dpi, 2 576 px | on |
| 4 | `claude-fable-5-1` | high | 4 000 | 200 dpi, 2 576 px | **off** (Kone, ADR-0011: it keeps data for 30 days and may be refused by the account) |

- **Effort is fixed within a rung** (a changed effort breaks the prompt cache) and is the only thinking control (`thinking` omitted, never `budget_tokens`). A larger picture on the upper rungs is a deliberate part of "resource completeness": a page that failed twice may simply have been read too small. The picture comes after the cache breakpoint, so this costs input tokens and nothing else.
- **The largest answer includes the model's thinking** and is also the worst-case bill of one call. See the budget rule (§6): it is why the upper rungs are smaller, so a single call can never cost more than a document is allowed. These numbers are the first thing a live run should check.
- **What the key can use** comes from the Models API (the free key check). A rung the key cannot use is skipped; a `ModelUnavailable` answer (a 403/404 on the model, or Fable refused for its retention rule) skips the rung and is remembered for that tenant for an hour, so the next document does not pay to find out again.
- The policy is validated when it is built (rungs in order, at least one on, each rung's worst-case call within the document cap, caps above zero) and is read from operator configuration, never from tenant input.

## 3. When a reading is accepted, escalated or sent to a person
This is the rule of the whole feature, and it uses **only** evidence ClearCut can compute. Nothing the model says about itself, and nothing the page says, picks the next step.

**Accepted** when all of these hold:
1. the answer follows the schema and converts (`PageReading::into_extraction`) with **no problem on a critical field** (the list is the gold set's: invoice number, date, currency, supplier name and VAT number, the three totals, each line's quantity, unit price and total);
2. the **page-local arithmetic** passes: quantity times unit price equals the line total on every line, no zero or negative quantity, amounts in range, one currency;
3. the **text check** passes, where the page has trustworthy text (route `text_and_image` or `image_first`, OCR text on `image_only`): every amount the model read must occur as digits in the page's text (P2-13c, proposed in ADR-0013 §4). This is what catches a number the model invented, which arithmetic alone cannot, and it is free.

**Escalated** (to the next rung, with a failure report in ClearCut's own vocabulary, ADR-0014) when any of 1 to 3 fails, with one named reason: `reading_problems`, `line_arithmetic`, `totals_mismatch`, `text_check_failed`. A schema failure or a cut-off answer is handled first on the same rung (§5).

**Not a reason to escalate**, because a stronger model cannot fix it and escalating would burn money:
- findings about the **document itself**: a due date before the invoice date, a South African tax-invoice rule that the document breaks (`compliance_za`). They are recorded on the page and shown to the reviewer;
- a page with **no lines** or **no totals**: a terms page or a continuation page is allowed to have neither. Whether the *document* has them is decided after the pages are put together (P2-07).

**The totals check is page-local only for a one-page document.** On page 3 of 5 the lines may sit on pages 1 to 3 and the totals on page 5, so a page-local "lines do not add up to the printed total" would be a false alarm and would escalate a correct reading. Reconciling lines to totals across pages is P2-07's job; when it fails, it asks the router to **re-read** the pages it names at the next rung (`PageReader::resume(from_rung, failure_report)`). Which pages to re-read is P2-07's decision and is not made here.

**Not used, and why.**
- *The model's own confidence.* ADR-0005 listed it as a trigger, but the reading shape deliberately has no confidence field (ADR-0013: models state high confidence on wrong numbers). Confidence attached to a value comes from evidence instead: confirmed in the page text and arithmetically consistent, 950; consistent but unconfirmed (no usable text), 850; anything on a page that ended with open problems, 500 (below the gold set's review threshold of 800, so it is flagged). These are provisional constants for P2-09 to refine, and they do not depend on the tier.
- *Two independent passes and a vote* (also in ADR-0005). Every page would cost at least double. The arithmetic and the text check are the second opinion, for free. A second pass is kept as an option for the one place they cannot help, a critical value with neither text nor arithmetic behind it, and is added only if the gold set shows such a case being wrong.
- *A model as judge.* A judge reads the page too, so it is as open to injection as the reader, and it is not deterministic.

## 4. Where each page starts
**Every page starts at rung 1**, including scans and doubtful pages. ADR-0005 started messy scans at Sonnet. The arithmetic says otherwise: at the planning prices (research 06 §10, R18/USD) a Haiku attempt costs about R0.02 a page and a Sonnet one about R0.40, so trying Haiku first is cheaper whenever it succeeds on more than R0.02 / R0.40, which is 5% of such pages (Haiku first costs `0.02 + 0.40 x (1 - p)`, Sonnet first costs `0.40`). A failed Haiku attempt wastes two cents. The ADR therefore starts low everywhere and lets a per-route start rung be set from the gold set, if scans turn out to fail at Haiku nearly always. This is the one place I deliberately depart from ADR-0005's table.

What the route (ADR-0017) changes is **what is sent**, not where it starts:
| Route | Sent |
|---|---|
| `text_and_image` | the page picture and its text layer, labelled `text_layer` |
| `image_first` | the picture, and the text layer labelled `unverified_text_layer`: "where they disagree, trust the picture" |
| `image_only` | the picture, and OCR text labelled `ocr` if OCR was run, otherwise no text |

The third label needs one sentence in the cached rules, so the prompt text changes. `prompt-v1` has not been used for any real call, so, as ADR-0014 did before, it is edited in place and the golden file regenerated; the first live call is what freezes it.

## 5. What each failure does
The router **never retries a transport failure itself.** The adapter already retries inline twice (ADR-0012 §5) and the job queue retries after that (ADR-0008), so a third layer would multiply them. The router's own loop is only for quality (escalation) and for two answer-shape cases.

| What came back | The router does |
|---|---|
| A reading that fails §3 | escalate, with the failure report; the same rung is not tried again |
| `Truncated` (cut off at the answer cap) | once more on the same rung with the cap raised to the adapter's 16 000 (or the policy's ceiling); a second cut-off means the page is too dense for one call: **`needs_review`, reason `too_long`** (a stronger model would be cut off the same way) |
| `InvalidOutput` (not valid against the schema, rare with structured outputs) | once more on the same rung; then escalate |
| `Refused` | `needs_review`, reason `refused`; the same input is not sent again (ADR-0011). Billed |
| `ModelUnavailable` | skip this rung, remember it, continue at the next |
| `KeyProblem` (rejected, billing, monthly cap, the owner's own limit) | **stop the whole document**, keep what is read, end the job as provider-unavailable with the long delay of ADR-0011. No escalation can help |
| `Unavailable`, `Timeout` | stop at this page; the job retries later (no attempt used for the first, one for the second) and **resumes at this page and rung** |
| `Rejected` (our request was wrong) | `needs_review`, reason `request_rejected`: a bug, and it is logged with the request id |
| `CannotBeSentSafely` (strict redaction) | `needs_review`, reason `cannot_be_sent_safely`; nothing was sent |

## 6. Budgets and caps: the loop cannot run away
Two kinds of limit, both checked by the state machine **before** a call, so a call that would break one does not happen:
- **Caps independent of money** (ADR-0011): at most **5 calls per page** (one per rung plus a retry) and **3 calls per page of the document plus 10** per document. They bound the damage of a bug in the rules, whatever the prices are.
- **Money:** the next call may happen only if `spent so far + the worst case of that call <= the document cap`, and `+ ... <= what is left of the month`. The worst case is the estimated input plus the rung's **largest answer** at its output price; token counting is free, so the input is exact for a real page. Reserving the worst case is stricter than needed (most answers are a tenth of the cap) and is what makes the promise of ADR-0011 true: a call that would breach a budget does not happen. When the check fails the page ends `needs_review`, reason `budget`, and the job says whether it was the document or the month.

At the defaults (R5 a document, R100 a month, ADR-0011) the output part of the worst case of one call, the answer cap times the output price at R18/USD, is about R0.14 on Haiku, R2.2 on Sonnet, R2.9 on Opus and R3.6 on Fable. Adding the input of a page (about 6 000 tokens, R0.01 to R1.1 depending on the model) every rung still fits in a document cap of R5, and that is why the caps in §2 are what they are. Fable only just fits, which is another reason it is off by default.

**A flat R5 stops long documents.** A 300-page statement read once at Haiku costs about R6 before any escalation. I propose the document cap **grows with the pages**: the larger of the configured amount and R0.50 a page, still never above the month's remaining budget. Question 6.

The monthly budget table, the tenant settings (Fable on or off) and the batch mode are P2-12. The router asks a small `SpendGate` (reserve, then record the real usage) so P2-12 can plug in; until then the gate is the per-document one, kept in memory, and **nothing may run against a real key until P2-12 is built** (the handler is not wired into the server yet, so this costs nothing now).

**Illustration, not a measurement.** If 90 pages in 100 pass at Haiku, 8 go on to Sonnet and 2 to Opus, the expected cost is `0.02 + 0.10 x 0.40 + 0.02 x 0.79`, about **R0.08 a page**: R0.80 for a 10-page invoice, R8 for 100 pages. The percentages are guesses; the first gold-set run replaces them. If every page used every rung it would be R3.19 a page, which is exactly what the budget exists to stop.

## 7. Order, context and the prompt cache
- **Pages are read in order, one at a time (concurrency 1) in this version.** Each page's pack carries what earlier pages established (the currency, the invoice number, the supplier, whether totals were seen, ADR-0014 §1), and the conversion needs `currency_known` from an earlier page. Reading pages in parallel would lose both. It also satisfies the cache rule for free: the first call finishes before the next starts, and each read refreshes the five-minute entry. A 100-page document at about 10 seconds a page is under 20 minutes, which suits a background job. Bounded fan-out is added with the rate-limit-aware client (P2-24), once the gold set shows that context matters less than speed.
- **Escalating pays a new cache write**, because the cache is per model: about 1.25 times the rules prefix once per rung used. It is small, and it is logged.
- **The cache lifetime** is five minutes; one hour for bulk uploads (P2-16).

## 8. What is stored
Two new tables, both under row-level security and erased with the document. **The SQL is shown to Kone before it is applied**, as for every schema change.
- **`page_readings`:** one row per page: the final reading as the model gave it (the `PageReading` JSON), the converted problems, the findings, the rung, the confidence basis and the outcome (`read` or `needs_review` with the fixed reason). It is what a job resumes from, what a reviewer starts with (a page that went to review keeps the **best draft**: the attempt with the fewest blocking issues, the higher rung on a tie), and what the audit of ADR-0004 needs. **It holds invoice content.** The readings of rejected attempts are not kept; only the final one is.
- **`model_calls`:** append-only, **no content at all**: one row per call with the page, attempt, rung, model, effort, picture size, the prompt and shape versions, the pack manifest (ADR-0014: section sizes and counts, never text), the four usage counts, the cost in rand, the time taken, Anthropic's request id, and the outcome (`accepted`, `escalated` with the reason, or the error kind). This is P2-15, and it answers "was context missing?" and "did the cache hit?".
- A `document.read` audit entry with the page counts, the rungs used, the total cost in rand cents and how the document ended (counts and fixed names only).

## 9. How a page ends, and how the document does
`PageOutcome` is `Read { rung, extraction, notes }` or `NeedsReview { reason, draft }`, with `notes` holding what was recorded but did not block (document-level findings, `injection_suspected`, redaction counts). The reasons are a fixed list: `top_rung_failed`, `too_long`, `refused`, `budget`, `call_cap`, `cannot_be_sent_safely`, `request_rejected`, `no_rung_available`. A document with any page in review ends `needs_review` but keeps every page it read, so a person starts from the drafts. Until P2-07 (putting pages together) exists, the handler still ends the job `needs_review` with "read; the later stages are not built yet", the same honest ending ADR-0017 uses, so no document is marked done for work that was not done.

## 10. Tests (no key, no money)
- **The state machine** against scripts: accept first time; escalate on each named reason up the ladder and stop at the top; every error row of §5; truncation retried once; the caps and the budget refuse the call that would break them; a skipped rung; resuming at a rung. A **termination property**: for any generated sequence of outcomes the machine reaches `Done` within `5 x rungs` steps and never exceeds a cap.
- **The assessment** over the 15 synthetic gold-set cases: a correct reading is accepted; each kind of corruption (one digit of a line total, a swapped total, a number not on the page, a bad date) is escalated with the right reason; a corruption that is consistent with the text and the arithmetic is *not* caught, and a test says so (that is the limit of deterministic checking, and what the reviewer is for).
- **Document-level subtleties:** a continuation page with no lines is accepted; a page 3 of 5 whose lines do not match the printed total is **not** escalated; a due date before the invoice date is a note and not a reason.
- **The driver** with `FakeProvider` and `FakeWorker`: the pack that is sent for each route, higher picture size on the upper rungs, the failure report in the second call, no text in any log or `Debug`, a hostile page (injection phrases) changes nothing about the next step, `CannotBeSentSafely` sends nothing.
- **The costs:** every price times every usage kind, rounding up, the 100 000-token step of Haiku, cache reads and the one-hour write.
- **Stored results:** resume after a stop skips finished pages and repeats only the page in progress; erasure takes both tables; a tenant cannot see another's.
- **A live check for the day there is a key** (`#[ignore]`, never run in CI): one real page through the whole path on Haiku, printing the cost in rand; and the gold-set run that finally measures the pass rate per rung, the cost per page and the cache hit rate.
- A hand mutation check, as for every crate.

## Implementation notes

**Step 1, `escalation` in `clearcut-core`** (the rules; no I/O, no money, no model; 74 new tests, 484 in the crate):
1. **`RouterPolicy`** is the ladder as data, checked when built: at least one rung on, plain unique model ids (letters, digits, `-`, `_`, `.`, at most 128), each rung's answer cap between 1 and the ceiling (16 000 by default, at most the provider's 64 000), each rung's picture inside what the worker draws (100 to 300 dpi, 500 to 2 576 pixels), call caps above zero. `default_ladder()` is the table of section 2 (Fable off). `with_enabled` turns a model on or off and checks again, so the last rung that is on cannot be switched off; `without_models` takes out the models a key cannot use and is deliberately not checked (a key may use none, and the router then says `no_rung_available` instead of failing to start). `document_call_cap(pages)` is 3 a page plus 10, saturating. A rung is named by its position in `rungs()`, so the driver must use the policy it started the page with.
2. **`assess`** takes the problems converting the reading, the findings of `validate`, a `TextCheck` (not available, passed, failed with a count) and whether the page is the whole document, and returns a verdict, how many things block, notes, every reason that applies (the first is the one named, in the order reading problems, line arithmetic, totals, text check) and a confidence basis. **A problem on a path nobody listed is critical**, so a new path in the converter can never hide a problem; the non-critical ones are exactly the recipient, the due date, the supplier's address and a credit note's original invoice number. Line arithmetic (a total that is not quantity times price, a zero or negative quantity, an amount out of range, a currency mismatch) escalates on every page; the totals check only on a one-page document; no lines is never a reason; a due date before the invoice date and a tax-invoice rule the document breaks are notes.
3. **Two things the ADR left open, settled here:** (a) `amounts_include_vat` and `too_many_lines` are review verdicts at once, never escalations (a stronger model reads the same printed amounts, and the same long page); `amounts_include_vat` wins over `too_long` whichever comes first, and both count as blocking so the reading can still be the best draft. (b) `ReviewReason` gains `amounts_include_vat` beside the eight of section 9, and `EscalationReason` has the two ADR reasons for same-rung problems as `schema_invalid` and `rung_unavailable`.
4. **`PageReader`** is the state machine: `start(policy, document_calls_left)` and `on_outcome(outcome)` each return a `Step`, a `Call(CallPlan)` (attempt number, rung, model, effort, answer cap, picture and a `CallKind`: first, escalation with the rung it came from and the reason, retry after a cut-off, retry after an invalid answer) or `Done(PageOutcome)`. It counts calls (a call refused before it was sent is not one), keeps the best draft (fewest blocking issues; a tie goes to the later, higher rung; a reading that could not be converted is no draft), retries a cut-off once with the ceiling and an invalid answer once, and gives each new rung its own retries. Once done it keeps saying so. It is `Serialize` and `Deserialize`, so a job taken over mid-page resumes from the stored machine; the stored form holds counters, model ids and fixed names.
5. **It always ends.** Tests run every sequence of five outcomes over twelve kinds (248 832 of them) and 20 000 generated policies and outcome streams, and require the page to be done within its cap, never issuing more calls than the page cap or the document's calls left.
6. **Mutation check:** about 130 deliberate breakages of the three files; the survivors of the first run were missing tests (the blocking count of the two review problems and the order in which they come, and the rung a reading is named by when an earlier rung is off) and are caught now. One field turned out to be redundant (a flag saying the cut-off retry had been used, since the raised cap already stops a second one) and was removed rather than tested.
7. **Not done in this step:** prices and cost (step 2), the text check itself (step 3), the pack's `unverified_text_layer` label (step 4), the driver (step 5), the tables (step 6) and the read stage (step 7).

## Build order (after approval)
1. `escalation` in `clearcut-core`: the policy, the assessment, the state machine, confidence from evidence. 2. `cost` in core (prices and `Cost`; the first half of P2-12). 3. The text check against the page's own text (P2-13c). 4. The pack: the `unverified_text_layer` label and the rules sentence. 5. The `clearcut-router` crate (the driver, with the fakes). 6. The migration for `page_readings` and `model_calls` (shown first) and the store functions. 7. The read stage in `clearcut-pipeline`, resuming. 8. Mutation check, live-test stub, docs.

## Alternatives considered
- *Start scans and doubtful pages at Sonnet* (ADR-0005). See §4: a failed cheap attempt costs two cents.
- *Ask the model how sure it is, and escalate on low confidence.* Not calibrated, and invisible to the checks; replaced by evidence.
- *Always read twice and compare.* Doubles the bill on every page to catch what arithmetic and the text check already catch.
- *Escalate the whole document, not the page.* Pays the top rung for the pages that were fine.
- *Let tools validate inside the model call.* Decided against in ADR-0014 (a loop, an action path from page text); unchanged.
- *The router inside `clearcut-pipeline`.* It would tie the rules to the database and make the loop harder to test; the split in §1 keeps the pure part pure.
- *Retry transport failures in the router.* Multiplies the adapter's and the queue's retries; refused in §5.
- *Read pages in parallel from the start.* Loses context between pages, and needs the rate-limit-aware client first.

## Questions for Kone (all eight answered: yes, as recommended)
1. **Layout (§1):** the rules, state machine and costs in `clearcut-core`; an async driver in a new `clearcut-router` crate with no database; the read stage and storage in `clearcut-pipeline`: agreed?
2. **Start every page at Haiku (§4),** including scans, with a per-route start rung to be set from the gold set; this departs from ADR-0005's table: agreed?
3. **Evidence only (§3):** no model confidence, no two-pass voting, no model as judge; accept on schema, critical-field problems, page-local arithmetic and the page-text check; the text check (P2-13c) built as part of this: agreed?
4. **Page-local versus document-level (§3):** the totals check is page-local only for one-page documents; reconciling across pages and choosing which pages to re-read belongs to P2-07, and the router offers "resume at this rung": agreed?
5. **One page at a time, in order (§7),** concurrency 1 for now, bounded fan-out with P2-24: agreed?
6. **Budgets (§6):** reserve the worst case of the next call; the defaults of §2; 5 calls a page and 3 per page plus 10 per document; and should the **per-document cap grow with the page count** (the larger of R5 and R0.50 a page), or stay a flat R5?
7. **Storage (§8):** `page_readings` keeps the final reading (invoice content, erased with the document) and the best draft of a page sent to review, but not the readings of rejected attempts; `model_calls` keeps metadata only. Agreed?
8. **The picture grows on the upper rungs (200 dpi, 2 576 px) and `image_first`/`image_only` pages get the `unverified_text_layer` label,** with `prompt-v1` edited in place because no live call has happened: agreed?
