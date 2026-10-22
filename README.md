<div align="center">

# ClearCut

**Upload any invoice. ClearCut reads it, explains it, groups what happened, and exports it.**<br>Agent-native invoicing for South Africa: one Rust core exposed as a REST API, an MCP server and a Claude plugin.

[![CI](https://github.com/konethegreat/clearcut/actions/workflows/ci.yml/badge.svg)](https://github.com/konethegreat/clearcut/actions/workflows/ci.yml)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
![Status: alpha](https://img.shields.io/badge/status-alpha-orange)

</div>

> ⚠️ **Alpha.** Today: invoice calculation, lifecycle rules, SA VAT thresholds and a stdio MCP server. **Not built yet:** document ingestion, narration and export (designed in [research 05](docs/research/05-ingestion-narration-export.md) and [ADR-0004](docs/adr/0004-ingestion-and-narration-pipeline.md)), persistence, e-invoicing, payments. See the [roadmap](docs/PHASES.md). Not tax advice.

## Why ClearCut

- **Reads and narrates any invoice, of any length.** Cited extraction → grouped events → narration checked against the source numbers → export to JSON, CSV, XLSX, PDF. _(in design: Phases 2–4)_
- **Built for agents, governed for finance.** Agents draft; policy and humans approve. Idempotent, audited tools. _(approval flow: Phase 1)_
- **South African VAT as tested code.** R5 000 / R50 thresholds, 15% VAT, VAT-number checks, cited rules.
- **E-invoicing ready.** Data model designed to map to EN 16931 / Peppol PINT ahead of SARS's digital VAT model.
- **Correct money.** Integer cents, deterministic rounding, no floats.
- **Open.** Apache-2.0, self-hostable.

See the [research](docs/research/) behind these claims, including where sources disagree.

## Quick start

```bash
cargo test --workspace                 # needs a local Postgres for the store/server tests (see CONTRIBUTING.md)
cargo run -p clearcut-mcp              # MCP over stdio (no database needed)

# The REST API needs a Postgres and an API key. As the database owner, once:
#   clearcut-admin migrate && clearcut-admin create-tenant "My Practice"
#   clearcut-admin create-key --tenant <id> --name me --scopes keys:admin,documents:read,documents:write > my.key
# then run the server as the restricted service role (see docs/ADMIN.md):
#   DATABASE_URL=<clearcut_app connection> CLEARCUT_STORAGE_DIR=<where uploads are kept> cargo run -p clearcut-server    # REST on 127.0.0.1:8080
curl -s localhost:8080/v1/whoami -H "Authorization: Bearer $(cat my.key)"   # who am I? (GET /health needs no key)

# Upload an invoice: the body is the file itself (needs documents:write). 201 new, 200 if already uploaded.
curl -s "localhost:8080/v1/documents?filename=march.pdf" -H "Authorization: Bearer $(cat my.key)" -H "Content-Type: application/pdf" --data-binary @march.pdf

curl -s localhost:8080/v1/invoices/calculate -H "Authorization: Bearer $(cat my.key)" -H 'content-type: application/json' -d '{
  "number":"INV-1","currency":"ZAR","status":"draft",
  "lines":[{"description":"Consulting","quantity_milli":10000,
            "unit_price":{"minor":150000,"currency":"ZAR"},"tax":"standard"}]}'
# → net R15 000.00, VAT R2 250.00, gross R17 250.00, full tax invoice required
```

Amounts are **cents**; quantities are **thousandths** (`10000` = 10 units).

### Use the MCP server with Claude
Add to your MCP config:
```json
{ "mcpServers": { "clearcut": { "command": "cargo", "args": ["run", "--quiet", "--release", "-p", "clearcut-mcp"] } } }
```
Tools: `calculate_invoice`, `za_required_tax_invoice`, `check_status_transition`.

## Layout

```
crates/clearcut-core     Rust  domain: money, tax, invoice, lifecycle, ZA compliance
crates/clearcut-server   Rust  REST API (axum)
crates/clearcut-mcp      Rust  MCP server (rmcp)
crates/clearcut-store    Rust  Postgres storage, auth, the job queue
crates/clearcut-jobs     Rust  job runner (claims jobs, heartbeats, retries)
crates/clearcut-provider Rust  model-provider trait, checked requests, test fake
crates/clearcut-docproc  Rust  document-worker client and test fake
crates/clearcut-eval     Rust  gold-set label format, checker and sample set
packages/sdk             TS    typed client
plugin/                  Claude plugin manifest
services/worker          Python document worker (reads untrusted files; no model calls)
docs/                    research, architecture, ADRs, phases, worklog
```

## Roadmap
Platform foundation → **ingestion & understanding** → **events & narration** → **export** → dashboard → issuing → e-invoicing → payments. Details: [docs/PHASES.md](docs/PHASES.md). Progress log: [docs/WORKLOG.md](docs/WORKLOG.md).

## Contributing
Read [CONTRIBUTING.md](CONTRIBUTING.md). Security issues: [SECURITY.md](SECURITY.md).

## Author
Kone Tshivhinda · Written by Kone Tshivhinda & Claude · Licensed under [Apache-2.0](LICENSE).
