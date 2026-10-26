-- Written by Kone Tshivhinda & Claude
-- ADR-0018 section 8: what the model router stores.
--
-- page_readings: one row per page. While the page is being read it holds the carry (the state
-- machine and the readings kept so far); when it ends it holds the final reading, or the best
-- draft of a page sent to a person. IT HOLDS INVOICE CONTENT, so it is erased with the document.
--
-- model_calls: one row per call the router made or considered. NO CONTENT AT ALL: ids, names,
-- sizes, token counts and rand. Append-only. It is what the month's spending is added up from,
-- so it is NOT erased with the document (and has no foreign key to documents).
--
-- Run this as the owner role, never as the application role.

-- ------------------------------------------------------------ page_readings ----
create table page_readings (
    tenant_id        uuid     not null,
    document_id      uuid     not null,
    page             integer  not null check (page > 0),
    state            text     not null check (state in ('in_progress', 'read', 'needs_review')),
    -- The stored state machine and kept readings of a page that is not finished
    -- (clearcut-router `Carry`). Null once the page has ended.
    carry            jsonb    check (pg_column_size(carry) <= 262144),
    -- Why a page went to a person (clearcut-core `ReviewReason`).
    review_reason    text     check (review_reason in (
                         'top_rung_failed', 'too_long', 'refused', 'budget', 'call_cap',
                         'cannot_be_sent_safely', 'request_rejected', 'no_rung_available',
                         'amounts_include_vat')),
    -- The reading that counts (or the best draft): its rung, attempt, evidence and model.
    rung             smallint check (rung between 0 and 7),
    attempt          integer  check (attempt between 1 and 50),
    confidence_basis text     check (confidence_basis in ('text_confirmed', 'arithmetic_only', 'unresolved')),
    model            text     check (char_length(model) between 1 and 100),
    reading          jsonb    check (pg_column_size(reading) <= 262144),
    -- What was left out of it, the checks that failed, what was noted, and the places of the
    -- figures the page's text did not confirm. Fixed names and places; `findings` carries amounts.
    problems         jsonb    not null default '[]' check (jsonb_typeof(problems) = 'array' and pg_column_size(problems) <= 65536),
    findings         jsonb    not null default '[]' check (jsonb_typeof(findings) = 'array' and pg_column_size(findings) <= 65536),
    notes            jsonb    not null default '[]' check (jsonb_typeof(notes) = 'array' and pg_column_size(notes) <= 65536),
    unconfirmed      text[]   not null default '{}' check (cardinality(unconfirmed) <= 500),
    injection_suspected text[] not null default '{}' check (cardinality(injection_suspected) <= 8),
    created_at       timestamptz not null default now(),
    updated_at       timestamptz not null default now(),
    primary key (document_id, page),
    foreign key (tenant_id, document_id) references documents (tenant_id, id),
    check (
        (state = 'in_progress' and carry is not null and review_reason is null and reading is null
            and rung is null and attempt is null and confidence_basis is null and model is null)
        or (state = 'read' and carry is null and review_reason is null and reading is not null
            and rung is not null and attempt is not null and model is not null
            and confidence_basis in ('text_confirmed', 'arithmetic_only'))
        or (state = 'needs_review' and carry is null and review_reason is not null
            and ((reading is null and rung is null and attempt is null and confidence_basis is null and model is null)
              or (reading is not null and rung is not null and attempt is not null and model is not null
                  and confidence_basis = 'unresolved')))
    )
);
create index page_readings_by_document on page_readings (tenant_id, document_id, page);

