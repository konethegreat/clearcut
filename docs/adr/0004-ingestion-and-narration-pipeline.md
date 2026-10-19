# ADR-0004: Ingestion, event and narration pipeline

Status: Accepted · Date: 2026-10-19 · Deciders: Kone Tshivhinda, Claude
Supersedes part of ADR-0001 (Python timing).

## Context
Kone's core requirement: upload an invoice of any length; the system reads and understands it, narrates events, groups them, and exports in several formats. See [research 05](../research/05-ingestion-narration-export.md).

## Decision
1. **Pipeline:** Upload → store (object storage) → classify → extract (cascade: embedded XML → text layer → OCR → LLM/vision) → **validate deterministically** → confidence → human review queue → **events** → grouping → narration → export.
2. **Event store is the source of truth.** Append-only events with citations (file, page, span), extractor, confidence and model/prompt version. Narration and every export are *derived* from events.
3. **Long documents:** page/section chunking (map), reconciliation (reduce), async resumable jobs with progress. No giant single prompt.
4. **Narration guardrails:** template-first, LLM only for prose between facts, per-sentence event IDs, post-generation **number guard**, fallback to template-only.
5. **Language split (revises ADR-0001):** Rust owns the domain, validation, event model, job orchestration and API. A **thin Python worker** owns OCR and model calls, because that is where the ecosystem is. It holds **no business rules**. Python moves from Phase 4 to Phase 2.
6. **Provider abstraction:** extraction and narration call a trait/interface so models can be swapped or self-hosted (POPIA s72 and law-firm confidentiality).
7. **Human in the loop** for anything below a confidence threshold or failing validation.

## Consequences
- Needs Postgres, object storage and a job queue earlier (Phase 1).
- A gold evaluation set becomes a first-class asset.
- Costs scale with pages; budgets and token estimates are required.

## Rejected
- *One LLM call per document:* fails on length and audit.
- *LLM-generated narration from raw PDF:* unverifiable numbers.
- *OCR-only:* brittle on unknown suppliers.
