# Phases and tasks

_Author: Kone Tshivhinda · Written by Kone Tshivhinda & Claude · Revised 19 Oct 2026 after adding ingestion/narration/export ([research 05](research/05-ingestion-narration-export.md))_

Legend: ✅ done · 🔄 in progress · ⬜ not started. Estimates are rough, for one part-time developer alongside an internship.
**Product order:** read & understand → narrate & group → export → then issue, e-invoice, collect.

## Phase 0 — Foundations ✅
- ✅ P0-01 Market, compliance and MCP research
- ✅ P0-02 Architecture and ADRs 0001–0003
- ✅ P0-03 Monorepo scaffold (Rust workspace, TS package, plugin manifest)
- ✅ P0-04 `clearcut-core`: money, VAT, totals, lifecycle, ZA thresholds
- ✅ P0-05 `clearcut-server` REST skeleton
- ✅ P0-06 `clearcut-mcp` stdio prototype (3 tools)
- ✅ P0-07 TypeScript SDK skeleton
- ✅ P0-08 Apache-2.0, CI, templates, security policy
- ✅ P0-09 Public GitHub repo live, CI green, topics and security features set
- ✅ P0-10 Gap review → research 05 + ADR-0004 (ingestion, narration, export)
- ⬜ P0-11 Branch protection on `main` (when PR workflow starts)

## Phase 1 — Platform foundation (≈3–4 weeks)
- ⬜ P1-01 Postgres + `sqlx` migrations: tenants, documents, jobs, events, audit log
- ⬜ P1-02 Object storage abstraction (local disk + S3-compatible); resumable/chunked upload endpoint
- ⬜ P1-03 Durable job queue (Postgres-backed) with retries, progress and cancellation
- ⬜ P1-04 AuthN/Z: API keys, tenant isolation (row-level)
- ⬜ P1-05 Idempotency keys on all mutating endpoints
- ⬜ P1-06 Upgrade MCP to the 2026-07-28 stateless spec + Streamable HTTP; adopt Tasks extension for long jobs
- ⬜ P1-07 Docker image + `docker compose` dev stack (API, Postgres, MinIO, worker)
- ⬜ P1-08 OpenAPI from code; TS types generated from JSON Schema; contract tests

