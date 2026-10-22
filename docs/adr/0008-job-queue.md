# ADR-0008: Durable job queue in Postgres

Status: **Accepted** (approved by Kone Tshivhinda, 2026-10-21, with all five leanings below) · Date: 2026-10-21 · Deciders: Kone Tshivhinda, Claude
Design for task P1-03. Built in the four steps listed under "What gets built".

## Context
Reading an invoice takes seconds to minutes (OCR, several model calls, escalation per [ADR-0005](0005-model-strategy-escalation.md)), so uploads must return at once and the work must happen in the background, survive restarts, and always end somewhere. The promises we have already made:

- **Every job ends `done` or `needs_review`; nothing hangs, nothing silently fails** (ADR-0005, ADR-0006; accuracy target "100% of jobs terminal").
- **An outage stalls the pipeline; it does not lose work** (ADR-0005: "jobs queue and resume, state is in Postgres").
- **Progress, retries, cancellation, resumability** for long documents (ADR-0004, P1-03).
- **Tenants are isolated by row-level security** ([ADR-0007](0007-auth-and-row-level-security.md)): the service role only sees the tenant it names in each transaction.

Postgres is already the system of record, so the queue lives there: `FOR UPDATE SKIP LOCKED` is a well-understood, transactional queue, and enqueuing can happen in the same transaction as the write that caused it. No new infrastructure.

## Proposal in brief
1. **One new table, `jobs`.** A job is one stage of work on one document. A 300-page statement is **one job with page-level progress**, not 300 rows (see "Rejected").
2. **Five states:** `queued`, `running`, and three terminal ones: **`succeeded`, `needs_review`, `cancelled`**. There is no `failed`; a job that cannot be completed ends `needs_review` and its document does too, so a person looks at it.
3. **Leases, not locks.** A worker claims a job for a few minutes and must heartbeat. A crashed worker's lease expires and the job is claimed again. A **fencing token** makes a stale worker's late report harmless.
4. **All state changes go through five `SECURITY DEFINER` functions**, the same pattern as `erase_document` and `authenticate_api_key`. The service role can read and insert jobs but cannot `UPDATE` them directly. The document's status moves in the same transaction as the job's, so the two can never disagree.
5. **The queue is Rust-only.** The Python worker (P2-04) never touches the database; the Rust orchestrator claims jobs and calls the worker. No business rules leak into Python.
6. **A deadline on every job** guarantees the "no job hangs" promise even if a worker is down for days.

## Schema (proposed migration `0005_jobs.sql`)

```sql
create table jobs (
    id               uuid primary key default gen_random_uuid(),
    tenant_id        uuid not null,
    document_id      uuid not null,
    kind             text not null check (btrim(kind) <> ''),   -- validated by clearcut-core's JobKind; new kinds need no migration
    payload          jsonb not null default '{}'
                     check (jsonb_typeof(payload) = 'object' and pg_column_size(payload) <= 16384),
    state            text not null default 'queued'
                     check (state in ('queued', 'running', 'succeeded', 'needs_review', 'cancelled')),
    priority         smallint not null default 0,               -- higher runs first
    attempts         integer  not null default 0 check (attempts >= 0),
    max_attempts     integer  not null default 5 check (max_attempts between 1 and 20),
    run_after        timestamptz not null default now(),        -- backoff and deferral push this forward
    deadline_at      timestamptz not null default now() + interval '48 hours',

    -- lease: present exactly while running
    lease_token      uuid,
    leased_by        text,
    lease_expires_at timestamptz,

    -- progress and resumability (no invoice content in any of these)
    progress_done    integer not null default 0 check (progress_done >= 0),
    progress_total   integer check (progress_total >= 0),
    progress_note    text check (char_length(progress_note) <= 200),
    checkpoint       jsonb check (pg_column_size(checkpoint) <= 65536),
    last_error       text check (char_length(last_error) <= 500),

    cancel_requested_at timestamptz,
    created_at       timestamptz not null default now(),
    updated_at       timestamptz not null default now(),
    started_at       timestamptz,
    finished_at      timestamptz,

    unique (tenant_id, id),
    foreign key (tenant_id, document_id) references documents (tenant_id, id),
    -- the lease columns and the state must agree
    check ((state = 'running') = (lease_token is not null and leased_by is not null and lease_expires_at is not null)),
    check ((state in ('succeeded', 'needs_review', 'cancelled')) = (finished_at is not null))
);

-- At most one active job of a kind per document: enqueuing twice is harmless (idempotent).
create unique index jobs_one_active on jobs (tenant_id, document_id, kind) where state in ('queued', 'running');
-- What a worker scans. Tiny, because finished jobs are not in it.
create index jobs_claimable on jobs (priority desc, run_after, created_at) where state in ('queued', 'running');
create index jobs_running_by_tenant on jobs (tenant_id) where state = 'running';
create index jobs_by_document on jobs (tenant_id, document_id, created_at);
```

