# ADR-0010: The gold set, its labelling format, and the evaluation harness

Status: **Accepted** (approved by Kone Tshivhinda, 2026-10-22, as written) · Date: 2026-10-22 · Deciders: Kone Tshivhinda, Claude
Design for tasks P2-01 (tooling) and P2-10. Build order is at the end.

## Context
[Research 05 §7](../research/05-ingestion-narration-export.md) makes a gold set of 200+ real South African invoices "non-negotiable", with one release-blocking target: **zero silent critical errors**. Everything downstream (prompts, model tiers, the escalation ladder, confidence thresholds) can only be tuned against it. Collecting and labelling real invoices is the long pole, and it depends on people, consent and POPIA care rather than on code, so it should start before the pipeline exists.

Three facts shape the design:
1. **The repository is public** (Apache-2.0). Real invoices contain personal and financial information. They must never be committed, and nothing in a report or a test may print their content.
2. **What the system consumes is already defined.** `clearcut_core::extraction::Extraction` is the shape an extractor reports; the labels should be the *same facts in the same shape*, so a score is a plain comparison and no second schema can drift.
3. **"Silent" is the whole point.** Plain accuracy treats a wrong total that was flagged for review the same as one that sailed through. They are not the same. The harness has to measure the system's *disposition* (did it say "a person must look"?) as well as its values.

## Proposal in brief
1. **The gold set lives outside the repository**, in a private, encrypted location. The repository holds only the format, the tools, and a small **synthetic** sample set that CI uses.
2. **One folder per case**: the anonymised document, a `label.json` with the true values, and consent and provenance metadata.
3. **Labels are written in decimal text** (`"1500.00"`, `"10"`) and converted exactly to cents and thousandths by the loader. Nobody labels in cents by hand, and nothing passes through a float.
4. **A new crate `clearcut-eval`** with a command-line tool `clearcut-gold`: `check` validates a gold set, `score` compares a run's results with the labels and applies a gate.
5. **The score reports silent errors first.** A wrong critical value with no flag is a *silent critical error*; the gate fails on any.
6. **Reports never contain document content**: case ids, field names and counts only, unless explicitly asked on a local run.

## The case folder
```text
gold/                       (outside the repo, for example D:\clearcut-gold\ on an encrypted volume)
  g-0001/
    document.pdf            the anonymised file, exactly as it would be uploaded
    label.json              the truth, below
  g-0002/ ...
```
Case ids are opaque (`g-0001`), never a supplier or client name.

### `label.json`
```json
{
  "format": 1,
  "case": "g-0001",
  "document": { "file": "document.pdf", "sha256": "<hex>", "pages": 2 },
  "tags": ["text-layer", "multi-page", "tax-invoice", "vat-standard"],
  "split": "dev",
  "provenance": {
    "tier": "consented",
    "consent_ref": "C-017",
    "anonymised": true,
    "retain_until": "2031-03-31",
    "labelled_by": "KT",
    "reviewed_by": null
  },
  "truth": {
    "currency": "ZAR",
    "invoice_number": { "value": "INV-000123", "page": 1 },
    "invoice_date":   { "value": "2026-03-31", "page": 1 },
    "supplier":  { "value": { "name": "Acme Advisory (Pty) Ltd", "vat_number": "4123456789", "address": "1 Main Rd, Rosebank" }, "page": 1 },
    "recipient": { "value": { "name": "Client Co", "vat_number": "4123456780", "address": "2 Side St, Sandton" }, "page": 1 },
    "lines": [
      { "page": 1, "description": "Consulting", "quantity": "10", "unit_price": "1500.00",
        "line_total": "15000.00", "tax": "standard" }
    ],
    "stated_totals": { "net": "15000.00", "vat": "2250.00", "gross": "17250.00", "page": 2 },
    "due": { "date": "2026-04-30", "terms": "30 days", "page": 2 }
  },
  "expect": { "findings": [] }
}
```
- `truth` mirrors `Extraction`: the same fields, optional where `Extraction` is optional, with **no confidence** (a person is certain by definition; the loader supplies the constant). `page` is required so citation accuracy can be scored; positions on the page can be added later.
- **Amounts are decimal text**, at most 2 decimals for money and 3 for quantities, converted exactly. Anything else is rejected, not rounded.
- **`expect.findings`** says what the validator *should* report for this document. Real suppliers make mistakes (a printed VAT that is not 15%, a total that does not add up). Those documents are valuable, so the label records the document as it is, and `expect` records which findings are correct (for example `"totals_mismatch:vat"`). The default is none.
- **`tags`** let results be broken down: `text-layer`, `scan`, `photo`, `multi-page`, `long` (over 10 pages), `credit-note`, `statement`, `handwritten`, `zero-rated`, `exempt`, `no-vat`, `foreign-currency`, `unknown-supplier`, and so on.
- **`split`** is `dev` or `test`. The split is made **by supplier, not by document** (about 70/30), so a layout seen while tuning cannot leak into the test set. Prompts and thresholds are tuned on `dev`; `test` is the release gate and is run sparingly.