## Phase 2 — Ingestion & understanding (≈6–8 weeks) ⭐ core
- ⬜ P2-01 **Gold set:** collect and anonymise 200+ real SA invoices; label fields (with the document owners' consent and a POPIA review)
- ⬜ P2-02 Document intake: type sniffing, page count, text-layer detection, de-duplication by hash
- ⬜ P2-03 Embedded-structured-data reader (UBL/CII/Factur-X XML inside PDFs)
- ⬜ P2-04 Python worker skeleton (gRPC/HTTP), provider trait, no business rules
- ⬜ P2-05 Text-layer extraction + OCR fallback for scans
- ⬜ P2-06 LLM/vision structuring with JSON-Schema output, per-page chunking, page-level provenance
- ⬜ P2-07 Map-reduce for long documents: page-spanning tables, carried balances, statement totals
- 🔄 P2-08 **Deterministic validation** in `clearcut-core` (line/VAT/total checks, SA field rules, duplicate detection) — done: `extraction::{Extraction, validate}` (line arithmetic, stated vs recomputed totals, due-before-issue, `compliance_za` rules; exact to the cent; 27 tests). Remaining: duplicate detection (needs other documents as input)
- ⬜ P2-09 Confidence scoring + human review queue (approve / correct / reject; corrections feed the gold set)
- ⬜ P2-10 Evaluation harness in CI: field-level accuracy, "zero silent errors" gate
- ⬜ P2-11 MCP tools: `upload_document`, `get_extraction`, `review_extraction`
- ⬜ P2-12 Cost guardrails: token estimate, per-tenant budget, batch mode
- ⬜ P2-13 Anthropic provider adapter (behind the provider trait); API key via env/secret manager; per-tenant budgets
- ⬜ P2-14 **Model router + escalation ladder** (Haiku 5.5 → Sonnet 5.5 → Opus 5.5 → Fable 5.1 → human review) driven by validation failures, disagreement, low confidence, truncation; escalation policy as config
- ⬜ P2-15 Adaptive thinking + effort per tier; log tokens, tier, effort, cache hits, escalation reason per call
- ⬜ P2-16 Prompt caching for the static prefix (instructions, schema, SA rules, examples); Batch API mode for bulk uploads
- ⬜ P2-16b **Context-pack builder** ("resource completeness"): page image + text, neighbours, schema, SA rules, examples, supplier history, validator/calculator tools, previous-tier failure report; logged per call
- ⬜ P2-17 Memory store (memory tool): written only from human-confirmed corrections; path validation, size caps, expiry, per-tenant
- ⬜ P2-18 Terminal-state guarantee: every job ends `done` or `needs_review`; chaos tests (timeouts, truncation, provider outage)
- ⬜ P2-19 POPIA: redaction of bank/ID numbers before sending, privacy notice and consent text, verify retention/ZDR terms with Anthropic, legal review

## Phase 3 — Events, grouping and narration (≈4–5 weeks) ⭐ core
- ✅ P3-01 Event model in `clearcut-core`: type, group, date, payload (integer cents), citation, extractor, confidence (`crates/clearcut-core/src/event/`; immutable events, corrections via `supersedes`; 24 tests)
- ✅ P3-02 Event taxonomy and deterministic grouping; views by date, party, document, severity (`event/view`: current/timeline/by group/document/severity + derived `Severity`, 13 tests; **party view deferred to P3-03** since events have no party field yet)
- ⬜ P3-03 Event derivation from validated extractions (incl. stated-vs-recomputed mismatch events)
- ✅ P3-04 Template narrator (deterministic, cited sentences) (`clearcut-core::narration`: `Sentence` cannot exist without a citation, one template per event kind, shared `format` module for the number guard; 24 tests)
- ⬜ P3-05 LLM prose smoothing behind the provider trait
- ✅ P3-06 **Number guard:** every numeral/date in narration must exist in cited events; fallback to template text (`narration::guard`: `check` / `guard`, strict scanner for figures, signs and whole dates, citation checks; 36 tests, mutation-checked)
- ✅ P3-07 Narration modes: summary, chronological timeline, grouped report (`narration::report`: `narrate(events, mode)` returns a `Report` of titled sections; every mode passes the number guard; 16 tests)
- ⬜ P3-08 Citation UI contract: event → page span / bounding box
- ⬜ P3-09 Store model/prompt versions and raw output for audit reproducibility
- ⬜ P3-10 MCP tools: `get_events`, `narrate_document`, `group_events`

## Phase 4 — Export (≈2–3 weeks) ⭐ core
- ⬜ P4-01 Export framework: all formats rendered from stored events only
- ⬜ P4-02 Shared **export schema** (document → groups → events → lines → checks → sources) with per-format layout spec and golden-file tests
- ⬜ P4-03 PDF report (narrative, grouped events, citations)
- ⬜ P4-04 Excel: Summary · Events · Line items · Checks · Sources sheets; real numeric cells
- ⬜ P4-05 Word (.docx) editable report
- ⬜ P4-06 CSV (events, lines) and JSON (canonical)
- ⬜ P4-07 Plain text, Markdown and HTML
- ⬜ P4-08 Batch export (many documents → one workbook/zip)
- ⬜ P4-09 MCP tool: `export_document`; SDK helpers

## Phase 5 — Dashboard and review UX (≈4 weeks)
- ⬜ P5-01 TypeScript web app: upload, progress, review queue, side-by-side page + extracted fields
- ⬜ P5-02 Timeline and grouped-event views with citation jump-to-source
- ⬜ P5-03 Export menu and saved exports
- ⬜ P5-04 Audit trail view

## Phase 6 — Issuing and delivery (≈3–4 weeks)
- ⬜ P6-01 Gapless invoice numbering; PDF rendering of full/abridged tax invoices
- ⬜ P6-02 Credit and debit notes (exact labels, original-invoice reference)
- ⬜ P6-03 Policy engine: approval thresholds, void limits; agent draft → human approve → send
- ⬜ P6-04 Email delivery and payment link; recurring invoices

## Phase 7 — E-invoicing readiness (≈3–4 weeks, then tracks SARS)
- ⬜ P7-01 Map the domain model to EN 16931 business terms
- ⬜ P7-02 UBL 2.1 / Peppol PINT export; evaluate `en16931` + `en16931-formats`; Factur-X evaluation
- ⬜ P7-03 Validate against official rule sets in CI
- ⬜ P7-04 Access-point provider shortlist and adapter interface
- ⬜ P7-05 Monitor SARS draft regulations; update `research/02`

## Phase 8 — Payments, reconciliation, ecosystem, hardening
- ⬜ P8-01 Payment events + reference matching; PSP spike (PayShap Request, pay-by-bank)
- ⬜ P8-02 Bank-feed/CSV import with fuzzy matching and short-pay handling
- ⬜ P8-03 Dunning, ageing/DSO reports; payment-delay forecasting
- ⬜ P8-04 Connectors: Xero, QuickBooks, Sage (verify SA demand and import formats first)
- ⬜ P8-05 Security review, threat model (incl. prompt injection from uploaded documents), dependency audit
- ⬜ P8-06 POPIA review, retention and deletion tooling; v1.0 criteria

## Immediate next three tasks (hand-off to Claude Code)
1. **P1-01 / P1-02:** Postgres schema and upload storage (keep migrations small and reviewed).
2. **P2-01:** start the gold set now, in parallel with code. It is the long pole.
3. ~~**P3-01:** event model~~ ✅ done 19 Oct 2026. ~~**P3-02**~~ ✅ and ~~**P3-04**~~ ✅ done. ~~**P3-06**~~ ✅ done 20 Oct 2026. ~~**P3-07**~~ ✅ done 20 Oct 2026. Remaining pure-Rust in Phase 3: **P3-03** event derivation (also adds the party view; needs a minimal validated-extraction shape, ideally designed alongside P2-08). P3-05 LLM smoothing needs the provider trait from Phase 2; P3-08/P3-09/P3-10 need storage or the MCP upgrade.

## Decisions made (19 Oct 2026)
- **Boosted memory:** the model gets every resource it needs for its task (context pack); details otherwise handled as we go.
- **Provider:** Anthropic API; model escalation; adaptive high thinking on higher tiers; caching + memory ([ADR-0005](adr/0005-model-strategy-escalation.md)).
- **Exports:** PDF, Excel, Word, CSV, JSON, text/Markdown/HTML, all from one structured schema.
- **Accuracy:** proposed defaults in [research 05 §7](research/05-ingestion-narration-export.md); calibrate after the first gold-set run.

## Still open
- Whether uploaded invoices are supplier invoices received, issued, or both (assumed both).
- Who operates ClearCut commercially (needed for the POPIA Information Officer and operator agreements).
- Verify Anthropic retention / zero-data-retention terms for the models we use.
