# ADR-0006: Postgres schema v1 (tenants, documents, events, audit log)

Status: **Proposed, awaiting review by Kone Tshivhinda** · Date: 2026-10-20 · Deciders: Kone Tshivhinda, Claude
Implements task P1-01. Nothing here has been applied to any database yet.

## Context
Phase 1 needs durable storage for the pipeline in [ADR-0004](0004-ingestion-and-narration-pipeline.md): uploaded documents, the **events** that are the source of truth, and an audit trail. `clearcut-core` already defines what an event is and derives events with deterministic string IDs (`<document>:line-2`). The schema should store events faithfully, make them append-only, and keep one customer's data away from another's (POPIA, ADR-0005).

## Proposal in brief
1. **Four tables now:** `tenants`, `documents`, `events`, `audit_log`.
2. **Events are stored as the canonical JSON of the core `Event`** (`body jsonb`), plus a few *generated* columns for querying. Generated columns cannot drift from the JSON, and reading an event back goes through `Event`'s own validation.
3. **Append-only is enforced by the database**, not by convention: triggers reject `UPDATE` and `DELETE` on `events` and `audit_log`, with one explicit escape hatch for POPIA erasure.
4. **Tenant safety is structural:** every table carries `tenant_id`, and child rows reference their parent with a *composite* foreign key `(tenant_id, id)`, so a row cannot point at another tenant's document. Row-level security comes with P1-04.
5. **Terminal states only:** a document ends `done` or `needs_review` (ADR-0005); `failed` does not exist.

### Where I deviate from the task list (push-back)
- **`jobs` is left out of this migration.** A job queue (leases, `FOR UPDATE SKIP LOCKED`, retries, cancellation, progress) is its own design problem (P1-03). Guessing columns now would mean rewriting them. `documents.status` is enough to start.
- **No `extractions` table yet.** Storing the raw extraction, model/prompt versions and raw output is P2-06 / P3-09, when we know what the worker returns.

## Schema (SQL, as it would be applied by two small `sqlx` migrations)

```sql
-- 0001_tenants_documents.sql
create table tenants (
    id         uuid primary key default gen_random_uuid(),
    name       text not null check (btrim(name) <> ''),
    created_at timestamptz not null default now()
);

create table documents (
    id                uuid primary key default gen_random_uuid(),
    tenant_id         uuid not null references tenants (id),
    original_filename text not null,
    content_type      text not null,
    byte_size         bigint not null check (byte_size >= 0),
    sha256            bytea  not null check (octet_length(sha256) = 32),
    storage_key       text   not null,       -- key in object storage (P1-02), never a path
    page_count        integer check (page_count > 0),
    status            text   not null default 'received'
                      check (status in ('received', 'processing', 'done', 'needs_review')),
    created_at        timestamptz not null default now(),
    updated_at        timestamptz not null default now(),
    unique (tenant_id, sha256),              -- de-duplication by content (P2-02)
    unique (tenant_id, id)                   -- target for composite foreign keys
);
```

```sql
-- 0002_events_audit.sql
create table events (
    seq         bigint generated always as identity,   -- total insertion order
    tenant_id   uuid not null,
    document_id uuid not null,
    id          text not null check (btrim(id) <> ''),  -- the core EventId, e.g. '<doc>:line-2'
    body        jsonb not null,                         -- canonical core `Event` JSON
    -- Queryable copies, generated from `body` so they can never disagree with it:
    event_type  text generated always as (body #>> '{payload,type}') stored not null,
    supersedes  text generated always as (body ->> 'supersedes') stored,
    recorded_at timestamptz not null default now(),
    primary key (document_id, id),
    foreign key (tenant_id, document_id) references documents (tenant_id, id),
    check (body ->> 'id' = id),
    check (body #>> '{citation,document}' = document_id::text)
);
create unique index events_seq on events (seq);
create index events_by_document on events (tenant_id, document_id, seq);
create index events_superseded on events (document_id, supersedes) where supersedes is not null;

create table audit_log (
    id           bigint generated always as identity primary key,
    tenant_id    uuid not null references tenants (id),
    at           timestamptz not null default now(),
    actor        jsonb not null,                 -- {"type":"api_key","id":..} | {"type":"user",..} | {"type":"system"}
    action       text  not null,                 -- 'document.uploaded', 'extraction.reviewed', ...
    subject_type text  not null,
    subject_id   text  not null,
    detail       jsonb not null default '{}'
);
create index audit_by_subject on audit_log (tenant_id, subject_type, subject_id, id);

-- Append-only. DELETE is allowed only inside a transaction that has said
-- `set local clearcut.erasure = 'on'` (POPIA deletion tooling, P8-06).
create function forbid_changes() returns trigger language plpgsql as $$
begin
    if tg_op = 'DELETE' and current_setting('clearcut.erasure', true) = 'on' then
        return old;
    end if;
    raise exception '% is append-only (% refused)', tg_table_name, tg_op;
end $$;

create trigger events_append_only    before update or delete on events
    for each row execute function forbid_changes();
create trigger audit_log_append_only before update or delete on audit_log
    for each row execute function forbid_changes();
```

## Why these choices
- **JSON body plus generated columns, not one column per field.** The event taxonomy will keep growing (new kinds, new payload fields). A column-per-field table would need a migration each time and would duplicate the validation that `Event` already does. With `body`, adding an event kind is a code change only. The generated `event_type` and `supersedes` columns give fast filtering and the "current events" query without a second source of truth. Dates are *not* generated: casting text to `date` is not immutable in Postgres; date queries use the JSON (and an expression index later if needed).
- **The `check` constraints tie `body` to the row** (`body.id = id`, `body.citation.document = document_id`), so a buggy writer cannot file an event under the wrong document.
- **`seq` gives a stable global order** for replaying or syncing the log, independent of timestamps.
- **Composite foreign keys** make "this event belongs to a document of the same tenant" a database fact. RLS (P1-04) then only has to filter on `tenant_id`.
- **Hash uniqueness per tenant, not global**, so two customers uploading the same public form never learn about each other.
- **Truncate is not blocked by row triggers.** The application database role (P1-04) must not hold `TRUNCATE`; noted here so it is not forgotten.

## Open questions (need a decision, not a guess)
1. **Re-extraction.** Derived event IDs are `<document>:<slug>`. If a document is extracted a second time (better model, new prompt), the same IDs would collide with the first run. Options: (a) put a run number in the ID (`<doc>:r2:line-2`) and supersede the old events; (b) treat re-extraction as a new document version. I lean to (a), but it affects P2-06 and P3-09, so I would rather decide with the worker design.
2. **Erasure.** Is hard deletion on request (POPIA) the right default, or should we anonymise in place? The `clearcut.erasure` escape hatch assumes hard deletion of a tenant's or a document's rows.
3. **Retention.** SARS requires 5-year retention of tax invoices. Whether uploaded *received* invoices fall under the same expectation for our customers is a question for the operator and legal review; the schema stores no expiry yet.

## Consequences
- Reading events back is `serde_json::from_value::<Event>(body)`, so stored data is re-validated every time it is read.
- Anything that needs to change an event writes a *new* event with `supersedes` (already how the core works).
- Local development needs Postgres 13+ (for `gen_random_uuid()`); the dev stack arrives with P1-07. Tests will use a throwaway database per test (`sqlx::test`) and CI will gain a Postgres service container.

## Rejected
- *One relational column per event field:* constant migrations and duplicated validation.
- *Storing events in a document-store or file:* loses transactional writes with the audit log.
- *`failed` as a document state:* contradicts "every job ends `done` or `needs_review`".
