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
    "recipient": { "value": { "name": "Client Co" }, "page": 1 },
    "lines": [
      { "page": 1, "description": "Consulting", "quantity": "10", "unit_price": "1500.00",
        "line_total": "15000.00", "tax": "standard" }
    ],
    "stated_totals": { "net": "16000.00", "vat": "2250.00", "gross": "18250.00", "page": 2 },
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
2. **`clearcut-eval`: scoring.** Outcomes, the silent-error rule, the report, the gate, with golden-file tests.
3. **The collection kit** in `docs/gold-set/`: the labelling guide, a consent form template, the anonymisation guide, the checklist, and the CI guard and `.gitignore` entries.

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
