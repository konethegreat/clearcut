# ClearCut AgentOS Working Tasks

Work through phases in order, one small reviewed pull request per task. Follow `AGENTS.md` and `CONTRIBUTING.md`, keep all tests and CI checks green, never commit secrets or `.env` files, and append one line to `docs/agentos/REPORT.md` in each pull request.

## Phase 1: CI Health, Dependency Audit, and Operational Hygiene
- [x] Research and diagnose the failing `Audit` workflow on `main` (run `43877942661`: `cargo audit` advisories `RUSTSEC-2026-0459` in `libsqlite3-sys`, `RUSTSEC-2026-0462` in `rmcp`, `RUSTSEC-2026-0429` in `zip`, and `RUSTSEC-2026-0486` warning in `rustix`) and document the root cause and remediation plan under `docs/agentos/` (see [AUDIT_REMEDIATION.md](AUDIT_REMEDIATION.md)).
- [x] Review and resolve or document exceptions for the open Dependabot PRs (`#151` `printpdf`, `#150` `uuid`, `#149` `zip`, `#148` `rmcp`, `#138` `rust_xlsxwriter`, `#137` `@types/node`) and `libsqlite3-sys` so `Audit` passes cleanly.
- [x] Update `README.md` alpha status and quick-start summary to match `docs/LAUNCH.md` (reflecting that ingestion, narration, and multi-format export are built while gold-set scoring and e-invoicing remain open).

## Phase 2: Long-Invoice Completeness and Pipeline Hardening
- [ ] Design and implement Slice B.2 (scans without OCR: whitespace bands, capability negotiation, and uncertain seams) per `docs/superpowers/specs/2026-11-03-long-invoice-completeness-design.md` with tests across `clearcut-core`, `clearcut-docproc`, and `services/worker`.
- [ ] Implement per-call `model_calls` logging as each provider call completes in `clearcut-router` / `clearcut-pipeline` so a mid-page crash does not under-count tenant spend (`docs/LAUNCH.md` section 5).
- [ ] Add row-level locking in `clearcut-store` `erase_document` to prevent races with concurrent `enqueue_job` or identical-byte re-upload, plus an orphan-file sweep for failed deletions (`docs/LAUNCH.md` section 5).
- [ ] Design and implement Slice C (bounded document-level repair round) per `docs/superpowers/specs/2026-11-03-long-invoice-completeness-design.md` with deterministic tests in `clearcut-pipeline`.
- [ ] Design and implement Slice D (coverage evidence for human review) per `docs/superpowers/specs/2026-11-03-long-invoice-completeness-design.md` and expose it in document review responses.

## Phase 3: API Contract, Idempotency, and MCP / SDK Parity
- [ ] Research `rmcp` 3.10.0 capabilities against the MCP 2026-07-28 stateless spec and Streamable HTTP (`P1-06`) and record the findings in `docs/adr/`.
- [ ] Implement `Idempotency-Key` header support on mutating REST endpoints (`P1-05`) in `clearcut-store` and `clearcut-server` with conflict and replay tests.
- [ ] Add `contracts/examples/` golden request/response JSON fixtures and a contract test validating them against `contracts/openapi.json` (`docs/LAUNCH.md` section 2).
- [ ] Update `packages/sdk` (`@clearcut/sdk`) to pass `Idempotency-Key` on mutating calls and add SDK helpers for `export_document` (`P4-09`).

## Phase 4: Review UX, Reproducibility, and Evaluation Experiments
- [ ] Store model version, prompt version, and raw output reference per reading in `clearcut-store` for audit reproducibility (`P3-09`).
- [ ] Define and implement the citation-to-bounding-box contract (`P3-08`) and wire side-by-side page image + event citation highlighting into the web review UI (`P5-01`, `P5-02`).
- [ ] Add Postgres indexing for `GET /v1/search` and `clearcut_core::duplicate` invoice-number lookups (`docs/LAUNCH.md` section 5) with query-plan verification tests.
- [ ] Run an evaluation experiment extending `clearcut-eval` (`clearcut-gold`) to score carried-forward and brought-forward figures on synthetic multi-page fixtures (`docs/LAUNCH.md` section 5).

## Phase 5: Storage Backends, Deployment Automation, and Security Review
- [ ] Implement the S3/MinIO `ObjectStore` backend in `crates/clearcut-objects` (`P1-02`) and add a MinIO service test in `docker-compose.yml` / CI.
- [ ] Add an automated backup and restore verification script under `deploy/` and document it in `docs/DEPLOY-ORACLE.md` (`docs/LAUNCH.md` section 2).
- [ ] Conduct a security review of `docs/THREAT-MODEL.md` covering exposure hardening, automatic retention, and carried-balance/band-reading changes, and update the document.
