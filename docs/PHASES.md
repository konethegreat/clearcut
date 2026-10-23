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
- ✅ P1-01 Postgres + `sqlx` migrations: tenants, documents, jobs, events, audit log — done per [ADR-0006](adr/0006-postgres-schema.md): `crates/clearcut-store` (migrations, `Store` API, 23 tests against a real Postgres, CI service container). `jobs` is deferred to P1-03; API keys and row-level security come with P1-04
- 🔄 P1-02 Object storage abstraction (local disk + S3-compatible); resumable/chunked upload endpoint — done: `crates/clearcut-objects` (`ObjectStore` trait, `LocalDisk`, content-addressed tenant-scoped keys, streaming with a hard size limit, all-or-nothing writes; 19 tests). and `crates/clearcut-intake` (the upload rules: needs `documents:write`, accepted media types, size limit, streaming into the object store, document row, audit entry; 26 tests; no HTTP). `POST /v1/documents` in `clearcut-server` over it (raw streamed body, 201/200/400/401/403/413/415/500/503; server 61 tests). and `uploadDocument` in the TypeScript SDK (raw bytes or a Blob; 11 tests, SDK now 36). Remaining: resumable/chunked uploads (its own design), the S3/MinIO implementation (with P1-07)
- ✅ P1-03 Durable job queue (Postgres-backed) with retries, progress and cancellation — design accepted in [ADR-0008](adr/0008-job-queue.md) (2026-10-21); built in four small PRs: ~~core job types and retry policy~~ ✅ (`clearcut-core::jobs`, 33 tests; `DocumentStatus` moved to the core), ~~migration 0005 + store methods~~ ✅ (`enqueue_job`, `claim_job`, `heartbeat_job`, `complete_job`, `fail_job`, `request_cancel`, `get_job`, `jobs_for_document`; 60 tests; store now 136), ~~runner library~~ ✅ (`crates/clearcut-jobs`: `Runner`, `Handler`, `JobContext`; 42 tests), ~~REST + SDK~~ ✅ (`GET /v1/documents/{id}/jobs` and `POST /v1/jobs/{id}/cancel`; server 61 tests, SDK 36). **P1-03 is complete.** Still to come: enqueue-on-upload and a runner process, both with the first real handler (P2-04)
- ✅ P1-04 AuthN/Z: API keys, tenant isolation (row-level) — design accepted in [ADR-0007](adr/0007-auth-and-row-level-security.md); building in small PRs; done: step 1 `clearcut-core::auth` (`Scope`, `Principal`; 12 tests) and step 2 migration 0003 (restricted `clearcut_app` and login-less `clearcut_eraser` roles, row-level security that fails closed, tenant-scoped `Store`, `erase_document` replacing the `clearcut.erasure` setting; the store tests now run as the restricted role, 37 tests). step 3 migration 0004 plus the key methods (`ccut_<id>_<secret>` keys; only a SHA-256 of the secret is stored and the app role cannot even read it; 365-day maximum life; every failure identical; `keys:admin`-gated create/list/revoke, audited; store tests now 70). step 4 `clearcut-admin` (`migrate`, `create-tenant`, `create-key`, `revoke-key`; owner connection only; stdout carries only the id or token; 17 tests + a real-binary smoke test; runbook in [ADMIN.md](ADMIN.md)). step 5 server and SDK authentication (identical 401 for every failure, 403 only to authenticated callers, the key never logged, and the service refuses to start on an unsafe database role; server 21 tests, SDK 13, store 76). **P1-04 is complete.** The protected upload endpoint is the remainder of P1-02.- ⬜ P1-05 Idempotency keys on all mutating endpoints
- ⬜ P1-06 Upgrade MCP to the 2026-07-28 stateless spec + Streamable HTTP; adopt Tasks extension for long jobs
- ⬜ P1-07 Docker image + `docker compose` dev stack (API, Postgres, MinIO, worker) — **security follow-up:** when the job runner becomes its own process, give it its own login role holding `EXECUTE` on the queue functions (`claim_job` is cross-tenant) and revoke them from `clearcut_app` (ADR-0008, risks; flagged again by the automated review of #26)
- 🔄 P1-09 **Try-it deployment** (proposed 23 Oct 2026, from Kone; design: [ADR-0015](adr/0015-try-it-deployment.md) accepted 23 Oct 2026; **first release is the local path only**, the hosted trial waits for Kone to decide who runs it): `CLEARCUT_MODE` `local` / `trial`; **mode C session key** (server memory only, opaque cookie in the browser, a Remove button that wipes the key and everything uploaded); the local `docker compose` path (with P1-07); ephemeral demo tenants and a janitor; the free hosting choice (Oracle Always Free VM with Caddy and a Cloudflare Tunnel; Cloud Run with one instance as fallback); a small try page and `docs/DEPLOY.md`. Built after the end-to-end slice
- ⬜ P1-08 OpenAPI from code; TS types generated from JSON Schema; contract tests

## Phase 2 — Ingestion & understanding (≈6–8 weeks) ⭐ core
- 🔄 P2-01 **Gold set:** collect and anonymise 200+ real SA invoices; label fields (with the document owners' consent and a POPIA review) — **tooling and kit ✅** ([ADR-0010](adr/0010-gold-set-and-evaluation.md): label format, `check`, `new-case`, the repository `guard` in CI, synthetic sample, and the guides, consent template and checklists in [`docs/gold-set/`](gold-set/README.md)); **collection ⬜** (needs Kone: a private location, consent, a first source, an attorney's review of the consent form)
- ⬜ P2-02 Document intake: type sniffing, page count, text-layer detection, de-duplication by hash
- ⬜ P2-03 Embedded-structured-data reader (UBL/CII/Factur-X XML inside PDFs)
- ✅ P2-04 Python worker skeleton (gRPC/HTTP), provider trait, no business rules — design accepted in [ADR-0009](adr/0009-worker-and-provider-boundary.md) (2026-10-22): the worker only reads documents (no API key, no egress) and the model provider lives in Rust, so P2-13 is a Rust adapter. Four small PRs: ~~`clearcut-provider` (trait, types, fake)~~ ✅ (25 tests), ~~the Python worker skeleton~~ ✅ (`services/worker`: FastAPI, bearer auth, limits, `/health`, `/v1/inspect`; 146 tests, a CI job on Python 3.12 and 3.14), `clearcut-docproc` (client and contract test), then the container with P1-07
- 🔄 P2-05 Text-layer extraction + OCR fallback for scans — design: [ADR-0016](adr/0016-page-text-images-and-text-quality.md) accepted 23 Oct 2026, built in small PRs; **step 1 (licence check, boxed-in child) and step 2 (`/v1/text`: text, word boxes, font facts, page-count agreement) built**, steps 3 to 7 open (pypdfium2 + pypdf + Pillow + Tesseract `eng`/`afr`, **PyMuPDF replaced because it is AGPL**; stateless `/v1/text`, `/v1/ocr`, `/v1/render`, `/v1/mask`; word boxes for the image masker; a text-quality verdict decided in `clearcut-core`)
- ⬜ P2-06 LLM/vision structuring with JSON-Schema output, per-page chunking, page-level provenance
- ⬜ P2-07 Map-reduce for long documents: page-spanning tables, carried balances, statement totals
- 🔄 P2-08 **Deterministic validation** in `clearcut-core` (line/VAT/total checks, SA field rules, duplicate detection) — done: `extraction::{Extraction, validate}` (line arithmetic, stated vs recomputed totals, due-before-issue, `compliance_za` rules; exact to the cent; 27 tests). Remaining: duplicate detection (needs other documents as input)
- ⬜ P2-09 Confidence scoring + human review queue (approve / correct / reject; corrections feed the gold set)
- 🔄 P2-10 Evaluation harness in CI: field-level accuracy, "zero silent errors" gate — format, `check` and **scoring with the gate ✅** (`clearcut-gold score`, ADR-0010 step 2); **running it in CI against a real pipeline run** waits for the pipeline slice
- ⬜ P2-11 MCP tools: `upload_document`, `get_extraction`, `review_extraction`
- ⬜ P2-12 Cost guardrails: token estimate, per-tenant budget, batch mode
- ✅ P2-13 Anthropic provider adapter (behind the provider trait): structured outputs, one cache breakpoint, usage incl. cache fields, error mapping — **bring-your-own key, no operator key**; **built, tested against a stub server, never yet run against the real API** (one ignored live test for Kone to run with their own key) (design: [ADR-0012](adr/0012-anthropic-adapter.md) accepted, [ADR-0011](adr/0011-bring-your-own-anthropic-key.md), facts: [research 06](research/06-anthropic-api.md)). **P2-13b** ✅ a model-facing reading shape (built: `clearcut_core::reading`, schema accepted by the adapter profile with 0 optional / 0 union properties), because core's `Extraction` schema is **measured over** the API's structured-output limits (30 union properties against 16) — design: [ADR-0013](adr/0013-model-facing-reading-shape.md) proposed, awaiting review. **P2-13c** (proposed in ADR-0013): check what the model read against the page's own text layer
- ⬜ P2-14 **Model router + escalation ladder** (Haiku 5.5 → Sonnet 5.5 → Opus 5.5 → Fable 5.1 → human review) driven by validation failures, disagreement, low confidence, truncation; escalation policy as config
- ⬜ P2-15 Adaptive thinking + effort per tier; log tokens, tier, effort, cache hits, escalation reason per call
- ⬜ P2-16 Prompt caching for the static prefix (instructions, schema, SA rules, examples); Batch API mode for bulk uploads
- ✅ P2-16b **Context-pack builder** (built: `clearcut-pack`, [ADR-0014](adr/0014-context-pack-builder.md) accepted; cached rules and synthetic examples, per-page data blocks with a random id, redaction hook, injection flags, per-call manifest; the first real model call has not happened) ("resource completeness"): page image + text, neighbours, schema, SA rules, examples, supplier history, validator/calculator tools, previous-tier failure report; logged per call
- ⬜ P2-17 Memory store (memory tool): written only from human-confirmed corrections; path validation, size caps, expiry, per-tenant
- ⬜ P2-18 Terminal-state guarantee: every job ends `done` or `needs_review`; chaos tests (timeouts, truncation, provider outage)
- 🔄 P2-19 POPIA: **redaction of bank/ID numbers before sending ✅ built** (`clearcut_core::redact`, `best_effort` / `strict` policy in `clearcut-pack`; image masking waits for the worker, P2-05), privacy notice and consent text, verify retention/ZDR terms with Anthropic, legal review
- ⬜ P2-20 **Glyph-obfuscated PDFs: detect, decode, verify** (proposed 22 Oct 2026, from Kone; **high risk: if it is wrong the system reads nothing**). Some PDFs scramble the glyph-to-character mapping (custom font encodings, no ToUnicode), so the text layer *exists* but is gibberish, and today it would pass as "has a text layer". Plan: (1) the worker scores text quality per page and flags gibberish; (2) a flagged page is read from its **rendered image** (vision/OCR), which does not depend on the mapping at all; (3) optionally rebuild the per-font glyph map by recognising each glyph's shape, for cheap exact text; (4) **never trust one path**: the decoded text and the vision read must agree on every amount, date and id, and the to-the-cent arithmetic must hold, else `needs_review`; (5) test fixtures: synthetic scrambled PDFs we generate (a Type 3 font with no Unicode map gives ground truth) **and real anonymised samples from Kone**; gold-set tag `obfuscated-glyphs`. ADR first, and a spike on real samples before any design is fixed (the **detection** half is proposed in [ADR-0016](adr/0016-page-text-images-and-text-quality.md))
- ⬜ P2-21 **Fee-line reconciliation** (proposed 22 Oct 2026, from Kone): every fee line reconciled to the cent, for statement-type documents (fees listed line by line, summing to a stated total). Invoices already reconcile line totals, VAT and totals to the cent in `clearcut-core`; this extends it to statements and their carried-forward balances. Needs Kone to say which documents (bank, payment-processor, supplier statements?)
- 🔄 P2-23 **API key handling, mode A** (ADR-0011): the secret type, loading from `ANTHROPIC_API_KEY` or a file, refusing a key on the command line, the narrow HTTPS client and the free key check through the Models API, redaction tests — **✅ built** (`clearcut-anthropic`, [BYOK guide](BYOK.md)); calling `reject_key_in_arguments` and `load` at server/MCP start-up waits for the adapter. Mode B (encrypted per-tenant keys) later, with its own review
- ⬜ P2-24 **Rate-limit-aware client** (ADR-0011): adaptive per-key concurrency, `anthropic-ratelimit-*` headers, `retry-after`, spend-cap and 401/402 handling mapped to the job queue's failure kinds
- ⬜ P2-22 **Line categorisation with Claude** (proposed 22 Oct 2026, from Kone): after validation, each line gets a category from a fixed taxonomy through the provider trait; low confidence or an unknown category goes to review; never part of the critical fields; the memory (P2-17) learns only from human-confirmed corrections. ADR first: whose taxonomy (fee types, GL accounts)?

## Phase 3 — Events, grouping and narration (≈4–5 weeks) ⭐ core
- ✅ P3-01 Event model in `clearcut-core`: type, group, date, payload (integer cents), citation, extractor, confidence (`crates/clearcut-core/src/event/`; immutable events, corrections via `supersedes`; 24 tests)
- ✅ P3-02 Event taxonomy and deterministic grouping; views by date, party, document, severity (`event/view`: current/timeline/by group/document/severity + derived `Severity`, 13 tests; **party view deferred to P3-03** since events have no party field yet)
- ✅ P3-03 Event derivation from validated extractions (incl. stated-vs-recomputed mismatch events) (`extraction::derive_events`: deterministic IDs like `doc_1:line-2`, printed facts cite their source, calculated VAT and findings are `System` events, new `LineTotalMismatch` kind; 22 tests. **By-party view dropped**: a party needs an identity that survives across documents, which belongs with storage)
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
3. ~~**P3-01:** event model~~ ✅ done 19 Oct 2026. ~~**P3-02**~~ ✅ and ~~**P3-04**~~ ✅ done. ~~**P3-06**~~ ✅ done 20 Oct 2026. ~~**P3-07**~~ ✅ done 20 Oct 2026. ~~**P3-03**~~ ✅ done 20 Oct 2026 (with the first half of P2-08). The pure-Rust part of Phase 3 is finished; the rest needs infrastructure: P3-05 LLM smoothing needs the provider trait (Phase 2), P3-08 needs the page/box contract with a UI, P3-09 and P3-10 need storage and the MCP upgrade. Next: **P1-01/P1-02** (Postgres schema and storage; Docker Desktop must be running; show Kone the schema first) and **P2-01** (gold set).

## Decisions made (19 Oct 2026)
- **Boosted memory:** the model gets every resource it needs for its task (context pack); details otherwise handled as we go.
- **Provider:** Anthropic API; model escalation; adaptive high thinking on higher tiers; caching + memory ([ADR-0005](adr/0005-model-strategy-escalation.md)).
- **Exports:** PDF, Excel, Word, CSV, JSON, text/Markdown/HTML, all from one structured schema.
- **Accuracy:** proposed defaults in [research 05 §7](research/05-ingestion-narration-export.md); calibrate after the first gold-set run.

## Still open
- Whether uploaded invoices are supplier invoices received, issued, or both (assumed both).
- Who operates ClearCut commercially (needed for the POPIA Information Officer and operator agreements).
- Verify Anthropic retention / zero-data-retention terms for the models we use.