**Access.** Row-level security is enabled like the other tables. `clearcut_app` gets `select` and a restricted `insert` (policy: own tenant, `state = 'queued'`, `attempts = 0`, no lease), and **no `update` or `delete`**. A new login-less role `clearcut_queue` owns the functions below and holds the narrow rights they need: `update` on `jobs`, `update (status, updated_at)` on `documents`, `insert` on `audit_log`, each with its own policy.

### The five functions
| Function | Who calls it | What it does |
|---|---|---|
| `claim_job(worker, lease_seconds, kinds[], per_tenant_cap)` | any runner; **the only cross-tenant function** | Takes the best runnable job with `FOR UPDATE SKIP LOCKED`: `queued` and due, or `running` with an expired lease. **Sweeps as it goes**: a candidate past its `deadline_at`, or out of attempts after a lost lease, becomes `needs_review`; one with `cancel_requested_at` becomes `cancelled` (document and audit updated in the same transaction) and the scan moves on. Otherwise `attempts += 1`, new `lease_token`, document `processing`. Skips tenants already at `per_tenant_cap` running jobs. Returns the job or nothing. |
| `heartbeat_job(tenant, job, token, lease_seconds, done, total, note, checkpoint)` | the worker holding the lease | Extends the lease, stores progress and checkpoint, **returns whether cancellation was requested**. Wrong token or expired lease: error `lease_lost`. |
| `complete_job(tenant, job, token, outcome)` | the worker | `outcome` is `succeeded`, `needs_review` (with a short reason) or `cancelled` (acknowledging a cancellation request). Document becomes `done`, `needs_review` or `received`. Audited. |
| `fail_job(tenant, job, token, retry_after_seconds, counts, error)` | the worker | A failed run. If it counts and attempts are used up: `needs_review`. Otherwise back to `queued` with `run_after = now() + retry_after`. If it does **not** count (provider outage), `attempts` is given back, so an Anthropic outage cannot exhaust attempts; only the deadline ends it. |
| `cancel_job(tenant, job)` | the API | `queued` becomes `cancelled` at once; `running` gets `cancel_requested_at` and the worker finds out at its next heartbeat and finishes with `complete_job(cancelled)`. Terminal jobs are left alone. Audited. |

Every tenant-scoped function requires `p_tenant = current_tenant()` and a matching `lease_token` (except `cancel_job`, which needs only the tenant), exactly as `erase_document` does. Time is always the database's `now()`, never a worker's clock.

### Document status moves with the job
| Job event | Document becomes |
|---|---|
| claimed | `processing` |
| `succeeded` | `done` |
| `needs_review` (any reason: worker said so, attempts used up, deadline passed) | `needs_review` |
| `cancelled` | `received` (it can be processed again later) |

This table is fixed for every kind today. It lives in the SQL functions so the coupling is atomic and crash-safe; `clearcut-core` carries the same table, and a store test asserts that the two agree.

