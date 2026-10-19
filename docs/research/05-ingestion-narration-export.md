# 05 · Ingestion, understanding, narration, grouping and export

_Author: Kone Tshivhinda · Researched 19 Oct 2026 · Written by Kone Tshivhinda & Claude_

**Requirement (from Kone):** someone uploads an invoice, of any length. ClearCut must accept it, actively read and understand it, **narrate the events** in it, **organise those events into groups**, and let the user **export** the result in various formats.

This file was added after a gap review showed research 01–04 covered only *issuing* invoices. Ingestion is now the **first** product capability.

> **Source quality.** Accuracy and cost numbers below come from vendor blogs and one under-specified paper. Use them for direction, not as targets. We must **benchmark on real South African invoices** (see §7).

## 1. How machines read invoices today

| Approach | Strengths | Weaknesses | Evidence |
|---|---|---|---|
| **Structured data already inside the file** (UBL/CII XML, Factur-X/ZUGFeRD hybrid PDF) | Exact, free, no AI | Rare in SA today; mandate is years away | [Hybrid invoice formats](https://econnect.eu/en/docs/knowledge/document-formats/basics/hybrid-invoices) |
| **Text layer of a born-digital PDF** | Fast, exact characters | Layout ambiguity; tables are hard | — |
| **Classic OCR** (Tesseract, cloud OCR) | Cheap, fast (50–200 ms/page claimed) | Needs templates per supplier; poor on messy scans (−10–20%) | [RaftLabs](https://www.raftlabs.com/blog/ocr-vs-llm-invoice-processing) |
| **LLM / vision-language models** | Handle unknown layouts; understand context (terms, PO refs) | Slower, costlier, can **hallucinate numbers** (1–3% claimed), not reproducible run-to-run | RaftLabs |
| **Hybrid** | Best claimed accuracy at lower cost | More moving parts | RaftLabs, [arXiv 2511.05547](https://arxiv.org/pdf/2511.05547) |

Claimed document-level accuracy (RaftLabs, **vendor, unverified methodology**): OCR 85–92% known formats / 60–75% unknown; LLM 92–97% / 88–95%; hybrid 95–98% / 90–96%. The arXiv paper reports 95–97% end-to-end but names no dataset and gives no per-field metrics, so we do not rely on it.

**Indicative cost per document** (vendor ranges, converted at an **assumed R18/USD**): OCR R0.18–R0.90; LLM R1.80–R9.00; hybrid R0.90–R3.60; manual handling roughly R113–R153 per invoice (the source gives $6.30–$8.50 and is inconsistent). Even the dearest AI path is far cheaper than manual entry; accuracy and audit matter more than price.

### Conclusion
Use a **cascade, cheapest-reliable-first**:
1. Embedded structured XML (exact) → 2. text layer + rules for known suppliers → 3. OCR where there is no text layer → 4. LLM/vision for structuring and unknown layouts → 5. **deterministic validation** → 6. **human review** for low confidence.

## 2. "No matter how long"

Claude's PDF support converts each page to an image plus extracted text, at roughly **1 500–3 000 text tokens per page** plus image tokens, with limits of **32 MB per request and 100 pages (600 on 1M-token-context models)**; large files go through the Files API; prompt caching and a Batch API exist ([Anthropic docs](https://platform.claude.com/docs/en/build-with-claude/pdf-support)). Dense pages can fail below the page limit.

Therefore "any length" is an **architecture** requirement, not a model feature:
- **Chunk by page/section**, process in parallel (map), then reconcile (reduce): totals, page-spanning tables, carried-forward balances.
- **Asynchronous jobs** with progress, retries and resumability; uploads are resumable and go to object storage, never through the request body.
- **Page-level provenance** so a 400-page statement can be re-processed page 212 only.
- Cost guardrails: token estimate before run, per-tenant budgets.
- Long-context models lose accuracy on mid-document facts ("lost in the middle"; the PHANTOM benchmark shows detection of fabricated answers degrades with context length, per [this summary](https://beancount.io/bean-labs/research-logs/2026/04/19/phantom-hallucination-detection-financial-long-context)). **Smaller chunks plus reconciliation beat one giant prompt.**

## 3. Validation: never trust the extractor

Deterministic checks (Rust, `clearcut-core`) run on every extraction:
- Line totals = quantity × price; lines sum to net; VAT = 15% per category (we already have this engine); gross = net + VAT.
- SA rules: VAT-number format, threshold-dependent required fields, 21-day issue window, credit/debit note wording.
- Duplicate detection by content hash and (supplier, number, date, total).
- Currency/date sanity, supplier-in-master-data checks.

A failed check lowers confidence and routes to review; it never silently "fixes" the number.

## 4. Events: what we narrate

An **event** is an atomic, dated, sourced fact. Candidate types:

| Group | Event examples |
|---|---|
| Document | Received, classified, pages processed, extraction completed |
| Parties | Supplier identified, recipient identified, VAT number verified |
| Charges | Line item billed, discount applied, fee added |
| Tax | VAT calculated per category, zero-rated/exempt line |
| Totals | Net, VAT and gross stated; **stated vs recomputed mismatch** |
| Terms | Due date, payment terms, late-fee clause, bank details |
| Adjustments | Credit note, debit note, correction referencing an earlier invoice |
| Payments | Payment recorded, partial payment, overdue |
| Compliance | Missing field, threshold requirement, 21-day breach |
| Risk | Possible duplicate, bank-detail change, unusual amount |

Each event has: id, type, group, timestamp (document date, not processing date), actor (system/human/model), structured payload (integer cents), **citation** (file, page, text span or bounding box), extractor and confidence.

**Grouping** is two-level: a fixed **taxonomy** (table above) for stable organisation, plus **views** by date, party, document or severity. Grouping is deterministic code over event types, not an LLM judgment.

## 5. Narration without making things up

LLMs invent numbers, and a narrated invoice is a financial statement. Design rules (informed by grounding/citation approaches in [this review](https://beancount.io/bean-labs/research-logs/2026/04/19/phantom-hallucination-detection-financial-long-context)):

1. **Narrate from events, not from the PDF.** The narrator only sees verified, structured events.
2. **Template first, model second.** Facts are rendered by deterministic templates ("Supplier X billed R15 000.00 for 10 × Consulting"). An LLM may only smooth the prose between facts.
3. **Every sentence carries event IDs**; the UI links back to the exact page span.
4. **Number guard:** after generation, extract every numeral and date from the narrative and confirm each appears in the cited events. Any mismatch fails the narration and falls back to template-only text.
5. **Reproducibility for audit:** store model name/version, prompt hash, inputs and raw output with the job.
6. **Audience modes:** one-paragraph summary, chronological timeline, grouped report.

A verifier that is a different (or larger) model is an optional extra, not a substitute for the number guard (small models are poor self-verifiers per the same source).

## 6. Export

| Format | Purpose | Phase |
|---|---|---|
| **PDF report** | What most people open and share: narrative, grouped events, citations | 4 |
| **Excel (.xlsx)** | Sheets: Summary · Events (by group) · Line items · Checks · Sources | 4 |
| **Word (.docx)** | Editable narrative report for advisers and clients | 4 |
| **CSV** | Events and line items for any spreadsheet, import or BI tool | 4 |
| **JSON** | Canonical, lossless; the API format | 4 |
| **Plain text / Markdown / HTML** | Email bodies, notes, web pages | 4 |
| **UBL 2.1 / Peppol PINT** | E-invoicing | 6 |
| **Factur-X/ZUGFeRD (PDF/A-3 + CII XML)** | Hybrid invoice; XML is authoritative | 6 (evaluate) |
| **Accounting imports** (Xero, QuickBooks, Sage) | Hand-off to ledgers | 8 — verify each vendor's current import format first |

Exports are generated from the stored events, so they never disagree with each other.

**Decision (Kone, 19 Oct 2026):** ship the popular everyday formats, each well structured: **PDF, Excel, Word, CSV, JSON, text/Markdown/HTML**. "Well structured" means one shared **export schema** (document → groups → events → lines → checks → sources) with a fixed column/section layout per format, the same field names everywhere, ISO dates, amounts shown both as numbers (Excel/CSV/JSON cells are real numbers, not text) and formatted as Rand in the PDF/Word views, and a source citation on every row. Open formats beyond these (e.g. ODS, Google Sheets export) are deferred.

## 7. Evaluation plan (non-negotiable)

- Build a **gold set of 200+ real, anonymised SA invoices** (mixed suppliers, scans, multi-page, statements, credit notes, handwritten notes). Measure **field-level** accuracy, not "document looks right".
- **Targets (proposed; Kone has no number yet, so these are defaults to be calibrated after the first gold-set run):**

| Metric | Proposed target | Why |
|---|---|---|
| **Silent critical errors** (a wrong total, VAT, date, invoice number or supplier identity that was *not* flagged) | **0 on the gold set**; any occurrence blocks release | A flagged error is a review task; a silent one is a financial error |
| Critical-field accuracy **before** review | Measure first; aim ≥ 97% initially, then raise | Establishes the real baseline; avoids inventing a number |
| Critical-field accuracy **after** review | ≥ 99.5% | Human-in-the-loop closes the gap |
| Non-critical fields (descriptions, notes, bank details) | ≥ 95% | Lower financial risk |
| Narration | **100%** of numbers and dates traceable to cited events (number guard is a hard gate) | Narrated figures must be real |
| Completion | **100%** of jobs end in a terminal state (`done` or `needs_review`), none stuck or silently failed | Matches "every task runs to completion" |
| Review rate | Track; aim to fall over time as memory and prompts improve | Cost and speed |

Re-baseline after the first measurements; if the human baseline on the same documents is better or worse, adjust. Accuracy numbers in vendor material (§1) are not targets.
- Re-run on every prompt, model or OCR change (regression suite in CI).

## 8. Privacy and POPIA

> **Decision update:** the provider is Anthropic's API; escalation, thinking and memory design and the cross-border position are in [ADR-0005](../adr/0005-model-strategy-escalation.md).

Per [Michalsons](https://www.michalsons.com/blog/how-popia-affects-ai/76842) (secondary source; get legal review):
- **s71**: people can challenge decisions made **solely** by automated means with legal or significant effect → keep a human in the loop for consequential outcomes.
- **s10 / s13**: purpose limitation and minimisation → send models only what they need; redact bank and ID numbers where possible.
- **s72**: cross-border transfer → using a model API hosted outside South Africa needs an adequacy basis or consent; do due diligence on the provider and keep an operator agreement. Many target users (law, forensics and advisory practices) owe their clients confidentiality, which adds to this. A self-hosted or in-region extraction option should be on the roadmap.

## 9. Competitive note
Existing tools split into *AP capture* (extract and post) and *AR* (issue and collect). Few present **a narrated, cited, grouped timeline** of what an invoice says as the product. That story-with-evidence is a credible differentiator, and it fits forensics and advisory work. _(inferred; competitor scan was not exhaustive)_

## Sources
- [RaftLabs: OCR vs LLM for invoice processing](https://www.raftlabs.com/blog/ocr-vs-llm-invoice-processing) (vendor)
- [arXiv 2511.05547: hybrid LLM+OCR invoice extraction](https://arxiv.org/pdf/2511.05547) (weak methodology)
- [Anthropic PDF support](https://platform.claude.com/docs/en/build-with-claude/pdf-support)
- [econnect: hybrid invoice formats](https://econnect.eu/en/docs/knowledge/document-formats/basics/hybrid-invoices)
- [Beancount/PHANTOM hallucination summary](https://beancount.io/bean-labs/research-logs/2026/04/19/phantom-hallucination-detection-financial-long-context)
- [Michalsons: how POPIA affects AI](https://www.michalsons.com/blog/how-popia-affects-ai/76842)
