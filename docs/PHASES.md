# Phases and tasks

_Author: Kone Tshivhinda · Written by Kone Tshivhinda & Claude_

Legend: ✅ done · 🔄 in progress · ⬜ not started. Estimates are rough and assume one part-time developer alongside an internship.

## Phase 0 — Foundations ✅
- ✅ P0-01 Market, compliance and MCP research (`docs/research/`)
- ✅ P0-02 Architecture and ADRs
- ✅ P0-03 Monorepo scaffold (Rust workspace, TS package, plugin manifest)
- ✅ P0-04 `clearcut-core`: money, VAT, totals, lifecycle, ZA thresholds
- ✅ P0-05 `clearcut-server` REST skeleton with tests
- ✅ P0-06 `clearcut-mcp` stdio prototype with 3 tools
- ✅ P0-07 TypeScript SDK skeleton
- ✅ P0-08 Licence (Apache-2.0), CI, templates, security policy
- 🔄 P0-09 Public GitHub repo created and pushed _(blocked until the repo exists on GitHub; see WORKLOG)_

## Phase 1 — Persistent API + MCP host (≈4–6 weeks)
- ⬜ P1-01 Postgres schema + migrations (`sqlx`): suppliers, customers, invoices, lines, audit log, outbox
- ⬜ P1-02 Gapless per-supplier invoice numbering
- ⬜ P1-03 CRUD + lifecycle endpoints with idempotency keys
- ⬜ P1-04 AuthN/Z: API keys, then OAuth 2.1 resource server; tenant isolation (row-level)
- ⬜ P1-05 Policy engine: approval thresholds, void limits
- ⬜ P1-06 Upgrade MCP to 2026-07-28 stateless spec + Streamable HTTP transport
- ⬜ P1-07 MCP tools for create/approve/send/void with annotations; approval via multi-round-trip
- ⬜ P1-08 OpenAPI spec generated from code; TS types generated from JSON Schema
- ⬜ P1-09 Docker image + `docker compose` for local dev
- ⬜ P1-10 Prebuilt release binaries via GitHub Actions

## Phase 2 — Documents and delivery (≈3–4 weeks)
- ⬜ P2-01 PDF rendering of full/abridged tax invoices
- ⬜ P2-02 Credit and debit notes (exact labels, original-invoice reference)
- ⬜ P2-03 21-day issue-window check and 5-year retention policy
- ⬜ P2-04 Email delivery + hosted payment/invoice link
- ⬜ P2-05 Recurring invoices and templates

## Phase 3 — E-invoicing readiness (≈3–4 weeks, then tracks SARS)
- ⬜ P3-01 Map the domain model to EN 16931 business terms
- ⬜ P3-02 UBL 2.1 / Peppol PINT export; evaluate `en16931` + `en16931-formats` crates
- ⬜ P3-03 Validation against official rule sets in CI
- ⬜ P3-04 Shortlist accredited access-point providers; adapter interface
- ⬜ P3-05 Monitor SARS draft regulations; update `research/02`

## Phase 4 — Payments, reconciliation, intelligence (≈6 weeks)
- ⬜ P4-01 Payment-event ingestion (webhooks) and reference matching
- ⬜ P4-02 PSP spike: PayShap Request / pay-by-bank (Ozow, Stitch, others)
- ⬜ P4-03 Bank-feed/CSV import with fuzzy matching and short-pay handling
- ⬜ P4-04 Dunning sequences and ageing/DSO reports
- ⬜ P4-05 Python analytics sidecar: payment-delay model, cash-flow forecast
- ⬜ P4-06 Dashboard (TypeScript) for approvals, ageing and audit trail

## Phase 5 — Ecosystem and hardening
- ⬜ P5-01 Connectors: Xero, QuickBooks, Sage (verify SA demand first)
- ⬜ P5-02 Webhooks for outbound events
- ⬜ P5-03 Security review, dependency audit, threat model (incl. prompt injection)
- ⬜ P5-04 POPIA review; data-retention and deletion tooling
- ⬜ P5-05 Plugin published; docs site; v1.0 criteria

## Immediate next three tasks
1. P0-09 repo on GitHub.
2. P1-01 schema design, reviewed with Theo or a colleague at MB before coding.
3. P1-06 spike on rmcp upgrade to the stateless spec.
