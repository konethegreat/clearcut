# Architecture

_Author: Kone Tshivhinda · Written by Kone Tshivhinda & Claude_

## Principle
**One brain, many mouths.** All invoice rules live in one pure Rust crate. REST, MCP, SDKs and plugins are thin transports over it.

```
                 AI agents / Claude plugin          Apps, scripts
                        │ MCP (stdio, HTTP)              │ REST / SDK
                 ┌──────▼──────┐                 ┌───────▼───────┐
                 │ clearcut-mcp│                 │clearcut-server│   ← transports (Rust)
                 └──────┬──────┘                 └───────┬───────┘
                        └──────────────┬─────────────────┘
                                ┌──────▼───────┐
                                │ clearcut-core│  money · tax · invoice · lifecycle · ZA compliance
                                └──────┬───────┘
              (Phase 1+)        ┌──────▼───────┐
                                │  Postgres    │  invoices, audit log, outbox
                                └──────────────┘
   packages/sdk (TypeScript)  → typed REST client        services/worker (Python, Phase 2)
```

## Components

| Path | Language | Role | Status |
|---|---|---|---|
| `crates/clearcut-core` | Rust | Domain: `Money`, `TaxCategory`, `Invoice`, `InvoiceStatus`, `compliance_za`, `event` (Event, EventPayload, Citation, grouping views), `extraction` (untrusted extractor output, deterministic validation, event derivation), `narration` (template narrator, formatting, number guard, summary/timeline/grouped modes) | 181 tests |
| `crates/clearcut-server` | Rust (axum) | REST: `GET /health`, `POST /v1/invoices/calculate` | 3 tests |
| `crates/clearcut-mcp` | Rust (rmcp) | MCP tools over stdio: `calculate_invoice`, `za_required_tax_invoice`, `check_status_transition` | smoke-tested |
| `packages/sdk` | TypeScript | Typed client for the REST API | 2 tests |
| `plugin/` | JSON | Claude plugin manifest + MCP entry | alpha |
| `services/worker` _(was `analytics`)_ | Python | OCR + model calls for extraction/narration; no business rules | planned, Phase 2 |

## Key design decisions
See [adr/](adr/):
- [0001 Multi-language stack](adr/0001-multi-language-stack.md)
- [0002 Money as integer minor units](adr/0002-money-representation.md)
- [0003 MCP host design](adr/0003-mcp-host-design.md)
- [0004 Ingestion, event and narration pipeline](adr/0004-ingestion-and-narration-pipeline.md)
- [0005 Model provider, escalation, thinking and memory](adr/0005-model-strategy-escalation.md)
- [0006 Postgres schema v1](adr/0006-postgres-schema.md) _(proposed)_

## Ingestion → narration → export pipeline (Phases 1–4)
```
upload ─► object storage ─► classify ─► extract ──────────────► validate ─► review ─► EVENTS ─► group ─► narrate ─► export
 (any      (resumable)     (pages,     embedded XML │ text     (Rust:     (human,    (source   (taxonomy  (template  JSON·CSV
 length)                    text layer) │ OCR │ LLM/vision      math, VAT,  low conf)  of truth,  + views)   + guarded  XLSX·PDF
                                        (Python worker)         SA rules)             cited)               LLM prose) MD·UBL
```
Events carry citations (file, page, span) and are the **only** input to narration and export. See [ADR-0004](adr/0004-ingestion-and-narration-pipeline.md) and [research 05](research/05-ingestion-narration-export.md).

## Planned data flow for issuing (Phase 6)
1. Agent or API creates a **draft** (idempotency key required).
2. Policy engine decides: auto-approve under threshold, else `PendingApproval`.
3. On approval, number is allocated (gapless per supplier), PDF + UBL rendered, invoice becomes **immutable**.
4. Delivery via email/portal link; status `Sent`.
5. Payment events (webhook or bank feed) are matched to invoice references → `PartiallyPaid`/`Paid`; unpaid past due → `Overdue` and dunning.
6. Every step appends to an audit log written in the same transaction as the state change (transactional outbox for events).

## Non-functional targets (to be measured, not promised)
- Stateless servers; any instance can serve any request (aligns with MCP 2026-07-28).
- Deterministic calculation: same input, same cents, on every platform.
- POPIA-aware: minimise personal data, encrypt at rest, define retention (5 years for tax records).
