# 04 · How ClearCut stands out

_Author: Kone Tshivhinda · Written by Kone Tshivhinda & Claude_

Positioning: **the open-source, agent-native invoicing engine built for South African VAT and ready for e-invoicing.**

| # | Differentiator | Why it is real | Evidence / risk |
|---|---|---|---|
| 1 | **MCP-native from day one** | Same core exposed as REST, MCP tools and a plugin | Xero's MCP wraps an existing ledger; we own the lifecycle (research 03) |
| 2 | **Governed agents** | Draft→approve→send workflow, spend/void limits, idempotency, full audit | Rarely described in competitor material _(inferred)_ |
| 3 | **Compliance as code** | SA thresholds, VAT-number checks and note wording tested and cited | Rules must be kept current; see research 02 |
| 4 | **E-invoicing ready** | Data model maps to EN 16931/Peppol PINT before the mandate lands | Mandate dates disputed; design for earliest |
| 5 | **Local payment reconciliation** | PayShap Request / pay-by-bank links tied to invoice references | Needs a PSP partner (Ozow, Stitch, others); not yet validated |
| 6 | **Correct money by construction** | Integer minor units, deterministic rounding, no floats, Rust type safety | Already tested in `clearcut-core` |
| 7 | **Open and embeddable** | Apache-2.0, self-hostable, API + SDK + plugin; transparent pricing | Competitors mostly "contact sales" |
| 8 | **Accountant-friendly sync** | Push to the ledger people already use rather than replace it | Connectors are Phase 5 work |

## What we deliberately will not do
- Replace a full accounting ledger (we integrate).
- Give personalised financial advice (AR insights are operational, not investment advice).
- Build a Peppol access point (we integrate an accredited one).

## How we will know it is working
- Time from "agent drafts" to "customer receives a compliant invoice".
- % of invoices auto-reconciled without human touch.
- Zero compliance defects against the rule test-suite.
- DSO reduction for pilot users (MB internal pilot first).
