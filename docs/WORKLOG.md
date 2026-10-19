# Worklog

_Newest first. Append an entry for each working session. Written by Kone Tshivhinda & Claude._

## 2026-10-19 (night) — "Boosted memory" defined
Kone: the AI must be given every resource it needs to achieve its task; everything else as we go. Recorded as the **context-pack / resource-completeness** principle in ADR-0005 §4 and task P2-16b. Kept two limits: include what the task needs (POPIA minimisation, no cross-tenant data) and keep memory writes human-confirmed. No code changed.

## 2026-10-19 (evening) — Provider, escalation, exports, accuracy decisions
**Decisions from Kone:** Anthropic API; escalate from smaller to larger models on failure; high thinking and "boosted memory" so tasks complete; popular export formats; accuracy targets unknown.
**Done**
- [ADR-0005](adr/0005-model-strategy-escalation.md): Haiku 5.5 → Sonnet 5.5 → Opus 5.5 → Fable 5.1 → human review; validation-driven escalation; adaptive thinking; prompt caching, chunking and a memory tool; budgets; ZAR cost illustration; POPIA position.
- Research 05: export formats now PDF, Excel, Word, CSV, JSON, text/Markdown/HTML on a shared schema; proposed accuracy targets table.
- PHASES.md: added P2-13…P2-19 and rewrote Phase 4 export tasks.
**Pushback recorded:** high thinking on every call does not guarantee completion; guarantees come from validation, retries, escalation and a `needs_review` terminal state. Memory must not be written from uploaded-document text (prompt-injection persistence).
**Findings:** no South African/African inference region is documented by Anthropic (global or US only); third-party source says default retention is 30 days and ZDR is by request; all unverified with Anthropic.
**No code changed.**

## 2026-10-19 (later) — Gap review: ingestion, narration, grouping, export
**Trigger:** Kone asked whether "narration" was in the research. It was not: research 01–04 only covered issuing invoices.
**Done**
- Added [research 05](research/05-ingestion-narration-export.md): OCR vs LLM vs hybrid, long-document strategy, validation, event taxonomy, guarded narration, export formats, evaluation plan, POPIA.
- Added [ADR-0004](adr/0004-ingestion-and-narration-pipeline.md); revised ADR-0001 (Python worker moves to Phase 2).
- Rewrote PHASES.md: ingestion → events/narration → export now come before issuing, e-invoicing and payments. Task IDs were renumbered.
**Decisions taken on Kone's behalf (to confirm):** support both received and issued invoices; narration is template-first with a number guard; events are the single source of truth.
**Risks logged:** accuracy figures are vendor claims; model hallucination on numbers; POPIA s72 cross-border processing and client confidentiality; gold set needs real invoices.
**No code changed** in this entry.

## 2026-10-19 — Research, scaffold, first working slice
**Done**
- Researched the invoicing market, SA VAT/e-invoicing, MCP 2026-07-28 and Rust MCP SDK (`docs/research/01–04`).
- Chose a Rust-core, TypeScript-edge stack; Python deferred (ADR-0001).
- `clearcut-core`: money (integer cents, half-away rounding), VAT categories, invoice totals, status machine, SA thresholds and field validation. 17 unit tests passing.
- `clearcut-server`: axum REST with `/health` and `/v1/invoices/calculate`. 3 tests passing.
- `clearcut-mcp`: stdio MCP server, 3 tools. Smoke-tested by hand over JSON-RPC: `initialize`, `tools/list`, structured `tools/call` all OK.
- `@clearcut/sdk`: typed client, 2 tests passing.
- Added Apache-2.0 licence, NOTICE, CI workflow, Dependabot, issue/PR templates, SECURITY, CONTRIBUTING, CODE_OF_CONDUCT, CODEOWNERS.

**Findings that change plans**
- MCP prototype runs rmcp **2.2.0** / protocol 2025-06-18, not the 2026-07-28 stateless spec (task P1-06).
- Sources disagree on SA e-invoicing dates (2028 vs. mandatory from 2030). Plan for the earlier date.
- TypeScript types are hand-mirrored from Rust; drift risk (P1-08).

**GitHub setup (same day)**
- Repo `konethegreat/clearcut` created by the owner (the build sandbox cannot create repos), pushed from the sandbox after the Claude GitHub App was granted access.
- Apache-2.0 detected by GitHub; first CI run green.
- Description, 10 topics and 6 labels set. Dependency graph, Dependabot alerts, private vulnerability reporting enabled; secret scanning and push protection were already on.
- Not done: branch protection / rulesets, Dependabot security-update toggle not verified.
- Note: repo-settings writes are blocked via the sandbox API proxy, so settings were changed through the signed-in browser.

**Next**: see "Immediate next three tasks" in PHASES.md.
