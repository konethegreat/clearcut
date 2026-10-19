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

**GitHub setup (same day)**
- Repo `konethegreat/clearcut` created by the owner (the build sandbox cannot create repos), pushed from the sandbox after the Claude GitHub App was granted access.
- Apache-2.0 detected by GitHub; first CI run green.
- Description, 10 topics and 6 labels set. Dependency graph, Dependabot alerts, private vulnerability reporting enabled; secret scanning and push protection were already on.
- Not done: branch protection / rulesets, Dependabot security-update toggle not verified.
- Note: repo-settings writes are blocked via the sandbox API proxy, so settings were changed through the signed-in browser.

**Next**: see "Immediate next three tasks" in PHASES.md.
