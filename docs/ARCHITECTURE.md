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
   packages/sdk (TypeScript)  → typed REST client        services/analytics (Python, Phase 4)
```

## Components

| Path | Language | Role | Status |
|---|---|---|---|
| `crates/clearcut-core` | Rust | Domain: `Money`, `TaxCategory`, `Invoice`, `InvoiceStatus`, `compliance_za` | 17 tests |
| `crates/clearcut-server` | Rust (axum) | REST: `GET /health`, `POST /v1/invoices/calculate` | 3 tests |
| `crates/clearcut-mcp` | Rust (rmcp) | MCP tools over stdio: `calculate_invoice`, `za_required_tax_invoice`, `check_status_transition` | smoke-tested |
| `packages/sdk` | TypeScript | Typed client for the REST API | 2 tests |
| `plugin/` | JSON | Claude plugin manifest + MCP entry | alpha |
| `services/analytics` | Python | Forecasting / extraction sidecar | planned |

## Key design decisions
See [adr/](adr/):
- [0001 Multi-language stack](adr/0001-multi-language-stack.md)
- [0002 Money as integer minor units](adr/0002-money-representation.md)
- [0003 MCP host design](adr/0003-mcp-host-design.md)

## Planned data flow (Phase 1–2)
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