## What a run reports
The runner (later `clearcut-gold run`, using the real pipeline) writes one `results/<case>.json` per case:
```json
{ "case": "g-0001", "disposition": "done", "extraction": { ... an Extraction ... },
  "findings": [ ... ], "model": { "name": "...", "prompt_version": "..." }, "usage": { "input_tokens": 0, "output_tokens": 0 } }
```
`disposition` is `done` or `needs_review`: the two terminal states a job can have.

## Measuring: the outcome of every critical field
**Critical fields:** invoice number, invoice date, currency, supplier name and VAT number, the stated net, VAT and gross totals, and each line's quantity, unit price and total. **Non-critical:** descriptions, addresses, terms, recipient details, due date.

Each field of each case gets one outcome:

| Outcome | Meaning |
|---|---|
| **correct** | Matches the truth after normalisation (trimmed, whitespace collapsed, names and descriptions case-folded; numbers, dates and ids exact) |
| **wrong, flagged** | Differs from the truth, but the system said a person must look |
| **wrong, silent** | Differs, and the system did not flag it. **A silent critical error.** |
| **missing, flagged / silent** | The truth has a value; the system gave none |
| **spurious** | The system gave a value; the truth has none |

**"Flagged" has one definition:** the case ended `needs_review`, **or** the validator produced a finding about that field or the document's totals, **or** the field's confidence is under the review threshold (a setting, so that P2-09 can tune it). Anything else is silent.

