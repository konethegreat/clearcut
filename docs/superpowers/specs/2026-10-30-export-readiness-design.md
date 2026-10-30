# Export readiness: a register and a period summary

_Written by Kone Tshivhinda & Claude, 30 Oct 2026. Approved in conversation before writing. The product target recorded in section 2 is Kone's: 95% of the system automatic, 5% human review._

## 1. Why

ClearCut already exports one document well. Eight formats come from one model built from the stored events ([`crates/clearcut-export/src/lib.rs`](../../../crates/clearcut-export/src/lib.rs), [`crates/clearcut-core/src/export/mod.rs`](../../../crates/clearcut-core/src/export/mod.rs)): a cited summary, every line, every total, every check, and the page each fact came from. A long invoice keeps all its pages and carried balances. Nothing in that needs rebuilding.

Finance departments and bookkeepers do not work one document at a time. They work a month, a batch, a supplier. They open a register, reconcile it to a VAT period, and hand a clean file to someone else. Today the system gives them a pile of separate downloads and no cross-document table. Bulk upload is also one file at a time: the screen takes a single file and jumps to it ([`crates/clearcut-server/src/web/app.js`](../../../crates/clearcut-server/src/web/app.js)).

This adds the layer they open first, without inventing anything that is not in the stored events: a register of many documents, a VAT summary for a period, a sheet of the few things a person must look at, and a manifest that says exactly what went into the file. Batch export (P4-08) closes with it.

## 2. The automation target

The product target is **95% of documents finish without a person, and 5% are reviewed**. It is measured at the document level: among documents that finished reading in a period, the share that ended `done` rather than `needs_review`. Pages are measured too, because a page is where a person actually works.

The target is a measured goal, not a claim, and it does not authorise weakening a check. A figure that is not certain still goes to a person. What it does change is everything around the check:

- **No setup.** The register defaults to the last complete month, every document, Excel. A person downloads it without choosing documents, reconciling rows or picking a format.
- **Exceptions, not paperwork.** The workbook leads with the register and the summary, and puts the documents that need a person on their own sheet, one row each, with the reason.
- **The rate is visible.** The preview and the manifest report the period's automation rate, so progress toward 95% is seen rather than assumed.
- **One review means one thing.** A document either needs no person or names exactly why it does.

**Where the 95% target meets current rules.** Two review reasons fire by design today and are the whole question:

- `amounts_include_vat`: every page whose amounts include VAT is sent to a person, even when the division and the page's own totals reconcile exactly (ADR-0023, ADR-0024).
- `tax_not_shown`: a page that says nothing about tax and whose totals do not settle it.

If the measured rate is below 95%, the first candidate is to let a reconciling inclusive page finish automatically, with the division recorded and visible, instead of sending every one to a person. That is a correction-to-reading decision, so it needs **its own ADR and gold-set evidence**. It is not decided here and not implemented by this work. Until then the register reports the real rate; it never hides a review.

## 3. Scope and non-goals

**In scope:** a cross-document register model in core; a store query that selects the documents of a period and loads what the register needs in a few queries; layouts for a workbook, a bundle of CSVs, JSON, PDF, HTML, Markdown and plain text; one endpoint; a Register screen with preview and download; and multi-file intake as a separate slice.

**Not in scope:** any change to the per-document exports or their formats; native Xero, QuickBooks or Sage files; a VAT201 form; deciding whether a document is input or output VAT; statement parsing; automatic line categorisation; scheduled or emailed exports; a background job or stored export files; any model call. The register spends no money.

## 4. What a register covers

A register is built for either a **period** or an **explicit list of documents**.

- **Period:** `from` and `to` are invoice dates, both inclusive. A document belongs to the period when the most recent current event that carries a date has an invoice date inside it. This is the same rule near-duplicate detection already uses: every derived event carries the invoice date ([`crates/clearcut-core/src/duplicate/mod.rs`](../../../crates/clearcut-core/src/duplicate/mod.rs)).
- **Undated documents** are never put in a month. In period mode they are not pulled into the period. The manifest reports how many of the tenant's documents have no invoice date, with a line saying they are not part of any period and can be selected explicitly. In explicit mode they are rows with a blank date and are flagged on **To check**. Nothing disappears silently.
- **Explicit list:** the ids win and the period is ignored. Every id is a register row even if it has no date, and the manifest records that the selection was explicit.
- **Not yet read:** a document still `received` or `processing` is a register row with its status and no figures, flagged on To check as "still being read". It is never counted in a VAT sum.
- **Needs review:** a document ending `needs_review` is a register row. Its figures are whatever its current events state, and it is flagged with the reason its page waits. VAT totals separate ready figures from figures awaiting review (section 6).
- **A document that is not the tenant's** answers `404`, exactly as the per-document endpoints do.

## 5. The register model

A new pure module, `clearcut_core::register`, takes for each document its stored facts, its current events and the state of its pages, and builds one `Register` with no I/O:

