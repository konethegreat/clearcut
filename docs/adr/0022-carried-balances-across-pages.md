# ADR-0022: Carried balances across the pages of a long invoice

Status: **Accepted** (approved by Kone Tshivhinda, 2026-10-28, with the three recommended answers below) · Date: 2026-10-28 · Deciders: Kone Tshivhinda, Claude
Task P2-07 (second half) and LAUNCH item 6c. Builds on [ADR-0004](0004-ingestion-and-narration-pipeline.md) (events are the source of truth), [ADR-0013](0013-model-facing-reading-shape.md) (the model-facing reading shape), [ADR-0017](0017-document-analysis-stage.md) and [ADR-0018](0018-model-router-and-escalation.md) (one page read at a time, whole-document checks after assembly), [ADR-0002](0002-money-representation.md) (integer money) and [ADR-0006](0006-postgres-schema.md) (events are JSON, so a new kind needs no migration).

## Context

**What is missing.** `clearcut_core::assemble` already puts a document's pages together: lines are concatenated in page order and facts printed once per document (number, date, parties, due, totals) come from the first page that has them. A later page that prints a different value is a `Conflict` and the document goes to a person. So a long invoice whose pages print "carried forward" / "brought forward" figures is today treated as **pages that disagree about the totals**, and the carried figures are either lost or mistaken for the invoice totals. The reading shape (`reading-v2`, ADR-0013) has fields only for an invoice's net, VAT and gross totals; it cannot say "a balance was carried forward". That is the gap LAUNCH item 6c names.

**What the words actually cover.** Two read-only investigations (28 Oct 2026) found that "carried balances and statement totals" is at least three separable changes, not one:

1. **A multi-page invoice** that prints page subtotals and carried/brought-forward figures, with its net/VAT/gross totals on one page.
2. **Statement-shaped documents** (supplier statement of account, bank statement, fee statement): opening and closing balances, debit/credit columns, statement totals, per-row dates. Their rows are not invoice lines (no quantity × unit price, often no VAT column), so reusing the invoice line and validation rules would produce wrong facts or send every page to a person. There is also no document-kind concept anywhere in the code, and no way for a page to say it is a statement.
3. **Page-spanning tables**: a table whose rows continue over a page break, where a continuation page may not repeat the column headers.

**What is promised, and what needs a decision.** The product promises "any invoice, of any length". No statement type is promised anywhere; which documents beyond invoices are in scope (bank, payment-processor, supplier statements) is explicitly one of the decisions only Kone can make, and the same question blocks P2-21's fee-line reconciliation. Real anonymised statements are needed to test statement reading honestly, and they are part of the gold set only Kone can supply. Building statement support without them would be guessing at a data model.

**The version cost.** The reading shape and the cached prompt are frozen by golden files, and stored page readings carry no shape version (`page_readings.reading` is plain JSON). A shape change therefore touches the schema golden, the prompt and its golden, the review form, the SDK and OpenAPI, and makes every stored reading from before the change unreadable. This ADR handles that with a frozen v2 reader (decision 8), so no stored document is stranded and no migration is needed.

## Decision

**Phase 1, this ADR: carried balances on long invoices.** Statements and page-spanning tables are deliberately not in this decision (see "Not decided here"). This is the half that matches the invoice promise, can be validated now, and settles what a carried balance means before any statement model is committed.

1. **Scope: invoices only.** A page of an invoice may print a **brought forward** figure at its head and a **carried forward** figure at its foot. Both are reported when printed, and are not the invoice's net/VAT/gross totals. A statement page, whose opening/closing balances are a different concept, waits for its own ADR.
2. **Reading shape `reading-v3`.** Every page reading gains one required object:

   ```json
   "carried": { "brought_forward": "", "carried_forward": "" }
   ```

   Amounts are written like any other amount (digits, one dot, optional leading minus, no separators), empty when not printed. Every property stays required, so the schema still has 0 optional and 0 union properties against the API's limits of 24 and 16 (ADR-0013). The prompt gets one short rule and one worked example: these fields are only for the printed continuation figures, never for the invoice totals; never calculate a figure, never carry one over from another page. `READING_SHAPE_VERSION` becomes `reading-v3` and `PROMPT_VERSION` becomes `prompt-v2`; both golden files are regenerated in the same change.
3. **Strict conversion.** The two figures go through the same exact decimal parser as every other amount. A figure that cannot be read is a `ReadingProblem` (the page goes to a person), never a guess; an empty string means "not printed". Each figure keeps its page as a `Sourced` value, so it can be cited.
4. **Assembly keeps them per page.** The pages' carried facts are a list in page order, not a first-value-wins document fact: pages may legitimately carry different figures. They produce no `Conflict`. The assembled extraction carries the list so the whole document can be checked and narrated.
5. **One deterministic whole-document check: continuity.** If page N prints a carried-forward figure and page N+1 prints a brought-forward figure, they must be equal **exactly to the cent**. A mismatch is a blocking finding, so the document ends `needs_review` and the events are still stored. If either side is not printed, no claim is made. Phase 1 does **not** add an arithmetic rule (`brought forward + page subtotal = carried forward`) because the meaning of a page subtotal (net or gross) depends on the layout, and a guess would create false reviews; real samples can decide it later.
6. **Facts become events.** Two new kinds, additive JSON (no migration; event types are generated from the payload):
   - `balance_stated` — **Info**, group Totals: `role` (`brought_forward` or `carried_forward`) and `amount`, cited to the page that printed it. Narrated as "The amount brought forward is R X." / "The amount carried forward is R X."
   - `balance_mismatch` — **Critical**, group Totals: `after_page`, `carried_forward` and `brought_forward`, for a broken link. Narrated as "The amount carried forward does not match the amount brought forward on the next page: R X and R Y."
