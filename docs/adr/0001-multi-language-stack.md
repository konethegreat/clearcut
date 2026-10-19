# ADR-0001: Multi-language stack

Status: Accepted · Date: 2026-10-19 · Deciders: Kone Tshivhinda, Claude

## Context
The goal is an automated invoicing backend that is an API service, an MCP host and a plugin. Kone has not used Rust before and wants a fast, reliable language and a multi-language approach.

## Decision
- **Rust** for the core, REST server and MCP host. Money correctness, no GC pauses, strong types for state machines, and the official MCP SDK (`rmcp`) is Rust.
- **TypeScript** for the SDK, plugin packaging and the future dashboard, where the ecosystem and hiring pool are strongest.
- **Python, only where it wins**: OCR and model calls. _Originally Phase 4; **moved to Phase 2** by [ADR-0004](0004-ingestion-and-narration-pipeline.md) because document understanding is now the core product._ Forecasting follows later.

## Pushback we considered (honest trade-offs)
- *"Why not all Rust?"* A dashboard and SDK are faster to build and easier for others to contribute in TypeScript.
- *"Why not more languages?"* Each language adds CI, tooling, security patching and context-switching for a tiny team. **Three is the ceiling.** The Python worker is now justified (ingestion) but stays thin and rule-free; Go was considered and rejected as it overlaps Rust without a unique benefit here.
- *Learning curve.* Rust is hard to learn. Mitigations: keep `clearcut-core` small and I/O-free, lean on the compiler and tests, avoid async and lifetimes in the core, pair-review early PRs.
- *Fast to market vs. fast at runtime.* Invoicing is not CPU-bound; Rust is chosen for **correctness**, not raw speed.

## Consequences
Cross-language type drift is a risk: TypeScript types mirror Rust serde types by hand today. Phase 1 adds generated types (JSON Schema from `schemars` → TS) and a contract test.