## Policy (in `clearcut-core`, pure and tested; the database just obeys the numbers it is given)
- **States and transitions** (`JobState`, `can_transition`), mirrored by a store test against the real functions.
- **Retry policy:** `max_attempts = 5`; backoff `30 s x 2^(attempt-1)`, capped at 15 minutes, with +/-20% jitter. Five attempts means four waits (30, 60, 120, 240 s), so a run of ordinary failures reaches `needs_review` after about 7.5 minutes; the cap only matters if `max_attempts` is raised (it first bites at the sixth wait). *(Corrected during implementation: the first draft said "about 45 minutes", which was wrong arithmetic. The approved numbers are unchanged.)* A provider outage is retried every 5 minutes (+/-20%) without counting, a figure added during implementation. The worker reports the failure kind and the orchestrator asks the policy for the delay; the database never computes a backoff.
- **Failure kinds:** `Transient` (counts; retry on the same tier first, as ADR-0005 says), `ProviderUnavailable` (does not count; retry later), `NeedsReview` (the top tier failed validation; straight to `needs_review`, no retry).
- **Lease:** 5 minutes, heartbeat every ~100 s. **Deadline:** 48 hours from enqueue (a parameter at enqueue; the default is a guess to be tuned).
- **Per-tenant cap:** a soft limit on concurrently running jobs per tenant so one large upload cannot starve everyone else. Soft because two concurrent claims can race past it by one; that is acceptable, the point is to stop 1,000 jobs from one tenant occupying every worker.

## What gets built, in order (each its own small PR, tests first)
1. **`clearcut-core::jobs`**: `JobKind`, `JobState`, transitions, `RetryPolicy`/backoff, `FailureKind`. Pure, no I/O.
2. **Migration `0005_jobs.sql` + store methods** (`enqueue_job`, `claim_job`, `heartbeat_job`, `complete_job`, `fail_job`, `cancel_job`, `get_job`, `jobs_for_document`). Also updates `erase_document` to delete a document's jobs (the foreign key would otherwise block erasure). Tests include two workers racing for one job, a worker dying mid-job, a stale worker's late report being refused, tenants not seeing each other's jobs, the document status coupling, and the deadline sweep.
3. **`clearcut-jobs` runner library**: a poll-claim-heartbeat-dispatch loop with a `Handler` trait (the real handlers arrive with P2-04+), so retries, progress and cancellation are exercised end to end with fake handlers.
4. **REST + SDK**: `GET /v1/documents/{id}/jobs` (needs `documents:read`) and `POST /v1/jobs/{id}/cancel` (needs `documents:write`). No new scopes.

**Not in P1-03:** wiring uploads to enqueue a job. Until a handler exists (P2-04), a job would sit in the queue and the document would never leave `received`. The upload will enqueue in the same transaction as the document row once there is something to run it.

## Implementation notes (step 2, migration 0005 and the store)
Built as proposed, with these refinements found while writing the tests:
- **A lease that has run out but has not been taken over still works** for the worker that holds its token. The ADR said an expired lease means `lease_lost`; but the token is the proof, nobody else holds the job, and refusing would throw away a slow worker's progress and pay for the run again. Once another runner has claimed it, the old token is dead.
- **The application role reads every job column except `lease_token`**, and may name only `tenant_id, document_id, kind, payload, priority, max_attempts, run_after, deadline_at` when queuing a job (column-level grants), so a SQL-injection-level caller can neither read a fencing token nor forge a running job. A test tries each forbidden column one at a time.
- **Two stronger table checks:** the lease columns are *all set while running and all clear otherwise* (the draft only forbade a lease on a non-running job in one direction), and `progress_done <= progress_total`.
- **Cancelling a job that never started leaves its document alone.** A finished document queued again for a re-run must not lose its `done` when that re-run is cancelled. Cancelling a job that was running returns the document to `received`, as decided.
- **`last_error` is the most recent failure message and is not erased by a later success.** Useful when a job needed a retry; a person reading it sees what went wrong on the way.
- **`fail_job` returns where the job ended up** (the Rust `FailureResult`): retrying, `needs_review`, or `cancelled`. The database has the last word: out of attempts, past the deadline, or cancellation requested end the job even when the caller asked for a retry. A provider outage is never "out of attempts", even on the final one.
- **Two helper functions** (`finish_job_internal`, `lock_leased_job`) are owned by the queue role and callable only from the five public ones. Each tenant-scoped function checks `p_tenant` against the current tenant and filters on it, because the queue role's policies match every row.
- `erase_document` now deletes the document's jobs first (the foreign key would otherwise block erasure). A worker whose document was erased under it gets `JobNotFound` at its next heartbeat.
- Not wired to uploads yet (decision 4). A new login-less role `clearcut_queue` appears in the database; like the others it has no password and nobody logs in as it.