7. **Reports, exports and checks.** The stated figures appear in the Totals sheet of the export (label and page) and in the event rows; the mismatch appears under the checks because it is not Info. The per-page text check (`clearcut_core::text_check`) also looks for the two figures in the page's own text, so an invented carried figure is caught like any other amount. The number guard sees the amounts through `money_fields`.
8. **Stored readings stay readable.** A small frozen `reading-v2` reader lifts a stored v2 row into v3 with empty carried figures. New readings are v3 only. No database migration, no reset, no stranded document, and the v3 schema keeps ADR-0013's all-required shape (a serde default would make the model-facing fields optional, which is why this is a reader and not a schema compromise).
9. **No migration, no new review reason, no new job state.** The page state machine, the queue and the review form's contract stay as they are apart from the two new fields; the review form and the SDK/OpenAPI type gain them.
10. **Delivery in two small PRs under this ADR.** (a) Shape, conversion, assembly, v2 compatibility, prompt and schema goldens. (b) Continuity finding, events, narration, exports, text check, reporting docs. Each is tested first and merged only with every gate green, as usual.

## What this protects, and what it does not

- **Protects:** a long invoice that prints continuation figures is no longer mistaken for a document whose pages disagree; the chain is checked exactly; the figures are cited, guarded by the number and text checks, and visible in reports and exports. A missing, duplicated or misordered page breaks the chain and goes to a person.
- **Does not do:** statements (opening/closing balances, statement totals, transaction rows, debit/credit columns, per-row dates); page-spanning table headers or printed-page-number/missing-page detection; P2-21 fee-line reconciliation; an arithmetic check of page subtotals. Statement support needs its own ADR, Kone's choice of document types and real anonymised samples.
- **Costs:** no model call is added; a page costs what it cost before. The reading-shape and prompt bump means gold-set scores taken before and after are not directly comparable (ADR-0013's rule). Carried figures are not in the gold-set label format yet, so their accuracy cannot be scored until that format is extended; that is a stated limit, not hidden.

## Alternatives considered

- *Put the carried figures in the existing `totals` field.* They are not net/VAT/gross; reusing it means inventing VAT or causing exactly the conflict this ADR removes.
- *Build statements in the same step.* Their rows are a different data model, no statement type has been chosen, and there are no real samples to test against; it would mix an invoice fix with an unverified statement model.
- *A separate statement reading shape and a second router path now.* Duplicates conversion, review, SDK and validation work before the scope is known.
- *Allow a tolerance in continuity.* The invoice rule is exact to the cent (ADR-0002, research 05 §3); a tolerance would let wrong money pass.
- *Make the new fields optional in the schema so old readings deserialize.* It would break ADR-0013's all-required shape and turn the model-facing schema into a union; the frozen reader keeps the shape strict.
- *Warn instead of blocking a broken chain.* A carried balance is money; the invoice precedent is to store the events that say what happened and send the document to a person.

## Tests the two PRs will add

Conversion (each figure, a bad decimal, an empty figure, a negative figure); per-page preservation through assembly; continuity (holds, breaks, one side missing, several pages, the last page); the new events' id, date, citation, severity and payload; narration sentences and the number guard; export rows; the text check finding an invented figure; a stored v2 reading lifted into v3; and end to end in the pipeline: a three-page invoice with a correct chain ends `done` with its events, and one with a broken chain ends `needs_review` with the events stored and the mismatch named. Plus the regenerated schema and prompt goldens, and the SDK/OpenAPI contract.

## Questions for Kone

1. **Scope:** do the multi-page **invoice** carried figures first (recommended: it matches the promise and can be tested now), or design **statements** now instead? If statements, which types (supplier statement of account, bank statement, fee statement) and can you supply a few real anonymised statements of that type? Statements cannot be tested honestly without them.
2. **A broken carried chain:** block the document for a person (recommended, exact to the cent) or only warn and still end `done`?
3. **The carried figures:** show them in reports and exports (recommended) or keep them in the event log only?

## Decisions (Kone's answers to the three questions)

1. **Long invoices first.** Statements wait for their own ADR, when Kone chooses the document types and supplies real anonymised samples.
2. **A broken carried chain blocks:** the document ends `needs_review`, and the events are still stored.
3. **The carried figures are shown in reports and exports**, not only in the event log.

## Not decided here (a later ADR, when Kone decides)

Statements as a document kind; statement transaction rows; opening/closing balances and statement totals; debit/credit conventions; per-row dates; page-spanning table headers; extending the gold-set label format for balances; whether a failed continuity check should re-read the pages it names (ADR-0018 §5 asks for it, but no path exists yet and it costs money).
