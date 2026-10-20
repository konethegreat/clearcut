-- Written by Kone Tshivhinda & Claude
-- ADR-0006: the append-only event log and audit log.

create table events (
    seq         bigint generated always as identity,   -- total insertion order
    tenant_id   uuid not null,
    document_id uuid not null,
    id          text not null check (btrim(id) <> ''),  -- the core EventId, e.g. '<doc>:r1:line-2'
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
-- TRUNCATE is not stopped by row triggers: the application role (P1-04) must
-- not be granted it.
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
