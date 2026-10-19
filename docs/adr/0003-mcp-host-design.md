# ADR-0003: MCP host design

Status: Proposed · Date: 2026-10-19

## Decision
1. Ship `clearcut-mcp` as a separate binary over **stdio** now, and **Streamable HTTP** in Phase 1, sharing `clearcut-core`.
2. Target the **2026-07-28 stateless spec**: no server-side sessions; state lives in Postgres and flows through explicit IDs.
3. Mutating tools require an **idempotency key** and pass through a **policy gate**; approval uses the multi-round-trip (`InputRequiredResult`) pattern.
4. Tool annotations distinguish read-only from mutating tools.
5. Auth: OAuth 2.1 resource server for HTTP; API keys for service-to-service.

## Status of the prototype
Built on rmcp 2.2.0 speaking protocol `2025-06-18`. Moving to the stateless spec needs a newer rmcp and is Phase 1 task P1-06.

## Rejected
Merging MCP into the REST server: separate processes allow different auth, scaling and blast radius.
