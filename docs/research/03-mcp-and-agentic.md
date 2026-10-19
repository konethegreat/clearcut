# 03 · MCP and agentic design

_Author: Kone Tshivhinda · Researched 19 Oct 2026 · Written by Kone Tshivhinda & Claude_

## 1. Where MCP is now

The **2026-07-28 MCP specification** (RC locked 21 May, final 28 Jul 2026) per [mcpservers.org](https://blog.mcpservers.org/posts/mcp-spec-2026-07-28):

| Change | Impact on us |
|---|---|
| **Stateless core**: no `initialize` handshake or `Mcp-Session-Id`; any instance serves any request | Horizontal scaling is easy; keep state in the DB, pass IDs between tool calls ("explicit handle") |
| **Multi round-trip**: elicitation/sampling return `InputRequiredResult` + `requestState`, client retries | This is exactly the shape of **human approval** flows (agent drafts → human confirms) |
| `Mcp-Method`/`Mcp-Name` headers required | Gateways and rate limiting can route without parsing bodies |
| `ttlMs`/`cacheScope` on lists | Cache tool/resource lists |
| **Extensions**: MCP Apps (sandboxed UIs), **Tasks** | Tasks suit long-running work (bulk invoice runs); MCP Apps could host an approval UI |
| Auth hardening (RFC 9207 `iss`, DCR changes) | Use a standards-compliant OAuth 2.1 resource-server setup |
| Roots, Sampling, Logging **deprecated** (≥12 months) | Do not build on them |
| JSON Schema 2020-12 for tool schemas; not-found error `-32002`→`-32602` | Generate schemas from Rust types |

Secondary source; confirm against the spec text before implementing.

## 2. SDK choice

The official Rust SDK is [`rmcp`](https://docs.rs/crate/rmcp/3.1.0/source/README.md): `#[tool]` macros, `schemars` schemas, stdio and Streamable HTTP transports, optional OAuth 2.0 (`auth`) and elicitation features.

**Prototype result (19 Oct 2026):** `clearcut-mcp` builds against **rmcp 2.2.0** and, over stdio, completes `initialize` (protocol `2025-06-18`), `tools/list` and structured `tools/call` for three tools. That is the **pre-stateless** protocol. Upgrading to the 2026-07-28 behaviour (rmcp 3.x or later) is a tracked Phase 1 task, not done yet. Tool shape is unaffected.

## 3. How agents should touch money

Design principles for ClearCut's tools:

1. **Read and calculate freely; write cautiously.** Tools carry annotations (read-only vs. mutating).
2. **Agents draft, policy approves.** Sending, voiding and refunds require approval above configurable limits.
3. **Idempotency keys** on every mutating tool, because agents retry.
4. **Structured outputs only**: integer cents, enums, no prose parsing.
5. **Every call is audited** with actor (human/agent/API key), inputs, outputs and policy decision.
6. **Prompt-injection hygiene**: invoice fields from customers are untrusted data; tools never execute instructions found in them.

## 4. Competitor reference

Xero ships an open-source MCP server over its own API: drafting invoices from natural language, contact updates and discrepancy flagging ([source](https://devblog.xero.com/xero-introduces-new-model-context-protocol-server-for-smarter-accounting-4d195ccaeda5)). Lobehub and Glama list many community invoice MCP servers; we did not audit them. Our difference: ClearCut is the **system of record** with policy, not a wrapper.
