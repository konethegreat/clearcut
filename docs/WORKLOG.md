# Worklog

_Newest first. Append an entry for each working session. Written by Kone Tshivhinda & Claude._

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
