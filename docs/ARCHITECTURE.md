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
| `crates/clearcut-core` | Rust | Domain: `auth` (Scope, Principal), `jobs` (job states and legal transitions, retry/backoff and lease policy, progress, error trimming), `document` (DocumentStatus), `Money`, `TaxCategory`, `Invoice`, `InvoiceStatus`, `compliance_za`, `event` (Event, EventPayload, Citation, grouping views), `extraction` (untrusted extractor output, deterministic validation, event derivation with re-extraction runs), `narration` (template narrator, formatting, number guard, summary/timeline/grouped modes) | 236 tests |
| `crates/clearcut-server` | Rust (axum) | REST: `GET /health` (public), `GET /v1/whoami`, `POST /v1/invoices/calculate` (both need an API key), `GET /v1/documents/{id}/jobs` (`documents:read`) and `POST /v1/jobs/{id}/cancel` (`documents:write`; 200 if cancelled or already finished, 202 if a running job has only been asked to stop), `POST /v1/documents` (needs `documents:write`; the body is the raw file, streamed through `clearcut-intake`; started with `CLEARCUT_STORAGE_DIR`). `Authed` extractor: every authentication failure gives the identical 401, an outage is a 500, scoped routes answer 403 only to authenticated callers, the key is never logged and the header is marked sensitive. Connects as the restricted service role and refuses to start on a superuser, `BYPASSRLS` or table-owner role | 61 tests |
| `crates/clearcut-store` | Rust (sqlx) | Postgres persistence ([ADR-0006](adr/0006-postgres-schema.md)): tenants, documents (hash de-duplication), append-only events and audit log; the durable job queue ([ADR-0008](adr/0008-job-queue.md)): leases with fencing tokens, retries, cancellation, progress, all state changes through five vetted database functions that move the document with the job; every call runs in a tenant-scoped transaction under row-level security ([ADR-0007](adr/0007-auth-and-row-level-security.md), restricted `clearcut_app` role); atomic, idempotent event append; events re-validated on read; `erase_document` for POPIA erasure; API keys (`keys` module: hashed, shown once, 365-day max life) with authentication through a login-less security-definer function | 136 tests (need `DATABASE_URL`) |
| `crates/clearcut-admin` | Rust (clap) | The operator command ([runbook](ADMIN.md)): `migrate`, `create-tenant`, `create-key`, `revoke-key`. Needs the owner database connection and refuses the service role; stdout carries only the tenant id or the new key, stderr the human notes | 17 tests |
| `crates/clearcut-provider` | Rust | The model-provider contract ([ADR-0009](adr/0009-worker-and-provider-boundary.md)): the `ModelProvider` trait, a request that is checked when built against the API's real limits (images, cache breakpoints, size, output cap), the response and `Usage`, `ProviderError` mapped onto the job queue's failure kinds, and a scripted `FakeProvider`. Nothing prints invoice content. The Anthropic adapter is P2-13 | 25 tests |
| `crates/clearcut-jobs` | Rust (tokio) | The job runner ([ADR-0008](adr/0008-job-queue.md)): claims jobs, keeps each lease alive with a background heartbeat, runs a `Handler`, reports the outcome through the core's retry policy. Stops a worker that loses its lease, treats a panic as a transient failure, resumes from checkpoints, shuts down gracefully | 42 tests |
| `crates/clearcut-intake` | Rust (tokio) | What happens on an upload, independent of HTTP so REST and MCP share it: needs `documents:write` (checked before reading), accepts PDF/PNG/JPEG/TIFF/XML only, refuses an over-limit declared length without reading, streams into `clearcut-objects` with a hard limit, records the document through the store (a repeat upload returns the existing document), audits it. File names are cleaned metadata and never paths. A file stored before a failed database write is deliberately left in place | 26 tests |
| `crates/clearcut-objects` | Rust (tokio) | Object storage for uploaded files: `ObjectStore` trait + `LocalDisk`; keys are `tenants/<uuid>/documents/<sha256>` (built from the content hash, never from a filename); streaming upload with a hard size limit; temp file renamed into place only when complete | 19 tests |
| `crates/clearcut-mcp` | Rust (rmcp) | MCP tools over stdio: `calculate_invoice`, `za_required_tax_invoice`, `check_status_transition` | smoke-tested |
| `packages/sdk` | TypeScript | Typed client for the REST API (API key, `whoami`, `calculate`, `uploadDocument`, `jobsForDocument`, `cancelJob`) | 36 tests |
| `plugin/` | JSON | Claude plugin manifest + MCP entry | alpha |
| `services/worker` _(was `analytics`)_ | Python (FastAPI) | Reads untrusted documents only ([ADR-0009](adr/0009-worker-and-provider-boundary.md)): `GET /health`, `POST /v1/inspect` (page count and text-layer detection for PDFs; OCR and page images arrive with P2-05). Bearer-token auth before the body is read, size, page and time caps, each inspection in a killable child process (JSON answer, never unpickled), JSON errors, no content in logs and no forgeable log lines. No API key, no egress, no database, no business rules | 146 tests |

## Key design decisions
See [adr/](adr/):
- [0001 Multi-language stack](adr/0001-multi-language-stack.md)
- [0002 Money as integer minor units](adr/0002-money-representation.md)
- [0003 MCP host design](adr/0003-mcp-host-design.md)
- [0004 Ingestion, event and narration pipeline](adr/0004-ingestion-and-narration-pipeline.md)
- [0005 Model provider, escalation, thinking and memory](adr/0005-model-strategy-escalation.md)
- [0006 Postgres schema v1](adr/0006-postgres-schema.md)
- [0007 Auth, roles and row-level security](adr/0007-auth-and-row-level-security.md)

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
