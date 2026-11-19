<div align="center">

# ClearCut

**Upload any invoice. ClearCut reads it, explains it, groups what happened, and exports it.**<br>Agent-native invoicing for South Africa: one Rust core exposed as a REST API, an MCP server and a Claude plugin.

[![CI](https://github.com/konethegreat/clearcut/actions/workflows/ci.yml/badge.svg)](https://github.com/konethegreat/clearcut/actions/workflows/ci.yml)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
![Status: alpha](https://img.shields.io/badge/status-alpha-orange)

</div>

> **Alpha.** Built today: invoice calculation, lifecycle rules, SA VAT compliance, multi-page document ingestion (PDF, DOCX, XLSX, CSV, images, and scanned PDFs), event grouping and narration, human review UI (`#/review`, `#/demo`, `#/trust`), multi-format export (JSON, CSV, XLSX, PDF), tenant-scoped API keys with Postgres RLS, and a stdio MCP server. **Still open:** gold-set accuracy scoring on real invoices, e-invoicing (UBL/Peppol), issuing/payments, and deployment/contract hardening. See [docs/LAUNCH.md](docs/LAUNCH.md) and the [roadmap](docs/PHASES.md). Not tax advice.

## Why ClearCut

- **Reads and narrates any invoice, of any length.** Cited extraction -> grouped events -> narration checked against the source numbers -> export to JSON, CSV, XLSX, PDF. _(built; gold-set accuracy scoring on real invoices remains open)_
- **Built for agents, governed for finance.** Agents draft; policy and humans approve via `#/review` or MCP. Tenant-scoped, audited tools. _(REST `Idempotency-Key` support open)_
- **South African VAT as tested code.** R5 000 / R50 thresholds, 15% VAT, VAT-number checks, cited rules.
- **E-invoicing ready.** Data model designed to map to EN 16931 / Peppol PINT ahead of SARS's digital VAT model.
- **Correct money.** Integer cents, deterministic rounding, no floats.
- **Open.** Apache-2.0, self-hostable.

See the [research](docs/research/) behind these claims, including where sources disagree.

## Quick start

```bash
cargo test --workspace                 # needs a local Postgres for the store/server tests (see CONTRIBUTING.md)
cargo run -p clearcut-mcp              # MCP over stdio (no database needed for calculation tools)

# The REST API needs Postgres and an API key. As the database owner, once:
#   cargo run -p clearcut-admin -- migrate
#   cargo run -p clearcut-admin -- create-tenant "My Practice"
#   cargo run -p clearcut-admin -- create-key --tenant <id> --name me --scopes keys:admin,documents:read,documents:write > my.key
# then run the server as the restricted service role (see docs/ADMIN.md and docs/LAUNCH.md):
#   DATABASE_URL=<clearcut_app connection> CLEARCUT_STORAGE_DIR=<where uploads are kept> CLEARCUT_KEY_MODE=tenant-only cargo run -p clearcut-server    # REST on 127.0.0.1:8080
curl -s localhost:8080/v1/whoami -H "Authorization: Bearer $(cat my.key)"   # who am I? (GET /health needs no key)

# Upload an invoice: the body is the file itself (needs documents:write). 201 new, 200 if already uploaded.
curl -s "localhost:8080/v1/documents?filename=march.pdf" -H "Authorization: Bearer $(cat my.key)" -H "Content-Type: application/pdf" --data-binary @march.pdf

# Export a processed document to JSON, CSV, XLSX, or PDF:
curl -s "localhost:8080/v1/documents/<id>/export?format=json" -H "Authorization: Bearer $(cat my.key)"

curl -s localhost:8080/v1/invoices/calculate -H "Authorization: Bearer $(cat my.key)" -H 'content-type: application/json' -d '{
  "number":"INV-1","currency":"ZAR","status":"draft",
  "lines":[{"description":"Consulting","quantity_milli":10000,
            "unit_price":{"minor":150000,"currency":"ZAR"},"tax":"standard"}]}'
# -> net R15 000.00, VAT R2 250.00, gross R17 250.00, full tax invoice required
```

Amounts are **cents**; quantities are **thousandths** (`10000` = 10 units).

### Use the MCP server with Claude
Add to your MCP config:
```json
{ "mcpServers": { "clearcut": { "command": "cargo", "args": ["run", "--quiet", "--release", "-p", "clearcut-mcp"] } } }
```
Tools: `calculate_invoice`, `za_required_tax_invoice`, `check_status_transition`, `upload_document`, `list_documents`, `get_document`, `wait_for_document`, `get_events`, `narrate_document`, `export_document`, `get_review_queue`, `confirm_page`, `erase_document`.

## Layout

```
crates/clearcut-core     Rust  domain: money, tax, invoice, lifecycle, ZA compliance
crates/clearcut-server   Rust  REST API (axum)
crates/clearcut-mcp      Rust  MCP server (rmcp)
crates/clearcut-store    Rust  Postgres storage, auth, the job queue
crates/clearcut-jobs     Rust  job runner (claims jobs, heartbeats, retries)
crates/clearcut-provider Rust  model-provider trait, checked requests, test fake
crates/clearcut-docproc  Rust  document-worker client and test fake
crates/clearcut-pipeline Rust  the process_document job handler: analyses a stored document page by page
crates/clearcut-pack     Rust  context-pack builder: cached prompt, redaction, injection handling
crates/clearcut-anthropic Rust  the user's own Anthropic key (protected), the HTTPS client, and the Anthropic adapter (schema profile, request, response, errors)
crates/clearcut-eval     Rust  gold-set label format, checker, scorer with release gate, repository guard, sample set
packages/sdk             TS    typed client
plugin/                  Claude plugin manifest
services/worker          Python document worker (reads untrusted files; no model calls)
docs/                    research, architecture, ADRs, phases, worklog
```

## Roadmap
Platform foundation, **ingestion & understanding**, **events & narration**, and **export** (built; gold-set scoring open) -> dashboard -> issuing -> **e-invoicing** (open) -> payments. Details: [docs/PHASES.md](docs/PHASES.md), [docs/LAUNCH.md](docs/LAUNCH.md). Progress log: [docs/WORKLOG.md](docs/WORKLOG.md).

## Contributing
Read [CONTRIBUTING.md](CONTRIBUTING.md). Security issues: [SECURITY.md](SECURITY.md).

## Author
Kone Tshivhinda · Written by Kone Tshivhinda & Claude · Licensed under [Apache-2.0](LICENSE).