## Implementation notes (step 3, the runner library)
`clearcut-jobs` is the loop between the queue and the work. It knows nothing about invoices; the work is a `Handler` (a trait the real pipeline implements from P2-04 on).
- **Per claimed job:** a background task sends the heartbeat at a steady rhythm (a third of the lease unless set) carrying the *latest reported* progress, while the handler runs on a task of its own. A heartbeat and a handler's own `report()` take the same lock, so an older progress can never overwrite a newer one.
- **A worker that loses its lease is stopped, not just told.** If a heartbeat finds the job taken over or erased, the handler's task is aborted at once and **nothing is reported**; the new holder's job is untouched. (A live worker that heartbeats cannot be robbed, which a test shows.)
- **Cancellation is cooperative:** the heartbeat sets a flag the handler reads with `is_cancelled()`; it ends with `Finished::Cancelled`.
- **Every handler result becomes a report.** `Finished::{Succeeded, NeedsReview, Cancelled}` complete the job; a `Failure` (transient, provider-unavailable, needs-review) goes through the core's `RetryPolicy`; **a panic is caught and treated as a transient failure**, so a bug in one handler cannot kill the runner or leave a job stuck.
- **Progress is durable:** `report(progress, note, checkpoint)` saves immediately; a retry's handler starts with `ctx.checkpoint()` from the last run, so long documents resume.
- **The loop** keeps up to `concurrency` jobs in flight, sleeps `poll_interval` when idle or when the database is unreachable (it logs and carries on rather than crashing), honours the per-tenant cap, and on the shutdown signal **stops claiming and waits for the jobs in flight to finish**.
- **Handlers must be safe to run twice** (the queue is at-least-once, not exactly-once); the document pipeline's event writes are already idempotent for this reason.

## Alternatives considered
- *One job per page, with a reduce step.* Needs dependencies between jobs ("reduce after all pages"), which is a workflow engine. A document-level job with page progress and a checkpoint gives resumability without one. Revisit only if per-page parallelism across workers is actually needed (P2-07).
- *Workers update `jobs` directly under RLS.* Simpler SQL, but any bug (or SQL injection) in the service could then rewrite attempts, leases or states. Five narrow functions are a smaller surface to reason about and test.
- *A separate queue (Redis, SQS, a Postgres extension).* More to run and secure, and enqueuing could no longer share a transaction with the document write.
- *`LISTEN/NOTIFY` for wake-ups.* Workers poll (an idle backoff of about a second, against a tiny partial index). NOTIFY can be added later as a latency optimisation without changing anything else.
- *A `failed` state.* Contradicts ADR-0005: nothing may end silently failed. `needs_review` is the dead-letter queue, and the review queue (P2-09) reads from it.

## Risks and notes
- **The claim function is cross-tenant by necessity.** It is the one place the RLS guarantee is bridged deliberately; it returns only job identifiers and parameters, never document content, and everything after the claim runs in the owning tenant's transaction. If the API process and the runner later become separate deployments (P1-07), the runner should connect as its own login role holding only the queue functions, so a compromised API process cannot claim other tenants' jobs. For now they share `clearcut_app`.
- **No invoice content in jobs.** `payload`, `checkpoint`, `progress_note` and `last_error` carry ids, counters and short machine messages only. The size checks bound them; the rule that they hold no content is a convention to enforce in code review (and POPIA erasure deletes them with the document in any case).
- **Finished jobs are kept** (a few hundred bytes each) as history. A retention rule waits for the same legal review as the rest of retention (ADR-0006, decision 3).
- **Clock and polling cost** are small at our scale; the partial indexes keep the scan to unfinished jobs only.

## Decisions (Kone, 2026-10-21)
1. **One job per document stage, with page progress** (not a row per page).
2. **`needs_review` is the job's own terminal state**; `cancelled` returns the document to `received`.
3. **A provider outage does not consume attempts**; the **48-hour deadline** is the backstop (a parameter at enqueue, to be tuned with real data).
4. **No enqueue on upload yet**; add it together with the first handler (P2-04).
5. **The numbers stand as defaults:** 5 attempts, 30 s doubling to a 15-minute cap with +/-20% jitter, 5-minute lease.