- **`RegisterRow`**, one per document: id, file name, uploaded at, status, kind, supplier name and VAT number, invoice number, invoice date, due date and terms, currency, stated net, VAT and gross, line count, page count, review reasons with their pages, possible duplicate of which document, and who or what read it.
- **`RegisterLine`**, one per line item: document id, invoice number, supplier, invoice date, line number, description, quantity, unit price, line total, tax category and **page number**, so a long invoice stays traceable.
- **`VatGroup`**, one per month, currency, document kind and tax category, from the calculated VAT events.
- **`StatedTotals`**, one per month, currency and document kind, from the stated totals events.
- **`ToCheckRow`**, one per exception: document, reason code, a sentence a person can act on, and the page where it applies.
- **`Manifest`**, the exact selection, the counts, the automation rate, the notice and the software version.

Rules the model enforces:

- **Only current events count.** A superseded event is ignored, using the existing current-event view, so a person's correction wins over the model's first reading.
- **Kind:** `credit_note` when a current `credit_note_referenced` event exists, otherwise `invoice` when there is an invoice number or stated totals, otherwise `unclassified`.
- **One currency per document.** The currency comes from the stated totals, otherwise from the lines. If one document's current money events disagree, the currency cell is blank and the document is flagged `mixed_currency`.
- **Blanks stay blank.** A document with no stated totals has empty net, VAT and gross cells and is counted under "no stated totals" on To check. It is never shown as zero.
- **No cross-currency sums.** Every group is per currency. There is no grand total across currencies.
- **Credit notes are separate.** They are never netted into invoices. Whether a credit note reduces input or output VAT depends on the tenant's role, and ClearCut does not guess.
- **Duplicate flags, not deletions.** A current `possible_duplicate` event puts the earlier document's id in the row. Both documents stay in the register.
- **Deterministic order:** invoice date ascending (undated last), then supplier, then invoice number, then document id.
- **No invented facts.** In particular, whether amounts include VAT is not stored as an event today, so the register does not have that column. It shows the `amounts_include_vat` review reason when one exists. Adding the fact itself would be a reading-shape change and needs its own ADR.

## 6. The VAT summary

Two tables, both per month, currency and kind, because stated totals cannot be split across tax categories without guessing:

- **By category (calculated):** month, currency, kind, category, rate, documents, taxable, VAT. These come from the current calculated VAT events, which are already per category and per rate.
- **Stated totals:** month, currency, kind, documents with stated totals, stated net, stated VAT, stated gross. These come from the stated totals events.

Alongside them, a reconciliation block per month and currency: the sum of calculated VAT beside the stated VAT, the difference, and how many documents disagree. A difference is shown, never smoothed. Rounding is exact integer cents throughout.

Only documents that ended `done` contribute to the ready figures. A document awaiting review is never mixed into them. Each group therefore has two parts: ready documents and documents awaiting review, with a document count and amount for each, so a reader can see exactly which part is verified. When `only_ready` is set, the awaiting-review part is left out entirely and the manifest says how many documents were left out.

## 7. Formats

- **XLSX** (default): sheets **Register**, **Lines**, **VAT by category**, **Stated totals**, **To check**, **Manifest**. Money is written as a number from exact cents. Every text cell read off a document is written as a string, never a formula.
- **CSV bundle** (`.zip`): `register.csv`, `lines.csv`, `vat-by-category.csv`, `stated-totals.csv`, `to-check.csv`, `manifest.json`. UTF-8, no BOM, the existing formula neutralising on every text cell. A CSV cannot hold several tables, so the bundle is the honest form.
- **JSON:** the whole `Register` model, which is also what the preview uses.
- **PDF, HTML, Markdown, plain text:** the register, the summary and the exceptions, with the notice. They do not print every line of every document; the workbook and the bundle are for that.
- **Word is not offered for a register.** A workbook and a bundle are the hand-off formats; per-document Word stays available.

Every file starts with the notice that it was made from what was read and checked, and that figures that matter should be checked against the original.

## 8. API and limits

`POST /v1/register`, scope `documents:read`:

```json
{
  "from": "2026-03-01",
  "to": "2026-03-31",
  "document_ids": ["..."],
  "format": "xlsx",
  "only_ready": false
}
```

- `from` and `to` are optional dates in `YYYY-MM-DD`, given together or not at all; both inclusive; `to` before `from` is a `400`.
- `document_ids` is optional and takes precedence over the period.
- `format` defaults to `xlsx`; unknown formats are a `400`.
- The response is the file itself, with `Content-Disposition: attachment; filename="clearcut-register-<from>-to-<to>.<ext>"` (or `clearcut-register-selection.<ext>` for an explicit list).
- It is read-only and needs no new store of its own: the file is built in memory and returned. No background job, no temporary file.
- **Limits:** 500 documents and 50 000 line rows. Over either, `400` with a sentence asking the caller to split the period. The limits protect memory and response time, not spending.
- `404` if an explicitly named document is not the tenant's. `503` without a store. `403` without the scope. The existing rate limiter applies.
- Logging records the tenant, counts and format, never supplier names or amounts.

