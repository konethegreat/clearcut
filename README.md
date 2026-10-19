<div align="center">

# ClearCut

**Agent-native invoicing for South Africa.** One Rust core exposed as a REST API, an MCP server and a Claude plugin.

[![CI](https://github.com/konethegreat/clearcut/actions/workflows/ci.yml/badge.svg)](https://github.com/konethegreat/clearcut/actions/workflows/ci.yml)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
![Status: alpha](https://img.shields.io/badge/status-alpha-orange)

</div>

> ⚠️ **Alpha.** Calculation, lifecycle rules and a stdio MCP server work today. Persistence, PDFs, e-invoicing and payments are on the [roadmap](docs/PHASES.md). Not tax advice.

## Why ClearCut

- **Built for agents, governed for finance.** Agents draft; policy and humans approve. Idempotent, audited tools. _(approval flow: Phase 1)_
- **South African VAT as tested code.** R5 000 / R50 thresholds, 15% VAT, VAT-number checks, cited rules.
- **E-invoicing ready.** Data model designed to map to EN 16931 / Peppol PINT ahead of SARS's digital VAT model.
- **Correct money.** Integer cents, deterministic rounding, no floats.
- **Open.** Apache-2.0, self-hostable.

See the [research](docs/research/) behind these claims, including where sources disagree.

## Quick start

```bash
cargo test --workspace                 # 20 tests across core + server
cargo run -p clearcut-server           # REST on 127.0.0.1:8080
cargo run -p clearcut-mcp              # MCP over stdio

curl -s localhost:8080/v1/invoices/calculate -H 'content-type: application/json' -d '{
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
packages/sdk             TS    typed client
plugin/                  Claude plugin manifest
services/analytics       Python (planned)
docs/                    research, architecture, ADRs, phases, worklog
```

## Roadmap
Persistence & auth → PDFs & credit notes → e-invoicing → payments & reconciliation → integrations. Details: [docs/PHASES.md](docs/PHASES.md). Progress log: [docs/WORKLOG.md](docs/WORKLOG.md).

## Contributing
Read [CONTRIBUTING.md](CONTRIBUTING.md). Security issues: [SECURITY.md](SECURITY.md).

## Author
Kone Tshivhinda · Written by Kone Tshivhinda & Claude · Licensed under [Apache-2.0](LICENSE).
