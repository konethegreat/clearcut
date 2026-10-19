# Worklog

_Newest first. Append an entry for each working session. Written by Kone Tshivhinda & Claude._

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

**Blockers**
- The build environment could not create a new GitHub repository itself (sessions are bound to existing repos), and the browser was not signed in to GitHub. Repo creation needs the owner to sign in or create an empty repo `konethegreat/clearcut`.

**Next**: see "Immediate next three tasks" in PHASES.md.