**Reported, in this order:**
1. **Silent critical errors**: the count, with the case ids and field names (never the values). The target is **zero**; the gate fails on one.
2. Critical-field accuracy before review (research 05's baseline, first measured, then raised).
3. Review rate: the share of cases ending `needs_review`.
4. Validation agreement: findings reported versus `expect.findings` (precision and recall).
5. Completion: every case reached a terminal state (target 100%).
6. Citation accuracy: the page cited for a value is the page it is on.
7. Cost per document in Rand, from token usage (at the assumed R18/USD), once an adapter reports it.
8. All of the above **per tag**, so a regression on scans cannot hide behind good text-layer results.

**The gate** (a small config file) defaults to: zero silent critical errors on `test`; the accuracy and completion targets from research 05; and "no tag worse than N points below the overall figure".

## The tool: `clearcut-gold`
- **`check <gold dir>`**: every folder has its files; the document's SHA-256 and page count match; the label parses and converts exactly; the truth, run through `validate`, produces exactly `expect.findings`; ids are unique; every case has a split, a provenance tier and, unless synthetic, a consent reference and a retention date; no field that should be anonymised looks like an unmasked ID number, bank account number or email address; the dev and test splits share no supplier; and a coverage table shows how the set compares with the target mix.
- **`score <gold dir> <results dir>`**: the report above, as JSON and Markdown, plus the gate's verdict as the exit code.
- **A synthetic sample set** in the repository (about 15 cases, generated deterministically) with deliberately flawed results, so the scorer's own rules have golden-file tests: a correct field, a wrong-and-flagged one, a wrong-and-silent one, a missing one, a spurious one, a per-tag breakdown, and the gate failing on a single silent error.

## Privacy and consent (needs legal review before real documents are collected)
The rules that make the set safe to build, drafted as a checklist the tooling then enforces where it can:

1. **Where real documents come from.** Only (a) documents ClearCut's operator owns, (b) documents whose issuer and recipient have given written consent, or (c) synthetic or properly licensed public samples. Nothing scraped, nothing "found". Each case records its tier (`own`, `consented`, `synthetic`) and, for `consented`, a consent reference.
2. **Consent covers the actual uses**: evaluating the software's accuracy; storage encrypted on a named device or bucket; **processing by a model provider outside South Africa (POPIA s72)** *only after that provider's retention and zero-data-retention terms are verified* (P2-19); no other sharing; withdrawal at any time with deletion of the case.
3. **Anonymise before a document enters the set**, with realistic substitutes so the test stays meaningful: names, addresses, telephone numbers, emails, bank details and ID numbers are replaced by plausible fictional values of the same shape (a valid-looking VAT number, a plausible address), consistently within a case. Logos that identify a person or business are replaced by a neutral block. A pseudonym map, if any is kept, is stored apart from the set, encrypted, and deleted when the consent says so.
4. **Storage.** Outside the repository, on an encrypted volume or a private bucket with a named access list. A CI guard fails if a document file appears anywhere in the repository outside the approved fixture folders, and `.gitignore` excludes the gold folder as a second net. (GitGuardian cannot see a PDF.)
5. **Nothing leaves in the open.** Reports and logs carry case ids, field names and counts, never values or file names. A local `--show-values` is the only exception.
6. **Retention and deletion.** Every non-synthetic case has a `retain_until` date. Deleting a case removes its folder; only aggregate scores remain.
7. **Until P2-19 clears, evaluation runs only on synthetic and own non-sensitive documents.** No consented real invoice is sent to any external API before that.
8. **Information Officer.** Whoever operates ClearCut commercially must be named before collection starts beyond one's own documents (an open item).

## Composition and process (targets for Kone to adjust)
- **Start with 25 cases** to get the loop running, grow to **200+**.
- **Mix:** about 40% text-layer PDFs; 30% scans and photos; 10% multi-page or long; 5% credit notes; 5% statements; 5% handwritten or annotated; 5% e-invoice XML. At least **60 distinct suppliers**; standard-rated, zero-rated, exempt and no-VAT suppliers all present; at least **10 documents whose printed totals are wrong**, because they test the validator's value; some in a foreign currency.
- **Labelling.** One person labels; a second reviews every critical field of at least 20% of cases, and the disagreement rate is reported (a high one means the labelling guide is unclear, not that the labeller is wrong). Test-split cases are labelled **from scratch**. Dev cases may be pre-filled by a model and corrected by a person to save time, because pre-filling a test case would bias the score toward the model that wrote the draft.

## What gets built, in order (each its own small PR, tests first)
1. **`clearcut-eval`: the format.** Types, the exact decimal loader, `check`, and the synthetic sample set.
2. **`clearcut-eval`: scoring.** Outcomes, the silent-error rule, the report, the gate, with golden-file tests. ✅ (see the step 2 notes below)
3. **The collection kit** in `docs/gold-set/`: the labelling guide, a consent form template, the anonymisation guide, the checklist, and the CI guard and `.gitignore` entries. ✅ (see the step 3 notes below)

## Implementation notes (step 1, `clearcut-eval`)
- **Example corrected.** The label example above had totals that did not add up and a recipient too thin for a full tax invoice (over R5 000 the rules need the recipient's address and VAT number). It is now a document the validator accepts cleanly, which a test checks.
- **The crate** has a library and the `clearcut-gold` command (`check <folder>`, `sample <new folder>`). Exit codes: 0 fine, 1 problems found in the gold set, 2 wrong usage, 3 a folder could not be read or written.
- **Strict labels.** Every struct rejects unknown fields (a typo is an error, not a dropped value) and amounts must be JSON **strings** (a JSON number would pass through a float parser). `1,500`, `1.005`, `01.5`, `-0` and anything that does not fit an `i64` are rejected, never rounded; `i64::MIN` is refused too. Dates are real dates, tax categories and currencies must be known words.
- **Reports never contain content.** A parse error says "line 14, column 9", never what was there; a bad amount is reported as `lines[2].unit_price` with the kind of mistake; folder names that are not case ids are counted ("folder number 2"), never printed. A test builds a set full of marker words and asserts none reaches the report. That test found a real leak while it was being written: malformed tags were being counted into the "Tags:" line of the coverage table. Only well-formed tags are counted now.
- **Free text in `expect.findings` is never echoed.** Entries must match the finding-key vocabulary (`no_lines`, `totals_mismatch:vat`, `line_total_mismatch:2`, `compliance:supplier_address`, ...); anything else is reported by position (`BadExpectedFinding`) and left out of the comparison, so a pasted sentence cannot reach a report. Found by the automated security review of PR #34. A test keeps the vocabulary in step with what the validator produces.
- **What `check` verifies** (beyond the list above): a case folder holds exactly `label.json` and the document (anything else is counted as unexpected); the document must be a plain file (a link or folder is refused, so a label cannot point outside its folder) and its SHA-256 must match; a label over 1 MiB is refused unread; the same document in two cases is reported; `consent_ref`, `retain_until` and `anonymised: true` are required for `own` and `consented` cases; a supplier (by VAT number **or** by normalised name) may not appear in both splits.
- **Not verified yet: the page count against the PDF itself.** `check` confirms that every cited page is within the declared page count, but not that the declared count is the file's real one: that needs a PDF reader, and the right one is the worker through `clearcut-docproc`. It joins the check when the pipeline slice reads documents.
- **The personal-data net looks for shapes** in free-text fields: an SA ID number (13 digits, a valid date, a valid check digit), an email address, or nine or more digits in a row or in groups of three or more. It cannot see a person's name. Invoice numbers and VAT numbers are not scanned, since long digit strings are normal there.
- **The synthetic sample set** is 15 invented invoices with real PDF text layers (a small deterministic PDF writer; read back with the worker's own PDF library to confirm): every tax category, a multi-page document, a credit note, USD, a rounding case (2.5 x 99.99), and planted mistakes (wrong printed VAT, wrong line total, due date before invoice date, missing supplier address, a non-VAT supplier) each with the exact findings expected. 10 dev and 5 test cases from 13 suppliers. It is checked in under `crates/clearcut-eval/sample`, and a test fails if the files differ from what the generator produces.
- **Mutation-checked** 100 rules in the loader, label rules, checker, scanner, command line and generators. Eleven did not die first time: nine were real test gaps (a non-digit after the dot, a 256-character file name, a blank-but-spaced reviewer, the checksum's carry, the two group-length rules and the gap rule of the long-number scanner, a single unexpected file, and cross-case problem ordering) and each got a test; one led to a small design change (file names compare case-insensitively only on Windows); two had patterns that no longer matched after formatting and were checked by hand. All caught now.

## Implementation notes (step 2, scoring)
- **Command:** `clearcut-gold score <gold> <results> [--split dev|test|all] [--gate gate.json] [--json report.json]`. Exit 0 if the gate passes, 1 if it fails (or the gold set has problems), 2 usage, 3 unreadable. The default split is **test**. A gold set that fails `check` is refused, because a score against bad labels means nothing. `--json` never overwrites an existing file.
- **Results format:** `results/<case>.json` holds `case`, `disposition` (`done` or `needs_review`), an optional core `Extraction`, and optional `model` and `usage`. Unknown extra fields are ignored. A missing file is "no result" (it counts against completion; no note), an unreadable or malformed one is noted by case and position, never content. Files over 8 MiB, links and folders are refused.
- **Decisions the ADR left open (please check these):**
  1. A **missing or invented** critical value that nobody flagged is a silent critical error too, not only a wrong one: a `done` invoice with no totals is as dangerous as one with wrong totals.
  2. A credit note's **amount** is critical (it is money); its original invoice number and a line's **tax category** are not.
  3. The scorer **recomputes the validator's findings** from the extraction with the pipeline's own code. A run's own "findings" are ignored, so it cannot hide one.
  4. A printed-vs-recomputed **totals mismatch flags that total and every line's quantity, price and total**, because the mistake may be in either; a line problem flags that line's three amounts; an SA field rule flags the field it names.
  5. **Lines are matched by position.** A dropped or invented line in the middle makes every later line differ: it over-reports, the safe direction.
  6. A case with **no result** counts its fields as missing-but-flagged (the gate already fails on completion). A finished case with **no extraction** is missing everything silently. An **empty selection fails** the gate rather than passing vacuously.
  7. **Review threshold** default is confidence below 800 (of 1000): a placeholder until P2-09 tunes it. It is a gate setting.
  8. **Cost:** the report sums tokens per case; the Rand cost per document arrives with the Anthropic adapter, which has the prices.
- **Gate file** (JSON, unknown keys refused, defaults are the research 05 targets): `split`, `max_silent_critical_errors` (0), `min_critical_accuracy_bp` (9700), `min_noncritical_accuracy_bp` (9500), `min_completion_bp` (10000), `max_tag_shortfall_bp` (1000), `min_tag_cases` (5), `review_confidence` (800). Percentages are in basis points (hundredths of a percent), compared **exactly** as integer ratios; the report prints them rounded down.
- **Sample results** (`crates/clearcut-eval/sample-results`, written by `clearcut-gold sample-results`) are flawed on purpose: the test split is read perfectly (the gate passes); the dev split has a wrong VAT number nobody flagged and a dropped credit-note reference (two silent critical errors, the gate fails), plus a wrong price with low confidence, a missing invoice date, a wrong total in a case sent to review, an invented recipient, and a description with different casing that still counts as correct. Tests and CI can run the whole path on the repository alone.
- **Reports are content-free** (case ids, field names, counts), proved by a test that fills a result with marker words and checks none appears in the Markdown, the JSON or the debug output.
- **Mutation-checked** 116 rules across the tally, field names, finding-to-field mapping, comparison, scoring loop, gate, loader, report and command line. Ten did not die first time: eight were test gaps (the per-tag non-critical tally, a tag exactly at the allowed shortfall, a broken result outside the selected split, precision and recall being swapped, an unknown flag, overwriting a report file, a gate file of exactly 64 KiB) and each got a test; one was a sample flaw (low confidence) that another finding was also catching, so it now rests on the low confidence alone; one (refusing a link in place of a result file) can only be tested on Linux: its test is `cfg(unix)` and runs in CI, not on this Windows machine. All others caught.

## Implementation notes (step 3, the collection kit and the guard)
- **`clearcut-gold guard <repository>`** (run by CI as the new `gold-set` job) fails if a **document** (PDF, image, Office file, archive, email) is anywhere in the repository outside the approved folders listed in `gold-guard.json`; if a file **starts like** a document whatever its name (PDF, PNG, JPEG, GIF, TIFF or ZIP signature); if a gold-set **`label.json`** is outside those folders, or inside one but **not synthetic** (so a real case cannot hide among the samples); or if `.gitignore` no longer lists `/gold/` and `/gold-*/`. Approving a new folder means editing `gold-guard.json`, so it shows up in review. The report names the offending path (already visible in the repository), never anything inside a file.
- **In a git working copy the guard judges what git would commit** (tracked files plus untracked ones that are not ignored); elsewhere it walks every file, skipping build output, dependencies and caches. Links are never followed. This is why a local, git-ignored archive in the working copy is not a finding.
- **`clearcut-gold new-case`** starts the next case: it picks the next id, copies the document in as `document.<ext>` (the original name is never kept; the original is untouched), computes the SHA-256 and writes a `label.json` to fill in. The new case is well formed but **visibly unfinished**: a real document starts as "not anonymised" until the labeller says otherwise, and the empty label fails `check`. It does not read PDFs, so the page count is given with `--pages`.
- **The kit** is in [`docs/gold-set/`](../gold-set/README.md): a README with the workflow, the labelling guide (field by field, `expect.findings`, the open questions it cannot answer), the anonymisation guide (including why a black box over text is not redaction, and a check-before-done list), a **draft** consent form that **needs an attorney's review** before use, and checklists (before the first real document, per document, per batch, on withdrawal).
- **Open questions the labelling guide hands back to Kone:** credit notes that print negative amounts, documents that print only VAT-inclusive prices, statements, and files holding several invoices.
- **Mutation-checked** 62 rules in the guard (including the file signatures), new-case and the command line. Ten survived the first pass: a GIF87 signature with no test, a redundant empty-path check (removed), a link whose target looks like a document (a `cfg(unix)` test, runs in CI), an over-long extension (its test had been lost in an edit and was restored), the next-id rule (restructured to take the maximum, so order cannot matter), a case-insensitive file system hiding a wrong-case extension (the test now compares stored names), the tier and split reaching the label (a new test), and clean-up after a failed write. All are now caught except the last, which cannot be provoked in a test on this machine. A review also asked for more signatures (old Office files, gzip, 7z, BMP, WebP); they are added, with a BMP check strict enough that text starting with "BM" is not mistaken for a picture.

## Alternatives considered
- *Keeping the gold set in the repository.* Rejected: the repository is public and the documents are personal and financial data.
- *Labels as raw `Extraction` JSON in cents.* Rejected: error-prone to write by hand, and a confidence field a human has no use for. The loader converts exactly instead.
- *A single accuracy number per document.* Rejected: it hides exactly what matters (a flagged error and a silent one count the same).
- *Splitting dev and test at random.* Rejected: the same supplier's layout would appear on both sides and flatter the score.
- *Using the model's own output as the labels.* Rejected for the test split: it would measure agreement with itself.

## Questions for Kone
1. **The gold set lives outside the repository** (an encrypted folder you choose, later a private bucket), with only the format, tools and a synthetic sample in the repo?
2. **Decimal-text amounts** in label files, converted exactly by the loader?
3. **Provenance tiers** `own`, `consented`, `synthetic` as the only allowed sources? Do you have a first source of real invoices (your own, or people who would consent)?
4. **Dev/test split by supplier, about 70/30**, with the test split labelled from scratch?
5. **The definition of "flagged"** (case ended `needs_review`, or a validator finding about it, or confidence under the threshold) and of **critical fields** as listed?
6. **Gate defaults**: zero silent critical errors on the test split, the research 05 targets otherwise?
7. **Composition targets**: start with 25 cases, grow to 200+, at least 60 suppliers and 10 documents with wrong printed totals?
8. **Names**: crate `clearcut-eval`, command `clearcut-gold`?