-- The application reads, adds, updates (a page moves from in_progress to its end) and removes
-- (a re-run replaces a document's rows). Derived data, like page_reads, so not append-only.
grant select, insert, update, delete on page_readings to clearcut_app;
alter table page_readings enable row level security;
create policy app_read   on page_readings for select to clearcut_app using (tenant_id = current_tenant());
create policy app_insert on page_readings for insert to clearcut_app with check (tenant_id = current_tenant());
create policy app_update on page_readings for update to clearcut_app
    using (tenant_id = current_tenant()) with check (tenant_id = current_tenant());
create policy app_delete on page_readings for delete to clearcut_app using (tenant_id = current_tenant());

-- Erasing a document takes its readings with it.
grant select, delete on page_readings to clearcut_eraser;
create policy eraser_read   on page_readings for select to clearcut_eraser using (true);
create policy eraser_delete on page_readings for delete to clearcut_eraser using (true);

-- ------------------------------------------------------------- model_calls ----
create table model_calls (
    id               bigint generated always as identity primary key,
    tenant_id        uuid    not null references tenants (id),
    -- Plain ids, no foreign keys: the record of spending outlives the document.
    document_id      uuid    not null,
    job_id           uuid,
    page             integer not null check (page > 0),
    attempt          integer not null check (attempt between 1 and 50),
    rung             smallint not null check (rung between 0 and 7),
    model            text    not null check (char_length(model) between 1 and 100),
    effort           text    not null check (effort in ('low', 'medium', 'high')),
    picture_dpi      smallint not null check (picture_dpi between 100 and 300),
    picture_max_edge smallint not null check (picture_max_edge between 500 and 2576),
    call_kind        text    not null check (call_kind in ('first', 'escalation', 'retry_truncated', 'retry_invalid')),
    escalated_from   smallint check (escalated_from between 0 and 7),
    escalation_reason text   check (escalation_reason ~ '^[a-z_]{1,40}$'),
    -- Only an escalation has an origin and a reason, and an escalation has both.
    check ((call_kind = 'escalation') = (escalation_reason is not null)
       and (call_kind = 'escalation') = (escalated_from is not null)),
    prompt_version   text    not null check (char_length(prompt_version) between 1 and 64),
    -- The pack's manifest (ADR-0014): section names and sizes, hashes, counts. Never text.
    -- Null when no request was built.
    manifest         jsonb   check (jsonb_typeof(manifest) = 'object' and pg_column_size(manifest) <= 16384),
    input_tokens          bigint not null default 0 check (input_tokens >= 0),
    output_tokens         bigint not null default 0 check (output_tokens >= 0),
    cache_read_tokens     bigint not null default 0 check (cache_read_tokens >= 0),
    cache_write_tokens    bigint not null default 0 check (cache_write_tokens >= 0),
    cache_write_1h_tokens bigint not null default 0 check (cache_write_1h_tokens between 0 and cache_write_tokens),
    -- What the budget was charged, in nano-rand (a billionth of a rand). For a call that may
    -- have been billed but reported nothing (a timeout) this is the worst case, not a bill.
    cost_nano_rand   bigint  not null default 0 check (cost_nano_rand >= 0),
    elapsed_ms       integer not null default 0 check (elapsed_ms >= 0),
    request_id       text    check (char_length(request_id) <= 100),
    label            text    not null check (label in (
                         'accepted', 'escalated', 'reviewed', 'invalid_output', 'truncated',
                         'refused', 'rejected', 'rung_unavailable', 'not_sent', 'interrupted')),
    -- The reason or the kind that goes with the label (a fixed name).
    label_detail     text    check (label_detail ~ '^[a-z_]{1,40}$'),
    created_at       timestamptz not null default now()
);
create index model_calls_by_page  on model_calls (tenant_id, document_id, page, id);
create index model_calls_by_month on model_calls (tenant_id, created_at);

-- Append-only, with no erasure escape hatch (the same trigger function as the audit log).
create trigger model_calls_append_only before update or delete on model_calls
    for each row execute function forbid_audit_changes();

grant select, insert on model_calls to clearcut_app;
grant usage on all sequences in schema public to clearcut_app;
alter table model_calls enable row level security;
create policy app_read   on model_calls for select to clearcut_app using (tenant_id = current_tenant());
create policy app_insert on model_calls for insert to clearcut_app with check (tenant_id = current_tenant());

-- ----------------------------------------------------------------- erasure ----
-- The same function as in 0006 with one more delete (page_readings). model_calls is kept on
-- purpose. It keeps its owner, search_path and grants (create or replace changes none of them).
create or replace function erase_document(p_tenant uuid, p_document uuid) returns void
    language plpgsql
    security definer
    set search_path = public, pg_temp
as $$
begin
    if p_tenant is null or p_tenant is distinct from current_tenant() then
        raise exception 'erase_document: tenant does not match the current tenant'
            using errcode = '42501';
    end if;
    if not exists (select 1 from documents where id = p_document and tenant_id = p_tenant) then
        raise exception 'erase_document: document not found' using errcode = 'P0002';
    end if;
    delete from page_readings where document_id = p_document and tenant_id = p_tenant;
    delete from page_reads    where document_id = p_document and tenant_id = p_tenant;
    delete from jobs          where document_id = p_document and tenant_id = p_tenant;
    delete from events        where document_id = p_document and tenant_id = p_tenant;
    delete from documents     where id = p_document and tenant_id = p_tenant;
    insert into audit_log (tenant_id, actor, action, subject_type, subject_id, detail)
    values (p_tenant, '{"type":"system"}', 'document.erased', 'document', p_document::text, '{}');
end $$;
