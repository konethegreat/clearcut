# Worklog

_Newest first. Append an entry for each working session. Written by Kone Tshivhinda & Claude._

## 2026-10-22 (part 8) — ADR-0010 accepted; gold-set tooling step 1 (`clearcut-eval`)
**Done** (ADR PR #33 merged as Accepted; own branch; tests first; 120 tests)
- New crate `clearcut-eval` with the `clearcut-gold` command: `check` (validates a gold set, shows coverage against the ADR's targets, exit code 1 on problems) and `sample` (writes the synthetic set).
- The label format: strict JSON, amounts as decimal text read exactly into cents and thousandths (no floats anywhere), truth that converts to the core `Extraction` and runs through the real validator so `expect.findings` is checked against the system's own rules.
- Reports never contain label or document content; a leak test found and fixed one real leak (malformed tags in the coverage line).
- 15-case synthetic sample set with real PDFs, checked in and kept in sync by a test; read back with pypdf to confirm the text layer and page counts match the labels.
- 100 rules in the loader, label rules, checker, scanner, command line and generators. Eleven did not die first time: nine were real test gaps (a non-digit after the dot, a 256-character file name, a blank-but-spaced reviewer, the checksum's carry, the two group-length rules and the gap rule of the long-number scanner, a single unexpected file, and cross-case problem ordering) and each got a test; one led to a small design change (file names compare case-insensitively only on Windows); two had patterns that no longer matched after formatting and were checked by hand. All caught now.
- ADR-0010's label example was wrong (totals did not add up; recipient too thin for a full tax invoice); corrected in the implementation notes.
**Next**: ADR-0010 step 2 (scoring: outcomes per critical field, the silent-error rule, report, gate), then step 3 (collection kit and the CI guard against documents in the repo). **Needs Kone**: where the private gold folder lives, a first source of real invoices, consent wording, the POPIA Information Officer.

## 2026-10-22 (part 7) — P2-04 step 3: the Rust document-worker client (`clearcut-docproc`)
**Done** (worker PR #31 merged first, after fixing its CI: a typing difference between Windows and Linux in a test helper, fixed with a small protocol type; own branch; tests first; 39 unit tests, 1 proxy test, 7 live tests)
- `DocumentProcessor`, `HttpWorker`, `FakeWorker`, `ProcessError` (mapped onto the queue's failure kinds), `Inspection`. The client is plain-`http` only, follows no redirects, ignores proxy environment variables, caps what it reads, and never prints the token, the address or anything the server said.
- **Live contract test against the real worker**, passing locally (7 of 7, no token in the worker's log), and a new CI job `contract` that starts the worker with a fresh masked token and runs it. PDF fixtures (mixed, scan, three pages, encrypted, damaged) are synthetic; `.gitattributes` marks documents binary because `core.autocrlf` would otherwise corrupt their byte offsets.
- Mutation-checked 64 rules in the client and its types. Eight survived the first pass: two did not compile (the compiler enforcing exhaustive error handling), one guard was dead code (deleted: a `http` URL always has a host), one is equivalent (the header library already rejects the byte the token check allows), and four were real gaps (a password with an empty user name, a zero page limit, the 598/599 status edge, and a media-type check). All real ones caught.
- **P2-04 is complete.**
**Next**: options are P2-02/P2-05 (intake sniffing and the worker's text and OCR, which need the first real handler to be useful), enqueue-on-upload with that handler, the container image with P1-07, P2-01 (the gold set, needs Kone), or P2-13 (the Anthropic adapter in Rust).

## 2026-10-22 (part 6) — P2-04 step 2: the Python worker skeleton
**Done** (provider PR #30 merged first; own branch; tests first; 90 tests)
- `services/worker`: a FastAPI app with `GET /health` and `POST /v1/inspect` (PDF page count and text-layer detection with `pypdf`), bearer auth checked before the body, size cap enforced as bytes arrive, page cap, deadline, encrypted and damaged PDFs refused with generic messages, JSON errors everywhere, the API description not served, logging that never carries content or the token. Refuses to start without a strong token.
- Tooling: Python 3.12+, exact pins plus a full `constraints.txt`, `ruff`, strict `mypy`, warnings-as-errors in pytest, and a CI job on Python 3.12 and 3.14.
- Smoke-tested the real server with `curl`: no token and short token exit 2; health 200; `/docs` 404; no key and wrong key 401, including a 3 MB body with no key (401, not 413); a 3-page PDF against a 2-page cap 422; a scan reports 0 text pages; encrypted 422; damaged 422; a 3 MB body and a 3 MB chunked body over a 2 MB cap 413; empty 400; GET on inspect 405; no token or content in the log. **The smoke test found a real gap**: the worker's request lines never appeared (uvicorn only configures its own loggers), so a test-first fix added a logging setup (and tests of the start-up options and port range).
- Mutation-checked about 70 rules (settings, each inspection rule, every auth and limit rule, each error's status and text, health, logging, start-up). The first pass caught 61 of 77. Of the 16 that survived, 3 were redundant auth parsing conditions (deleted: the constant-time comparison already rejects those inputs), 2 were equivalent or no-op, and 11 were real gaps (the deadline boundary, a page whose text extraction fails, a one-page text layer, the exact 404 and 405 texts, the validation and unexpected-error handlers, the port range, and a log test that was satisfied by the HTTP client's own log lines instead of the worker's). All caught now.
**Security review follow-up (same PR).** An automated review flagged two things in the worker. (1) *Log injection:* the logged path came from the caller, so control characters could forge or garble log lines. Fixed test-first: known routes are logged by template, unknown paths and the method are escaped and the path is cut to 200 characters. A surprise on the way: Python's URL parser silently drops a decoded newline from `request.url.path`, so the attack was blocked by accident, but other control characters passed; the worker now logs the path from the ASGI scope and escapes it. (2) *Parser DoS:* a deadline checked between pages cannot stop a parser stuck inside one call, and a crash takes the worker down. Fixed test-first by running each inspection in a **child process that the parent kills** at the hard limit (new `isolation.py`), so a stuck or crashing parser is contained. The child's answer is tiny JSON with a size limit, validated field by field and **never unpickled** (a test sends a malicious pickle and checks the parent runs nothing), so even a compromised parser cannot use the channel. Tests: now 146. The suite takes about 25 seconds because each HTTP test really spawns a child (on Linux CI it is faster).
**Next**: step 3, `clearcut-docproc` in Rust (the `DocumentProcessor` trait, `HttpWorker` client, `FakeWorker`, error mapping onto failure kinds) with a live contract test against this worker in CI.

## 2026-10-22 (part 5) — P2-04 step 1: the `clearcut-provider` crate
**Done** (ADR-0009 merged as Accepted first, with Kone's five decisions recorded; own branch; tests first; 25 tests)
- New crate: the `ModelProvider` trait (`complete(request) -> Result<ModelResponse, ProviderError>`), a `RequestBuilder` whose `build()` checks the request against the API's limits, `Block`/`TextBlock` (text, images, cache breakpoints), `CallContext` (tenant, job, document, page, purpose; for logging only), `Usage` (saturating arithmetic, `Add`/`Sum`), `ProviderError` with `failure_kind()`, `usage()` and `retry_after()`, and `FakeProvider`.
- Privacy by construction: nothing in the crate prints prompt text, image bytes or extracted data. Costs and tokens only; prices in Rand arrive with budgets (P2-12).
- Mutation-checked 69 rules (every limit and its boundary, the defaults, the model-id rules, each validation, every `Debug` leak, each `Usage` field and overflow, the failure-kind table, the usage and retry accessors, the messages, and the fake's order, recording and exhaustion). Ten survived the first pass and were real gaps: unpinned constants, the model-id length, system text not counted toward the size cap, a "bytes not characters" test that was too weak, exact `Debug` strings, and a total that overflowed in only some positions. All now caught.
**Next**: step 2, the Python worker skeleton (`services/worker`: FastAPI app, bearer auth, limits, `/health`, `/v1/inspect`, pytest, a CI job).

## 2026-10-22 (part 4) — P2-04: worker and provider design (no code)
**Done** (PR #28 merged first, completing P1-03; then docs only: ADR-0009 as Proposed, design before code)
- Read ADR-0004/0005 for the worker. Found a real design question: they put OCR **and** model calls in the Python worker, so the component that parses untrusted PDFs and images would also hold `ANTHROPIC_API_KEY` and have internet access. Proposed splitting: the worker only reads documents (no key, no egress, no database), and the `ModelProvider` trait with its Anthropic adapter lives in Rust beside the router, budgets and logging. The cost is maintaining the request types for the small slice of the API we use (no official Rust SDK). If Kone prefers the original wording, only the provider's implementation changes.
- Also proposed: HTTP and JSON (not gRPC), a walking skeleton (health plus inspect), two Rust traits (`DocumentProcessor`, `ModelProvider`) with fakes, an error table mapping onto the queue's failure kinds, a CI job that runs the real worker against a Rust contract test, and four small PRs.
- Kone approved all five leanings (decisions recorded at the end of the ADR); ADR-0009 is Accepted.
**Next**: `clearcut-provider`, then the worker skeleton, then `clearcut-docproc`.

## 2026-10-22 (part 3) — P1-03 step 4: job endpoints and SDK methods
**Done** (runner PR #27 merged first; own branch and PR; tests first; server 61 tests, SDK 36)
- `GET /v1/documents/{id}/jobs` and `POST /v1/jobs/{id}/cancel`, behind a `JobQueue` trait with a fake for the HTTP tests and the real store for the end-to-end ones. Behaviour is in ADR-0008's new implementation notes: 404 for unknown or foreign things, 200/202 on cancel, a JSON 400 for a bad id, authentication before the path is read, and a job shown without its payload, checkpoint, worker name, lease or tenant.
- SDK: `jobsForDocument` and `cancelJob` (returns `{ job, stopping }`), ids checked as UUIDs before any request.
- Mutation-checked 40 server rules (scopes, tenant and ids passed through, 200 vs 202, the audit actor, every field of the job view, ordering, each error's text and status, detail not leaked, the unknown-document check, the cancel really happening) and 17 SDK rules. All server mutations were caught first time. One SDK mutant survived (a UUID last group longer than 12 characters was accepted) and got its test cases.
- **Smoke-tested the real binaries** against a scratch database: no key 401; writer-only key listing 403 naming `documents:read`; reader cancelling 403 naming `documents:write`; another tenant listing or cancelling 404; a queued job cancelled with 200 and again with 200; a running job asked to stop with 202; two audit rows by `api_key`; no key in the server log. The smoke test also showed the malformed-id response was axum's plain text, so it now returns the JSON error.
**P1-03 is complete.** Nothing enqueues from the API yet (decision 4 in ADR-0008): that arrives with the first real handler.
**Next**: P2-04 (the provider trait and worker skeleton), or the remaining P1-02 items (resumable uploads), or P2-01 (the gold set, which needs Kone).

## 2026-10-22 (part 2) — P1-03 step 3: the `clearcut-jobs` runner library
**Done** (job queue store PR #26 merged first; own branch and PR; tests first; 42 tests)
- New crate `clearcut-jobs`: `Runner::run_once` / `Runner::run(shutdown)`, the `Handler` trait (a native async trait method, so no boxing), `JobContext` (job, `checkpoint()`, `is_cancelled()`, `lease_lost()`, `report()`), `Finished`/`Failure`, `RunnerConfig` (validated: lease, retry, heartbeat rhythm, poll interval, concurrency, per-tenant cap, kinds), `Ended`/`RunReport`.
- Behaviour: background heartbeat with the latest progress; a worker that loses its lease (or whose document is erased) is **aborted and reports nothing**; cancellation is cooperative; a panic is a transient failure; a retry resumes from the last checkpoint; the loop respects concurrency and the tenant cap, survives a database outage, and drains in-flight jobs on shutdown.
- Tests against a real Postgres with scripted handlers: success, review, each failure kind, five failures to review, outages never using attempts, panics, resume from a checkpoint, progress not reset by heartbeats, a 4.5 s job keeping a 3 s lease, cancellation, lease theft, erasure mid-run, the loop's concurrency and cap, graceful shutdown, idle shutdown, and a closed pool. Repeated eight times to check for flakiness. One test was racy at first (a live worker's heartbeat correctly renewed the lease before my simulated thief arrived); it now uses a slow heartbeat so the worker has genuinely stalled.
- Mutation-checked about 65 rules in the runner (configuration validation and defaults, every claim argument in both the single run and the loop, each failure kind, each result mapping, the heartbeat, cancellation, lost leases, progress and checkpoints, the loop's slots, polling and shutdown). About a dozen survived the first passes and each was a real test gap, so each got a test (for example: a test that passed by coincidence because a panic inside the handler ended the same way; the lease and tenant cap never checked through `run_once`; a hot idle loop, now observable through a small public `polls()` counter). **Four are left, on purpose:** three are equivalent mutants (the store's fencing token already guarantees the outcome, e.g. a stale worker's report is refused whether or not the runner tries it) and one is a scheduling race that cannot be tested deterministically (the handler's future being dropped just before `run_once` returns, rather than just after).
**Next**: step 4, REST and SDK: `GET /v1/documents/{id}/jobs` and `POST /v1/jobs/{id}/cancel`. Then the first real handler (P2-04) can turn on enqueue-on-upload.

## 2026-10-22 — P1-03 step 2: the job queue in Postgres (migration 0005 and the store)
**Done** (core job rules PR merged first; own branch and PR; tests first; store tests 76 -> 136)
- Migration `0005_jobs.sql`: the `jobs` table, a login-less `clearcut_queue` role, row-level security, column-level grants for the application role (read all but `lease_token`; insert only the caller-decided columns; no update, no delete), and seven functions: five public (`claim_job`, `heartbeat_job`, `complete_job`, `fail_job`, `cancel_job`) and two helpers. `erase_document` now deletes a document's jobs.
- Store: `NewJob`, `Job`, `ClaimedJob`/`LeaseToken`, `FailureResult`, and the methods. Error text and notes are trimmed with the core's `short_error`/`short_note` so a long message cannot break the write that records a failure.
- 60 tests: a job's defaults; idempotent enqueue; foreign documents refused; each privilege the application role must not have, column by column; claiming (priority, due time, cross-tenant, a per-tenant cap, twelve runners racing for six jobs, a locked row skipped rather than waited for); lost leases (a replaced worker is locked out of heartbeat, complete and fail); final-attempt, deadline and cancel sweeps; heartbeats (progress, checkpoint, cancel flag, a slow worker carrying on); completion; transient retries, provider outages that never use attempts, exhaustion; cancellation; erasure; and a property test that **every operation on every starting state only makes a move the core's transition table allows**.
- Mutation-checked 88 rules in the SQL and the Rust. 71 were caught at first and 16 survived; each survivor was a real gap and got a test (a deterministic row-lock test for `skip locked`, DB-level argument guards, a cross-tenant token, a missing token on a non-running job, raw row-level security without the Rust `where`, per-column insert grants, an outage on the final attempt, audit actors, and the returned results). All now caught.
- Refinements to ADR-0008 are recorded in its new implementation notes (an expired-but-not-replaced lease still works; stronger table checks; cancelling a never-started job leaves its document alone).
**Next**: step 3, the `clearcut-jobs` runner library (poll, claim, heartbeat in the background, dispatch to a `Handler` trait, apply the outcome), then step 4, REST and SDK.

## 2026-10-21 (part 12) — P1-03 step 1: job rules in `clearcut-core`
**Done** (ADR-0008 merged as Accepted first, with Kone's five decisions recorded; own branch; tests first; core 203 -> 236)
- New `clearcut_core::jobs`, pure: `JobKind`, `JobState` (five states, no `failed`) with the legal-transition table, the table of what each job state does to its document (claimed -> processing, succeeded -> done, needs_review -> needs_review, cancelled -> received), `Outcome`, `FailureKind`/`Decision`, `RetryPolicy` (5 attempts, 30 s doubling to 15 min, +/-20% jitter; randomness is passed in as a sample so the rules stay deterministic), `LeasePolicy` (5 min, heartbeat at a third), the 48 h `DEFAULT_DEADLINE`, `Progress`, and `short_error`/`short_note`, which fit worker text to the database column limits so a long error cannot break the very write that records a failure.
- `DocumentStatus` moved from the store to `clearcut_core::document` (the job rules need it); the store re-exports it.
- Mutation-checked 47 rules (names, each legal and illegal transition, document effects, every default number, backoff doubling/cap/overflow, jitter bounds and clamp, outage never counting or ending a job, exhaustion boundary, policy validation, lease minimum, progress, percentage rounding, text limits); all caught.
- **Correction to ADR-0008:** it said five retries "span roughly 45 minutes". Five attempts are four waits (30+60+120+240 s), about 7.5 minutes. The ADR now says so; the approved numbers are unchanged, and Kone can raise `max_attempts` if a longer window is wanted. Also added: a provider outage is retried every 5 minutes (+/-20%) without counting.
**Next**: step 2, migration `0005_jobs.sql` and the store methods (show Kone the final SQL in the PR; also teaches `erase_document` to delete jobs).

## 2026-10-21 (part 11) — P1-03: job queue design (no code)
**Done** (docs only: ADR-0008 as Proposed, per "show me the schema and design first, before any migration")
- Proposed one `jobs` table (a job is one stage of work on one document; page-level progress and a checkpoint instead of a row per page), five states with no `failed` (`queued`, `running`, `succeeded`, `needs_review`, `cancelled`), leases with a fencing token, a deadline on every job, and a provider outage that does not consume attempts.
- All state changes through five `SECURITY DEFINER` functions (`claim_job`, `heartbeat_job`, `complete_job`, `fail_job`, `cancel_job`), the same pattern as `erase_document`; the document's status moves in the same transaction. Retry/backoff numbers and the state table live in `clearcut-core`, with a store test that the two agree.
- Flagged: the claim function is the one deliberate cross-tenant bridge; a separate runner login role is advised once the runner is its own deployment (P1-07). Wiring uploads to enqueue is deferred until a handler exists (P2-04).
- Kone approved all five leanings (decisions recorded at the end of the ADR); ADR-0008 is Accepted.
**Next**: build in four small PRs: core job types and retry policy; migration 0005 and store methods (also teaches `erase_document` to delete jobs); the `clearcut-jobs` runner library; REST and SDK.

## 2026-10-21 (part 10) — P1-02: `uploadDocument` in the TypeScript SDK
**Done** (#21 `clearcut-intake` and #22 `POST /v1/documents` merged first; tests first; SDK tests 13 -> 24)
- `client.uploadDocument(file, { contentType?, filename? })`: sends the bytes as they are (a `Blob`, `ArrayBuffer` or `Uint8Array`, held in memory; streaming is a later addition), the content type as the `Content-Type` header (a Blob supplies its own, an explicit option overrides it), the name as a properly encoded `?filename=`. Returns the server's `UploadedDocument` (`duplicate` tells a repeat). Errors are the usual `ClearCutError` (403 carries `requiredScope`).
- Refuses before any request when there is no content type, or one that could inject a header (line breaks, control characters). The shared `request` helper now only defaults to JSON when the caller did not set a content type.
- Mutation-checked 13 rules; all caught after one survivor: with no file name the URL could end in a bare `?`, which `new URL().search` hides, so the test now compares the raw URL.
- **Smoke-tested the built SDK against the real server and database**: bytes and Blob uploads (the repeat flagged `duplicate`), a read-only key gets 403 with `requiredScope`, `text/html` 415, 5000 bytes against a 2000-byte limit 413, empty 400; `Café & Co/march #1.pdf` is stored as `march #1.pdf`; one document row.
**Next**: resumable/chunked uploads (own design), then P1-03 (job queue; needs Kone's schema review) and P2-04.

## 2026-10-21 (part 9) — P1-02: `POST /v1/documents`
**Done** (stacked on the intake PR; tests first; server tests 21 -> 39)
- The endpoint is the raw file as the body (`Content-Type` says what it is, optional `?filename=`), so it streams to storage instead of being held in memory. Order: authenticate, read the query, check `documents:write`, and only then touch the body. Answers: 201 new / 200 repeat (flagged `duplicate`), 401, 403 naming the scope, 415 listing the accepted types, 413 stating the limit, 400 empty or interrupted, 500 with no detail (the detail goes to the log), 503 if started without an intake.
- A test proves a 3 MiB body streams through, past axum's default 2 MiB limit (which only applies to buffering extractors). A body that breaks off arrives at the intake as an aborted connection, so a vanished client is a 400 rather than a server fault.
- Intake is behind a small dyn-compatible trait (`Ingest`), like `Authenticator`, so the HTTP rules are tested with a fake and the real thing is tested end to end (store, disk, two tenants, refused uploads leave no file and no row).
- Startup: `CLEARCUT_STORAGE_DIR` is **required** (the server will not guess where to put invoices); `CLEARCUT_MAX_UPLOAD_BYTES` optional, default 64 MiB.
- Mutation-checked 21 HTTP rules (scope check removed or wrong, 201/200 swapped, content type defaulted, length and name ignored, broken body not an aborted connection, each refusal's status, limit not stated, internal detail leaked, unconfigured = 500, accepted list, response fields, GET accepted); all caught after one survivor (the response's media type was only ever tested with PDF) got its own test.
- **Smoke-tested the real binaries** against a scratch database: the server exits 2 without `CLEARCUT_STORAGE_DIR` and with a bad `CLEARCUT_MAX_UPLOAD_BYTES`; no key 401, read-only key 403, new upload 201 (name `../../a invoice.pdf` arrives as `a invoice.pdf`), same bytes 200 flagged duplicate, `text/html` 415, over the limit 413 (also for a chunked upload with no declared length), empty 400; exactly one file on disk and no temp leftovers, two `document.uploaded` audit rows, the key and the file name absent from the server log.
- Known quirk, documented on the field: for a repeat upload the response echoes the name sent *this* time; the existing document keeps its first name.
- The file name travels in the query string. Trade-off noted in the module docs: a proxy could log it; this server does not log URLs and the name is only a label.
**Next**: an SDK `uploadDocument`; then P1-03 (job queue; needs Kone's schema review) and P2-04.

## 2026-10-21 (part 8) — P1-02: the upload rules (`clearcut-intake`)
**Done** (own branch; tests first; 26 tests, the upload ones against a real Postgres and a temp directory)
- PR #20's `rust` check failed on CI with a flaky test: `tracing` caches, per log statement, whether anyone was listening, so a per-test subscriber is lost when a parallel test hits the same statement first. Fixed by installing one process-wide subscriber that writes to a per-thread buffer; 8 local runs green.
- New crate `clearcut-intake`, no HTTP so the REST endpoint and the later MCP tool share one set of rules. Order: scope `documents:write`, media type (PDF, PNG, JPEG, TIFF, XML), declared length against the limit, all **before the body is read** (tests use a reader that fails if touched); then stream into the object store with a hard limit that applies whatever the sender claimed; then the document row; then an audit entry `document.uploaded` (file name, size, duplicate flag, never content), also for repeat uploads.
- File names are cleaned metadata: last path component only, control and invisible direction characters removed, whitespace collapsed, 255 characters, fallback `upload`. They never become paths (storage keys come from the content hash).
- Decision: if the database write fails after the file is stored, the file is **kept**. It is named by its content and owned by one tenant, a retry finds it, and deleting it could race with an identical upload that just found it. An orphan sweep can come later.
- Mutation-checked 29 rules (scope, media type, declared-length boundary, each filename rule, duplicate flag, audit for new and repeat uploads, each error mapping, tenant, stored type and page count); all caught. My first harness mislabelled results (a failing test also prints `error:`), so it was redone with explicit "could not compile" detection; the redo found that two rules had no test (the stored media type being the canonical one, and the page count staying unknown), so a test was added.
- Interrupted uploads (connection reset or aborted) are told apart from our own storage failures.
**Next**: `POST /v1/documents` in `clearcut-server` over this crate (raw streamed body, `filename` query parameter, 201 new / 200 duplicate, 413/415/400).

## 2026-10-21 (part 7) — P1-04 step 5: authentication in the server and the SDK
**Done** (admin command PR #19 merged first; own branch and PR; tests first; server tests 3 -> 21, store 72 -> 76, SDK 2 -> 13)
- `clearcut-server`: the `Authed` extractor, `GET /v1/whoami`, `/v1/invoices/calculate` now behind a key, `/health` public. Identical 401 for every authentication failure; outage is a 500; 403 only for authenticated callers and it names the scope; auth runs before the body is read; `Authorization` marked sensitive; failures logged without the key. Tests use a fake authenticator (no database) plus three against the real store; one captures `tracing` output to prove the key never reaches the log.
- Store: `verify_service_role` (refuses a superuser, a `BYPASSRLS` role, or a table owner), and the server **refuses to start** on such a role. Found while designing this: a service accidentally given the owner's connection string would silently turn row-level security off, with no visible symptom.
- TypeScript SDK: options-object constructor with `apiKey`, `whoami()`, key sent only on authenticated calls, held in an ES private field, validated against header injection; `ClearCutError.requiredScope` for 403s.
- Mutation-checked 12 server rules (scheme case, any scheme accepted, whitespace tokens, repeated header, outage-as-401, 403-as-401, skipped scope check, missing `WWW-Authenticate`, key logged, calculate unprotected, header not marked sensitive, body-before-auth) and 7 SDK rules. One server mutation survived (whitespace inside a token) because the fake authenticator rejects such tokens anyway; the fix was to assert the authenticator is never *asked*. "Body read before the key is checked" cannot even be written: axum requires the body extractor last, so the type system enforces that rule.
- **Smoke-tested the real binaries end to end** against a scratch database: the admin command bootstraps a tenant and key; the server refuses to start on the owner's credentials (exit 1) and with no `DATABASE_URL` (exit 2); run as `clearcut_app` it answers `/health` 200, no key 401, wrong secret 401, real key 200 with the right tenant and scopes, calculate without a key 401 even with a garbage body, calculate with the key 200; after the operator revokes the key, 401 at once; the secret appears 0 times in the server log.
- README quick-start and docs/ADMIN.md updated for the new flow.

**P1-04 is now complete** (scopes, roles and RLS, erase function, API keys, admin command, server and SDK authentication). Deferred by decision: expiry cap on created keys (no cap), throttling (left to the deployment).
**Next**: the protected upload endpoint (P1-02's remainder: streams through `clearcut-objects`, records the document through the store, needs `documents:write`), then P1-03 (job queue; needs Kone's schema review) and P2-04.

## 2026-10-21 (part 6) — P1-04 step 4: the `clearcut-admin` command
**Done** (API keys PR #18 merged first; own branch and PR; tests first; 17 admin tests + 2 store tests, store now 72)
- Merged #18 after recording Kone's decision on the open ADR question: **no expiry cap** (new keys get up to 365 days regardless of the creator's expiry).
- New crate `clearcut-admin` (library + thin `main`, clap): `migrate`, `create-tenant <name>`, `create-key --tenant --name --scopes [--days]`, `revoke-key --tenant --key`. Owner connection only (`DATABASE_URL`); the restricted service role is refused with a clear message. stdout = only the tenant id or the key (one line), stderr = human notes that never contain the secret; failures print nothing on stdout; exit codes 0/1/2. Tenant creation is audited as the operator. Unknown tenant, blank name, bad scope (error lists the valid ones), `--days` outside 1 to 365 are all refused cleanly.
- Store: `revoke_api_key_as_operator` (no admin key needed; audited as operator; idempotent; wrong tenant looks absent).
- Mutation-checked 11 rules (token to stderr, secret in the notes, blank names, service-role detection, unknown-tenant detection, days ignored, valid scopes unlisted, days range, tenant audit, missing key, stdout before success); all caught. Two of my mutation attempts were malformed shell quoting and proved nothing, so the secret-leak one was redone with a script file.
- **Smoke-tested the real binary** against a scratch database: no-`DATABASE_URL` exit 2, migrate, create-tenant, create-key (one stdout line, secret absent from stderr), bad scope exit 2, unknown tenant exit 1 with empty stdout, revoke, and the audit trail (tenant.created, api_key.created, api_key.revoked, all by operator); scratch database dropped.
- New runbook `docs/ADMIN.md`.

**Next in P1-04**: the `Bearer` extractor in `clearcut-server` (authenticate, require a scope per route, redact the header from logs) and the protected upload endpoint (uses `clearcut-objects` and the store), which is what P1-02 has been waiting on.

## 2026-10-21 (part 5) — P1-04 step 3: API keys
**Done** (own branch and PR; tests first; store tests 37 -> 66: 9 pure key-format tests, 20 database tests; ADR-0007 gains implementation notes)
- `keys` module (pure apart from OS randomness): `ccut_<32 hex id>_<64 hex secret>`; strict parsing (no trimming, case folding or `Bearer ` prefix); the stored value is SHA-256 of the 32 secret bytes; `ApiKeyToken`'s `Debug` shows the key id and hides the secret.
- Migration `0004_api_keys.sql`: `api_keys` table (`expires_at NOT NULL`), column-level privileges so the application role **cannot read `secret_hash`**, change scopes/name/expiry, or delete; RLS; login-less `clearcut_authenticator` role owning `authenticate_api_key` (security definer, fixed `search_path`, executable only by the app role; failures return nothing and reveal nothing; `last_used_at` refreshed at most once a minute).
- `Store`: `authenticate` (returns the core `Principal`), `create_api_key` / `revoke_api_key` / `list_api_keys` (need `keys:admin`, own tenant only, audited without the secret), `create_api_key_as_operator` (for the admin command). Revoking is immediate and idempotent; another tenant's key looks absent. Default and maximum life 365 days.
- Mutation-checked 14 rules across the SQL and Rust (secret-hash visibility, scope tampering, revoked/expired/wrong secret accepted, last-used throttle, tenant visibility, search_path, execute grant, three missing scope checks, lifetime cap, scope de-duplication); all caught. Two of those rules had no test until the mutation pass listed them, so I added tests for the lifetime cap and for scope de-duplication/name trimming.

**Security review finding, fixed before merge**: the automated review of this PR noticed that `create_api_key` required `keys:admin` but never checked that the caller held the scopes it was handing out, so a key with only `keys:admin` could mint a key with any scope, against "no scope implies another". Fixed tests first: a creator may now only grant scopes it holds (one missing scope refuses the whole request); the operator path is the only way to issue scopes nobody holds. Two of my own fixtures had been quietly relying on the hole and were corrected. Store tests 66 -> 70. The review's optional second suggestion (cap a new key's expiry at its creator's) is **not** implemented: it would stop an admin key creating its own successor, a trade-off that Kone decided: **no cap** (recorded in ADR-0007).

**Deviation recorded in the ADR**: the secret comparison happens inside the database function (on hashes of hashes) instead of in Rust, so the application role never has to read the stored hash.
**Tooling lessons**: Git Bash rewrites an argument that starts with `//` into a path, which mis-spliced a test block once (caught immediately by the compiler); heavy quoting in one-line shell scripts failed twice, so scripts are now written to files.
**Next in P1-04**: the `clearcut-admin` bootstrap command (create tenant, create first key, run with the owner connection), then the `Bearer` extractor in `clearcut-server` and the protected upload endpoint.

## 2026-10-21 (part 4) — P1-04 step 2: roles, row-level security, tenant-scoped store, `erase_document`
**Done** (own branch and PR; tests first; store tests 23 -> 37; ADR-0007 gains implementation notes)
- Migration `0003_roles_rls.sql`: restricted `clearcut_app` (cannot own tables, bypass RLS, update/delete events or audit rows, truncate, delete documents or create tenants) and login-less `clearcut_eraser`; RLS on `tenants`, `documents`, `events`, `audit_log` that **fails closed** when no tenant is set; `erase_document` (SECURITY DEFINER, owned by the eraser, fixed `search_path`, executable only by the app role). The `clearcut.erasure` setting is retired.
- `Store`: every operation now runs in a transaction that sets `clearcut.tenant` locally (it cannot leak onto a pooled connection); new `erase_document`; `create_tenant` documented as owner-only.
- **The existing store tests now run through the restricted role**, since a superuser bypasses RLS and would prove nothing. Raw SQL used to inspect the database goes through the admin connection. 14 new tests: role attributes, nothing visible without a tenant, a deliberately sloppy query only sees its own tenant, the setting does not outlive its transaction, cross-tenant writes refused, no change/delete/truncate of the log, no tenant creation, erasure (rows, audit entry without content, other tenant untouched, foreign or missing document refused, the function checks the tenant itself, security-definer properties, who may execute).
- Mutation-checked seven rules (event visibility, delete grant, tenant check, search_path, execute grant, eraser-only deletes, bypassrls). 

**Problems found and fixed along the way**
- Creating the shared roles was **racy** when many databases migrate at once: an `if not exists` check and then a single `DO` block both worked from a stale snapshot. Fixed by taking the advisory lock as its own statement, catching duplicates, and verified on a brand-new cluster plus repeated runs.
- A mutation that weakened the role attributes initially survived because the shared role already existed in the dev cluster. That exposed a real weakness (a pre-existing role with `bypassrls` would be accepted), so the migration now enforces the attributes, and a test run proves it repairs a corrupted role.
- `sqlx::migrate!` does not trigger rebuilds on changed `.sql` files; added a `build.rs`.
- Honest note: the one run made while the role was deliberately corrupted had 2 failures (tests ran while the attribute was still flipping); every normal run is 37/37.

**Deviation from the ADR sketch**: RLS is enabled but not `FORCE`d (reason in ADR-0007 implementation notes).
**Next in P1-04**: API keys (create with 365-day default expiry, authenticate in constant time, revoke), then the `clearcut-admin` command, then the server Bearer extractor.

## 2026-10-21 (part 3) — P1-04 step 1: `clearcut-core::auth`
**Done** (ADR-0007 merged as Accepted first; own branch and PR; tests first; 12 new tests, core now 203)
- `Scope` (`documents:read`, `documents:write`, `events:read`, `review:write`, `keys:admin`): exact text forms, serde as those strings, exact parsing (no trimming, case folding, wildcards or comma lists), and `parse_all` that fails on the first unknown name rather than trimming a stored list.
- `Principal` (tenant, key id, scopes): `has`, `require` (names the missing scope), deduplicated and ordered scopes. **No scope implies another**, not even `keys:admin`; a principal with no scopes can do nothing.
- New errors `UnknownScope` and `MissingScope`. Mutation-checked: scope implication, trimming and silent dropping of unknown scopes each fail a test.
- ADR-0007 decisions recorded: scope list as written, no cross-tenant super key, 365-day default key expiry, throttling left to the deployment.

**Next in P1-04**: migration with the three roles, row-level security, a tenant-scoped `Store` and the `erase_document` function (retiring `clearcut.erasure`); then API keys; then the `clearcut-admin` command; then the server Bearer extractor.

## 2026-10-21 (part 2) — P1-04 design proposal (docs only)
**Done**
- Drafted [ADR-0007](adr/0007-auth-and-row-level-security.md): API keys (`ccut_<id>_<secret>`, shown once, SHA-256 of the secret stored, constant-time check, identical error for every failure), five initial scopes enforced by a pure `Principal` in `clearcut-core`, **row-level security** on every tenant table that fails closed when no tenant is set, three database roles (owner, restricted `clearcut_app`, `clearcut_eraser`), and erasure turned into a `security definer` function so the application can call it but cannot delete events itself. This retires the `clearcut.erasure` setting and closes the follow-up from ADR-0006.
- Status: **Accepted** by Kone on 2026-10-21 with the proposed leanings (scope list as written, no cross-tenant super key, 365-day default key expiry, throttling left to the deployment). Being built in small PRs, tests first.

**Open questions in the ADR**: the scope list, no cross-tenant operator key, default key expiry, failed-login throttling left to the deployment.
**Next (after review)**: tests first for `Scope`/`Principal`, then the migration (roles, RLS policies, `api_keys`, `erase_document`, `authenticate_api_key`), the tenant-scoped `Store` refactor, a `clearcut-admin` bootstrap command, then the `Bearer` extractor in `clearcut-server` and the protected upload endpoint.

## 2026-10-21 — P1-02 part 1: `clearcut-objects` (object storage)
**Done** (own branch and PR; tests first; 19 tests; workspace now 191 core + 19 objects + 23 store + 3 server)
- New crate `clearcut-objects`: `ObjectStore` trait (`put`, `open`, `exists`, `delete`) and `LocalDisk`. A key is `tenants/<uuid>/documents/<sha256 hex>`: **content-addressed and tenant-scoped**, so the same upload twice is stored once, and two tenants never share an object. `ObjectKey` can only be built from a tenant and a hash, or parsed back in exactly that canonical shape (lowercase hyphenated UUID, lowercase hex), so a filename, `..`, an absolute path, a backslash or an odd UUID form can never reach the filesystem.
- `put` streams in 64 KiB chunks, hashes while writing to a temp file, enforces a hard size limit on what is actually read (not on what the sender claims), rejects empty files, and renames into place only when complete. Failed, oversized or empty uploads leave nothing behind; simultaneous identical uploads all succeed and leave one intact object.
- Mutation-checked six rules (limit boundary, empty check, temp cleanup, canonical UUID form, lowercase hex, tenant in key). The UUID-form mutation initially survived, which led to extra test cases for uppercase, braced, `urn:` and simple-form UUIDs.

**Decisions / push-back**
- **Not built yet, on purpose:** the upload endpoint (an unauthenticated write endpoint is the wrong thing to ship; it waits for P1-04), resumable/chunked uploads (a design of its own: sessions and partial state), and the S3/MinIO implementation (arrives with the Docker stack, P1-07; the trait is shaped for it).
- Content addressing means a delete removes the object for every document of that tenant with the same bytes; the store layer must not delete an object another document row still references (decide with the upload flow).
- Virus/malware scanning of uploads is not designed yet; uploads are treated as untrusted bytes and nothing here executes or parses them.

**Environment note**: the `clearcut-pg` container had exited (Docker Desktop restarted); `docker start clearcut-pg` brings it back, and the store tests need it.
**Next**: P1-03 (job queue; decides the `jobs` table, needs Kone's schema review), P2-04 (worker skeleton and provider trait), or P1-04 (auth and roles, which also unblocks the upload endpoint and the `clearcut.erasure` role restriction).

## 2026-10-20 (part 7) — P1-01: `clearcut-store` crate, migrations, real-Postgres tests
**Done** (own branch and PR; tests first; 23 new store tests)
- With Kone's approval: pulled the official `postgres:17` image and started a dev container `clearcut-pg` bound to `127.0.0.1:54329` with no password (`trust` auth, localhost only; Kone's own Postgres on 5432 and the other containers were left alone). CONTRIBUTING.md has the command; `.env.example` has `DATABASE_URL`. No password literal appears anywhere in the repo.
- `crates/clearcut-store` (sqlx 0.9): the two migrations from ADR-0006 and a `Store` API: `create_tenant`, `add_document` (hash de-duplication per tenant), `document_status` / `set_document_status`, `append_events`, `load_events`, `record_audit`. Every call takes the tenant; another tenant's document looks absent (`DocumentNotFound`).
- `append_events` is atomic and idempotent: re-appending the same ID with the same content is a no-op (safe for retried jobs); the same ID with different content is `EventIdConflict`; an event citing a different document is refused. `load_events` re-validates every stored event through the core.
- Tests (one throwaway database each via `sqlx::test`) cover the round trip of core-derived events, re-extraction runs, generated columns, append-only triggers on `events` and `audit_log` (and the erasure escape hatch inside a transaction), composite tenant foreign keys, the body/row `check` constraints, and refusal of `failed` as a status. CI gets a `postgres:17` service container.
- Mutation-checked in `lib.rs`: conflict check, tenant filter, citation check and commit each make tests fail when broken.

**Two review findings handled before merge**
- GitGuardian flagged the throwaway CI Postgres password as a generic secret. It was harmless (an ephemeral service container), but a later commit cannot clear a flag on an earlier commit, so the first PR (#12) was closed and the work re-opened from a clean single commit with no password anywhere (CI and local use `trust` auth). No force-push was needed.
- The automated security review pointed out that the audit log shared the events table's erasure escape hatch. Fixed, tests first: `audit_log` now has its own unconditional append-only trigger (`forbid_audit_changes`), and a test proves it cannot be deleted even with `clearcut.erasure = 'on'`. Erasure applies to events only. Still open for P1-04: restrict who may set `clearcut.erasure` to a dedicated role (today any session that can run SQL can); also recorded in the migration comment.

**Gap, stated honestly**: removing `order by seq` from `load_events` is not caught. On tiny tables Postgres returns rows in insertion order anyway, so a faithful test would be flaky. The ordering is in the SQL and documented; revisit with a real-size fixture if it ever matters.
**Note**: plain `cargo test --workspace` now needs `DATABASE_URL` (the store tests fail with a clear message without it); CONTRIBUTING.md and CLAUDE.md say so.
**Next**: P1-02 (object storage and upload), P1-03 (job queue, which decides the `jobs` table), P2-04 (worker skeleton and provider trait).

## 2026-10-20 (part 6) — Re-extraction runs in event IDs (decision from ADR-0006)
**Done** (own branch and PR; tests first; 10 new, core now 191)
- `derive_events(extraction, run, earlier)`: IDs are now `<document>:r<run>:<slug>`. Each new event supersedes the *current head* of the same slug from an earlier run (run 3 supersedes run 2, not run 1). Events of other documents, and of this or later runs, are ignored, so re-deriving the same run (a retried job) gives identical events.
- A fact a person confirmed or corrected (`Extractor::Human` head) is never superseded by a machine re-run; both stay current for a reviewer to reconcile.
- Mutation-checked: human protection, later-run filtering and the same-run boundary each fail a test when broken; the same-run case was a missing test, now added.

**Known limits (documented in the module and ADR-0006)**: a fact the new run no longer finds stays current (no retraction yet), so a stale finding such as a mismatch a better model fixed will linger until a reviewer deals with it. A retraction event may be added if this proves noisy (P2-09).

## 2026-10-20 (part 5) — P1-01 schema proposal (docs only)
**Done**
- Checked the environment for P1-01: Docker Desktop is now running, but there is no Postgres image locally and a native Postgres is already listening on port 5432 (not ours; left alone). Nothing was downloaded or started.
- Drafted [ADR-0006](adr/0006-postgres-schema.md): `tenants`, `documents`, `events`, `audit_log`; events stored as canonical core `Event` JSON with generated `event_type`/`supersedes` columns; append-only enforced by triggers (with a POPIA erasure escape hatch); tenant safety via composite foreign keys; hash de-duplication per tenant. Status: **Proposed**. CLAUDE.md requires Kone to see the schema before it is applied.

**Push-back recorded**: `jobs` is left to P1-03 (queue semantics decide its columns) and there is no `extractions` table until P2-06 / P3-09.
**Open questions in the ADR**: re-extraction and event-ID collisions, erasure vs anonymisation, retention.
**Next (after review)**: pull the official `postgres` image and run a dev container on a spare port (needs permission; it is a download), then write the two migrations, a small `clearcut-store` crate with a round-trip test (derive events, store, load, compare), and a Postgres service in CI.

## 2026-10-20 (part 4) — P3-03 event derivation
**Done** (own branch and PR, stacked on the P2-08 PR; tests first; core now 181, workspace 184)
- `extraction::derive_events(&Extraction) -> Result<Derived { validation, events }>`: runs `validate`, then derives events in a fixed order: supplier, each line (as printed), VAT per tax category present, stated totals, due date, credit-note reference, then one event per finding.
- **Printed facts** are attributed to the extractor and cite the page/locator they were read from, with that value's confidence. **Calculated VAT** and **findings** are `System` events: VAT uses the first line of the category as its citation and the weakest line confidence; a wrong line total or printed total cites the printed value it is about; other findings become numbered `ComplianceIssueFound` events (`doc:issue-N`), citing the relevant line/due date or page 1.
- IDs are deterministic (`doc_1:line-2`, `doc_1:totals-mismatch-vat`), so deriving twice gives identical events and the core needs no randomness or clock.
- New event kind `LineTotalMismatch` (group Charges, severity Critical) with its template: "On line 2 the stated total of R1 100.00 does not match the recomputed R1 000.00." (no invented difference).
- `Confidence::CERTAIN` constant; shared test builders in `extraction::fixtures`.
- End-to-end test: derived events narrate and pass the number guard, for a clean and a deliberately messy extraction. Mutation-checked four rules (weakest confidence, mixed-currency line skip, line-mismatch citation, VAT only for trusted lines).

**Decisions**
- A fact with amounts in two currencies cannot be an event of its own kind, so it becomes a compliance-issue event with the same ID instead of an error. An invalid location (bad bounding box) from the extractor *is* an error: better a visible rejection than quietly losing citations. The orchestrator should treat that as `needs_review`.
- A line in the wrong currency is not narrated as a line; its finding explains why. A line with a wrong total is still narrated as printed, beside an event saying it is wrong.
- **By-party view dropped** (was deferred in P3-02): it needs a party identity that survives across documents, which belongs with storage.
- Recipient is not yet an event (no `RecipientIdentified` kind); add when a use needs it.

**Phase 3 status**: the pure-Rust part is complete. Remaining items need infrastructure: P3-05 (provider trait, Phase 2), P3-08 (UI contract), P3-09/P3-10 (storage, MCP upgrade).
**Next**: P1-01/P1-02 (Postgres schema and upload storage; Docker Desktop must be running, and the schema should be shown to Kone before it is applied) and P2-01 (gold set, needs Kone). P2-08 duplicate detection is still open.

## 2026-10-20 (part 3) — Extraction model and deterministic validation (P2-08, first half)
**Done** (own branch and PR; tests first; 27 new, core now 157, workspace 160)
- `clearcut-core::extraction`: `Extraction` is what an extractor reports for one document (untrusted; JSON-friendly; only document, extractor and currency required). Every value is `Sourced<T>` = value + page + optional locator + confidence, so each fact can later be cited. Derives `schemars::JsonSchema` under the feature, ready to become the structured-output schema for the worker (P2-06).
- `validate(&Extraction) -> Validation { recomputed, findings }`, never fails and never edits: no lines, line currency mismatch, non-positive quantity, amounts out of range, line total vs quantity x price, printed net/VAT/gross vs totals rebuilt from the lines, sums out of range, due date before invoice date, and the `compliance_za` field rules (on the printed gross, else the recomputed one; rand only). Findings come in a fixed order. Both the printed and recomputed values are kept in each finding.
- Mutation-checked three rules (VAT comparison, rand-only, due-date boundary); each is caught.

**Decisions**
- Exact-to-the-cent comparison: a one-cent difference from per-line vs per-category VAT rounding is reported as a finding (safe direction); the gold set (P2-01) will tell us whether to allow a tolerance.
- Totals are rebuilt from quantity x unit price, not from the printed line totals; a wrong printed line total is its own finding.
- Compliance rules are skipped when no amount is known or the currency is not ZAR (thresholds are in rand).
- The by-party view deferred in P3-02 is dropped for now: a party needs an identity that survives across documents, which belongs with storage.
- Not done in P2-08: duplicate detection (needs other documents as input).

**Next**: P3-03 derive events from an extraction plus its validation (deterministic event IDs, new `LineTotalMismatch` event type).

## 2026-10-20 (part 2) — P3-07 narration modes
**Done** (own branch and PR; tests first; 16 new, core now 130, workspace 133)
- `clearcut-core::narration::report`: `narrate(events, NarrationMode)` returns a `Report` of `Section`s. `Timeline` is one untitled section (same sentences as `narrate_timeline`); `GroupedReport` is one titled section per non-empty group in taxonomy order (headings: Parties, Charges, VAT, Totals, Payment terms, Adjustments, Compliance, Risks, …); `Summary` is one untitled section with the key facts first (supplier, stated totals, due date, credit notes) then Critical then Warning events, leaving line detail out.
- `Report::to_narrative()` flattens any mode so the number guard can check it; a test runs the guard over all three modes.
- Mutation-checked: breaking the summary filter fails 2 tests.

**Decisions**
- Modes never add connective sentences such as "3 issues were found": a count is in no event, so the number guard would reject it, and an uncited sentence cannot exist. Summary and headings therefore carry no derived numbers.
- Empty input gives no sections rather than one empty section.
- `NarrationMode` is serde snake_case (`summary`, `timeline`, `grouped_report`) so it can be the parameter of the future `narrate_document` MCP tool (P3-10).

**Next**: P3-03 (event derivation, party view) is the last pure-Rust Phase 3 item and needs a minimal validated-extraction shape, best designed with P2-08. After that the work needs infrastructure (P1-01/P1-02 schema and storage; Docker Desktop must be running) or the gold set (P2-01).

## 2026-10-20 — Dependabot PRs cleared; P3-06 number guard
**Done**
- Merged Dependabot PRs #3, #4, #5; fixed and merged #1 (rmcp 3.5.1: `ServerInfo` → `ServerConfig`; MCP smoke-tested over stdio) and #2 (TypeScript 7.0.2: needs `"types": ["node"]` in `packages/sdk/tsconfig.json`). Note rmcp 3.5.1 still speaks protocol 2025-06-18, so P1-06 is unchanged.
- **P3-06 number guard** (`clearcut-core::narration::guard`), tests first, on its own branch and PR. 36 new tests; core now 114, workspace 117. Mutation-checked: disabling the date check or the sign handling makes tests fail.
  - `check(narrative, events)` returns violations; `guard(candidate, events)` returns the candidate if clean, else template sentences for the cited events (once each, in citation order).
  - Per sentence, the allowed facts are the figures and dates in the template wording of the cited events plus their own dates. Because it reuses the narrator's wording, guard and narrator cannot disagree on formats.
  - Dates are checked as whole units (`30 May 2026` fails if the event says 30 April, even though 30 and 2026 are both "present"); ISO dates are accepted when they are the same day; impossible dates fail. Signs are part of the number. Cited events must exist and not be superseded.

**Decisions / findings**
- Deliberately strict: `R1500` (no decimals), `R15 000.00` with an ordinary space, abbreviated months and `1,500.00` are rejected. A false rejection only means falling back to template text; a false acceptance would put a wrong number in a financial narrative.
- Known limits, written into the module docs: numbers written as words ("ten") are not seen, and a sentence can mislead while every figure in it is real. Template wording of facts is the main defence; the guard is the second.
- `INV-0042` scans as `-0042` (a hyphen before digits reads as a sign). That is consistent between template and candidate, so it only matters if a rewrite drops the hyphen, which is then rejected (safe direction).
- A first sed-based mutation silently did not apply and looked like a passing test; re-did it with a real edit. Lesson: confirm a mutation actually changed the file.

**Next**: P3-07 narration modes (summary, timeline, grouped report) and P3-03 event derivation. P3-05 (LLM smoothing) waits for the provider trait (Phase 2). P2-01 gold set is still the long pole.

## 2026-10-19 (Windows session, part 3) — P3-04 template narrator
**Done** (tests first; 24 new, core now 78, workspace 81)
- `clearcut-core::narration`: `narrate_event` (one sentence per event, citing it), `narrate_timeline` (current events in timeline order), `Sentence` (private fields; `Sentence::new` rejects an empty citation list, so an uncited sentence cannot exist) and `Narrative` (serialises to `{"sentences":[{"text","cites"}]}`). One template per event kind, with no wildcard arm so a new kind will not compile without wording.
- `narration::format`: Rand as `R15 000.00` (no-break-space thousands separators), `-R5.00`, other currencies as `USD 10.00`; quantities from thousandths and rates from basis points with trailing zeros dropped; dates as `31 March 2026`.
- New `Error::InvalidNarration`.

**Decisions / findings**
- Templates print **only numbers present in the cited events**. A mismatch sentence shows stated and recomputed amounts but never their difference: a computed figure no event contains would fail the number guard (P3-06).
- Formatting lives in one public module so P3-06 can format the cited events' facts the same way and require every narrated figure to match one exactly, instead of guessing at number formats.
- Free-text fields (supplier name, compliance message) are inserted verbatim. They come from untrusted documents, so the LLM-smoothing step (P3-05) must treat narration text as data, never as instructions.
- A test of mine was too loose (`"R2 250.01"` contains `"0.01"`); tightened to `"R0.01"`. Clippy's `is_multiple_of` suggestion was avoided by rewriting the digit grouping, since I could not confirm the method exists on the declared minimum Rust (1.88).
- Open for P3-06: dates are written out in words, so the guard must recognise "31 March 2026" as one date, not as the numbers 31 and 2026.

**Next**: P3-06 number guard. Nothing pushed; P3-02 is committed, P3-04 is not.

## 2026-10-19 (Windows session, part 2) — P3-02 grouping views
**Done** (tests first; 13 new, core now 54, workspace 57)
- `Severity` (`Info < Warning < Critical`), derived from the event type like `group`: totals mismatch is Critical; compliance issue and possible duplicate are Warning; the rest Info. `Event::severity()`.
- `clearcut-core::event::view`: `current` (drops superseded events, follows correction chains), `timeline` (date, undated last, then group, then ID), `by_group` (taxonomy order), `by_document`, `by_severity` (most severe first). Empty buckets are omitted. Every view applies `current` first so a corrected event never sits beside its replacement. Output order is fully deterministic (ID is the final tie-break).

**Decisions / findings**
- **Party view deferred to P3-03**: events have no party field, so grouping by party now would be a guess. Revisit once derivation decides how parties attach to events.
- A `supersedes` link to an event not in the slice is ignored. A cycle (A supersedes B and B supersedes A) would hide both events; that can only come from corrupt stored data, so the store should prevent it (note for P1-01).
- Rust lesson of the session: `by_document` returns references into the events, so the helper needed an explicit lifetime (`'a`) to say the keys borrow from the same slice.

**Next**: P3-04 template narrator, then P3-06 number guard. Nothing pushed yet (2 local commits ahead of origin/main; P3-02 is still uncommitted).

## 2026-10-19 (Windows session) — Rust toolchain, P3-01 event model
**Done**
- Local `main` was 6 commits behind `origin/main` (no `CLAUDE.md`, research 05, ADR-0004/0005); fast-forwarded.
- Installed Rust via `winget install Rustlang.Rustup` (rustc/cargo 1.99.1, stable-msvc; MSVC Build Tools were already present). Baseline before changes: 17 core + 3 server tests green, fmt and clippy clean. Docker CLI 29.5.2 is installed but Docker Desktop was not running (not needed until P1-01/P1-07).
- **P3-01** `clearcut-core::event`, tests first (red = compile errors, then green): `Event` (immutable, private fields, built via `Event::new(EventParts)`), `EventPayload` (9 typed variants over 8 of the 10 groups), `EventType`/`EventGroup` taxonomy, `Citation` (1-based `NonZeroU32` page, optional text span or per-mille bounding box), `Extractor`, `Confidence` (integer 0–1000), `EventId`/`DocumentId` newtypes. 24 new tests including a golden-JSON test; core now has 41 tests, workspace 44.
- Fixed a false-green SDK test script: `node --test dist/` ran 1 vacuous test; now `node --test "dist/**/*.test.js"` runs the real 2.

**Design decisions**
- Payload is an enum with typed fields, not a JSON map; amounts are `Money` (ADR-0002). `group` is derived from the type, never stored, so they cannot disagree.
- Corrections are new events with `supersedes` (append-only, ADR-0004). Deserialising JSON re-runs the same validation as `Event::new` (`serde(try_from)`).
- Dates use the `time` crate (`Option<Date>`, ISO `YYYY-MM-DD`): the date is the fact's date and may be absent.
- `money_fields()` on a payload exists for the future number guard (P3-06).
- IDs are supplied by the caller; the core never generates them (stays I/O-free).

**Findings**
- `time` 0.3.55 needs Rust 1.88, so workspace `rust-version` was raised from 1.85 to 1.88. CI uses `stable` so it was unaffected.
- With the workspace `schemars` feature on (server/mcp enable it), `EventParts` needs `JsonSchema` too because `Event` deserialises through it.
- `EventGroup` has `Document` and `Payments` but no event types yet; added when ingestion (P2) and payments exist.

**Not done / next**: nothing committed or pushed yet. Next pure-Rust steps: P3-02 grouping views, P3-04 template narrator, P3-06 number guard. P2-01 gold set is still the long pole.

## 2026-10-19 (night) — "Boosted memory" defined
Kone: the AI must be given every resource it needs to achieve its task; everything else as we go. Recorded as the **context-pack / resource-completeness** principle in ADR-0005 §4 and task P2-16b. Kept two limits: include what the task needs (POPIA minimisation, no cross-tenant data) and keep memory writes human-confirmed. No code changed.

## 2026-10-19 (evening) — Provider, escalation, exports, accuracy decisions
**Decisions from Kone:** Anthropic API; escalate from smaller to larger models on failure; high thinking and "boosted memory" so tasks complete; popular export formats; accuracy targets unknown.
**Done**
- [ADR-0005](adr/0005-model-strategy-escalation.md): Haiku 5.5 → Sonnet 5.5 → Opus 5.5 → Fable 5.1 → human review; validation-driven escalation; adaptive thinking; prompt caching, chunking and a memory tool; budgets; ZAR cost illustration; POPIA position.
- Research 05: export formats now PDF, Excel, Word, CSV, JSON, text/Markdown/HTML on a shared schema; proposed accuracy targets table.
- PHASES.md: added P2-13…P2-19 and rewrote Phase 4 export tasks.
**Pushback recorded:** high thinking on every call does not guarantee completion; guarantees come from validation, retries, escalation and a `needs_review` terminal state. Memory must not be written from uploaded-document text (prompt-injection persistence).
**Findings:** no South African/African inference region is documented by Anthropic (global or US only); third-party source says default retention is 30 days and ZDR is by request; all unverified with Anthropic.
**No code changed.**

## 2026-10-19 (later) — Gap review: ingestion, narration, grouping, export
**Trigger:** Kone asked whether "narration" was in the research. It was not: research 01–04 only covered issuing invoices.
**Done**
- Added [research 05](research/05-ingestion-narration-export.md): OCR vs LLM vs hybrid, long-document strategy, validation, event taxonomy, guarded narration, export formats, evaluation plan, POPIA.
- Added [ADR-0004](adr/0004-ingestion-and-narration-pipeline.md); revised ADR-0001 (Python worker moves to Phase 2).
- Rewrote PHASES.md: ingestion → events/narration → export now come before issuing, e-invoicing and payments. Task IDs were renumbered.
**Decisions taken on Kone's behalf (to confirm):** support both received and issued invoices; narration is template-first with a number guard; events are the single source of truth.
**Risks logged:** accuracy figures are vendor claims; model hallucination on numbers; POPIA s72 cross-border processing and client confidentiality; gold set needs real invoices.
**No code changed** in this entry.

## 2026-10-19 — Research, scaffold, first working slice
**Done**
- Researched the invoicing market, SA VAT/e-invoicing, MCP 2026-07-28 and Rust MCP SDK (`docs/research/01–04`).
- Chose a Rust-core, TypeScript-edge stack; Python deferred (ADR-0001).
- `clearcut-core`: money (integer cents, half-away rounding), VAT categories, invoice totals, status machine, SA thresholds and field validation. 17 unit tests passing.
- `clearcut-server`: axum REST with `/health` and `/v1/invoices/calculate`. 3 tests passing.
- `clearcut-mcp`: stdio MCP server, 3 tools. Smoke-tested by hand over JSON-RPC: `initialize`, `tools/list`, structured `tools/call` all OK.
- `@clearcut/sdk`: typed client, 2 tests passing.
- Added Apache-2.0 licence, NOTICE, CI workflow, Dependabot, issue/PR templates, SECURITY, CONTRIBUTING, CODE_OF_CONDUCT, CODEOWNERS.

**Findings that change plans**
- MCP prototype runs rmcp **2.2.0** / protocol 2025-06-18, not the 2026-07-28 stateless spec (task P1-06).
- Sources disagree on SA e-invoicing dates (2028 vs. mandatory from 2030). Plan for the earlier date.
- TypeScript types are hand-mirrored from Rust; drift risk (P1-08).

**GitHub setup (same day)**
- Repo `konethegreat/clearcut` created by the owner (the build sandbox cannot create repos), pushed from the sandbox after the Claude GitHub App was granted access.
- Apache-2.0 detected by GitHub; first CI run green.
- Description, 10 topics and 6 labels set. Dependency graph, Dependabot alerts, private vulnerability reporting enabled; secret scanning and push protection were already on.
- Not done: branch protection / rulesets, Dependabot security-update toggle not verified.
- Note: repo-settings writes are blocked via the sandbox API proxy, so settings were changed through the signed-in browser.

**Next**: see "Immediate next three tasks" in PHASES.md.