The store gains one selection method that loads, in three queries, the documents of the period (or the listed ids), their current events, and their page states. The SQL date filter uses a new generated `event_date` column on `events`, so a large account does not read every event to find a month. That filter is only a candidate set: the core applies the period to each document's authoritative date and drops any candidate outside it, so the two can never disagree. The migration is new and additive; no applied migration is edited.

## 9. The screen

A **Register** tab beside Documents.

1. **Period:** This month, Last month (default), This quarter, or a custom range. A selection by id from the documents list is also offered.
2. **Preview:** the register table, the totals, the automation rate for the period, and the count that needs a person. The exceptions are visible before any download.
3. **Download:** Excel, CSV bundle, JSON, PDF. One click; the filename says the period.
4. **Bulk intake:** a separate control that accepts several files, uploads them one after another, and shows a row per file with its result. The existing single-file path stays.

The copy is plain and does not promise accuracy it does not have. Light mode, keyboard reachable, no em dashes, no inline handlers, the existing content security policy unchanged.

## 10. Bulk intake

The upload screen accepts several files at once:

- Files are uploaded one at a time through the existing endpoint, because the pipeline reads one document per account at a time. The queue shows each file as waiting, uploading, reading or finished, polling the document's status for the reading step.
- A failure on one file does not stop the rest. A failed file can be retried.
- A selection is capped at 20 files per batch. Each document is sent to Anthropic and costs money, and the screen says so before the upload starts.
- No API change, no new credential, no change to the pipeline.

## 11. Honesty and safety

- Every figure comes from a stored event or a stored page state. The register adds arithmetic across documents and nothing else.
- A summary figure can name the documents behind it; the manifest lists them.
- The file contains personal and commercial data the tenant already holds. The screen says the download contains what was read from the documents, and the existing retention rules are unchanged.
- Text from a document is data: it is escaped in HTML and PDF, kept as text in a spreadsheet cell, and neutralised in CSV, exactly as the per-document exports already do.
- No model is called and no document leaves the server to build a register.
- The automation rate is computed from statuses, not estimated.

## 12. Testing

- **Model:** synthetic events for documents with stated totals, no totals, a credit note, mixed currencies, a duplicate, an inclusive page, a superseded event, no date and a still-reading status. Assert blanks, grouping, ordering, flags and that credit notes never net.
- **VAT:** calculated sums equal the event sums; a stated difference is shown, not smoothed; no cross-currency addition; awaiting-review figures are separate.
- **Renderers:** sheet names and headers; no formula cell in the workbook XML; CSV neutralises `=`, `+`, `-` and `@`; HTML and PDF escape; JSON matches the model; the notice is present.
- **Store:** period selection by invoice date (including an invoice uploaded months later), current events only, tenant isolation with two tenants, limits.
- **Endpoint:** `200` per format with the right content type and filename; `400` on bad dates, bad format and over-limit; `404` across tenants; `403` without scope; `503` without a store.
- **Screen:** browser run with a real key against the local stack, one period, every download opened; the preview and the downloaded JSON agree; keyboard and 390-wide checks.
- **Bulk intake:** three files, one refused, the other two finish; a retry does not upload twice.
- Gates as usual: `cargo fmt --all --check`, clippy with warnings as errors, the workspace tests with `DATABASE_URL`, the contract test, and the deployed check after merge.

## 13. Delivery slices

Each is its own small PR, with docs and the worklog in the same PR.

1. **Core model** and its tests.
2. **Store selection:** the additive migration, the generated date column, the batched source query and database tests.
3. **Layouts:** workbook, CSV bundle, JSON, PDF, HTML, Markdown, text, with golden tests.
4. **Endpoint and limits**, with contract and database tests.
5. **Register screen**, preview and downloads.
6. **Bulk intake**, independent of 1 to 5.

Docs touched: `docs/ARCHITECTURE.md`, `docs/LAUNCH.md`, `docs/PHASES.md` (P4-08 closes with slices 1 to 5), `docs/WORKLOG.md`.

## 14. Deferred

Native accounting files, a VAT201 form, input and output classification, statement support (needs samples and an ADR), line categorisation, scheduled or emailed registers, stored export archives, and any relaxation of a review rule. Each has a reason above.

An archive of per-document files (a zip of the existing single-document exports) is also deferred. The workbook and the CSV bundle answer the hand-off need; an archive can reuse the per-document renderers later if a customer asks for one file per document.

## 15. Decisions recorded here

1. The register is cross-document and period-first; per-document exports are unchanged.
2. Period means the invoice date, read from the events, not the upload date.
3. Undated documents are excluded from period figures and listed as exceptions, never dropped.
4. Needing review marks a document and separates its figures; it does not hide it by default.
5. Credit notes stay separate and are never netted.
6. The VAT summary shows calculated VAT by category and stated totals beside it, with any difference.
7. CSV is a bundle of one table per file.
8. The automation rate is reported for every period and used to decide, with evidence, whether a review rule should change.
