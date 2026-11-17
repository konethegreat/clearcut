# ClearCut AgentOS Working Phases

This roadmap defines the ordered execution phases for autonomous maintenance and feature delivery in ClearCut. It complements [docs/PHASES.md](../PHASES.md) (product-wide milestones) and [docs/LAUNCH.md](../LAUNCH.md) (current operational status).

## Phase 1: CI Health, Dependency Audit, and Operational Hygiene
Goal: Keep CI and security audits green, triage open Dependabot PRs, and align top-level documentation with what is already built in `docs/LAUNCH.md`.
Done when: The `Audit` workflow passes on `main`, stale Dependabot PRs are resolved or documented, and `README.md`, `docs/INDEX.md`, and `docs/TASKS.md` accurately reflect the current codebase.

## Phase 2: Long-Invoice Completeness and Pipeline Hardening
Goal: Finish the remaining long-invoice slices (B.2 scans without OCR, C bounded repair, D coverage evidence) and close the database-race and call-logging limits documented in `docs/LAUNCH.md`.
Done when: Band-based scan reading, bounded repair, and coverage evidence are tested and merged, per-call `model_calls` logging survives mid-page crashes, and document deletion is locked against concurrent job queueing and duplicate re-uploads.

## Phase 3: API Contract, Idempotency, and MCP / SDK Parity
Goal: Stabilize the external contract for TimeLex and agent callers with idempotency keys, golden contract examples, and MCP/SDK parity.
Done when: Mutating REST endpoints honor `Idempotency-Key`, `contracts/examples/` is validated in CI against `contracts/openapi.json`, `rmcp` upgrade feasibility for MCP 2026-07-28 is documented, and the TypeScript SDK exposes all server endpoints.

## Phase 4: Review UX, Reproducibility, and Evaluation Experiments
Goal: Make human review fast and reproducible by storing prompt/model provenance, mapping citations to page coordinates, indexing search, and extending `clearcut-eval` to carried balances.
Done when: Readings record model and prompt versions, the citation-to-box contract is wired into the web review view, search queries use indexed lookups, and `clearcut-gold` scores carried-balance fixtures.

## Phase 5: Storage Backends, Deployment Automation, and Security Review
Goal: Complete S3/MinIO object storage support, automate backup verification for Oracle/Compose deployments, and bring `docs/THREAT-MODEL.md` up to date.
Done when: `clearcut-objects` has a tested S3-compatible backend, backup/restore verification scripts pass against the compose stack, and `docs/THREAT-MODEL.md` reflects current retention, quota, and pipeline controls.
