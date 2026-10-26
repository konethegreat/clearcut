-- Written by Kone Tshivhinda & Claude
-- ADR-0019: each tenant's own Anthropic key, encrypted by the service (AES-256-GCM, the tenant id
-- bound in) before it reaches the database. The database never holds a key in the clear.
-- Also: erase_document can no longer remove a document while a job is queued or running.

select pg_advisory_xact_lock(hashtext('clearcut_roles'));

do $$
begin
    begin
        create role clearcut_vault nologin nosuperuser nocreatedb nocreaterole nobypassrls;
    exception when duplicate_object or unique_violation then
        null;
    end;
    if exists (select from pg_roles where rolname = 'clearcut_vault'
               and not (not rolcanlogin and not rolsuper and not rolcreatedb
                        and not rolcreaterole and not rolbypassrls)) then
        alter role clearcut_vault nologin nosuperuser nocreatedb nocreaterole nobypassrls;
    end if;
end $$;

grant clearcut_vault to current_user;

create table tenant_anthropic_keys (
    tenant_id     uuid primary key references tenants (id),
    nonce         bytea not null check (octet_length(nonce) = 12),
    ciphertext    bytea not null check (octet_length(ciphertext) between 17 and 512),
    master_key_id text  not null check (btrim(master_key_id) <> '' and char_length(master_key_id) <= 64),
    last4         text  not null check (char_length(last4) = 4),
    set_by        uuid,                      -- the ClearCut key that set it (an id, never a secret)
    created_at    timestamptz not null default now(),
    updated_at    timestamptz not null default now(),
    checked_at    timestamptz,
    check_ok      boolean
);
alter table tenant_anthropic_keys enable row level security;

-- The application role has NO privilege on this table. Only the vault role reaches it, only for the
-- current tenant, and only through the functions below.
grant usage on schema public to clearcut_vault;
grant select, insert, update, delete on tenant_anthropic_keys to clearcut_vault;
create policy vault_all on tenant_anthropic_keys for all to clearcut_vault
    using (tenant_id = current_tenant()) with check (tenant_id = current_tenant());

grant create on schema public to clearcut_vault;

create function set_tenant_anthropic_key(
    p_tenant uuid, p_nonce bytea, p_ciphertext bytea, p_master_key_id text,
    p_last4 text, p_set_by uuid, p_check_ok boolean) returns void
    language plpgsql security definer set search_path = public, pg_temp
as $$
begin
    if p_tenant is null or p_tenant is distinct from current_tenant() then
        raise exception 'set_tenant_anthropic_key: tenant does not match the current tenant'
            using errcode = '42501';
    end if;
    insert into tenant_anthropic_keys
        (tenant_id, nonce, ciphertext, master_key_id, last4, set_by, checked_at, check_ok)
    values (p_tenant, p_nonce, p_ciphertext, p_master_key_id, p_last4, p_set_by, now(), p_check_ok)
    on conflict (tenant_id) do update set
        nonce = excluded.nonce, ciphertext = excluded.ciphertext,
        master_key_id = excluded.master_key_id, last4 = excluded.last4, set_by = excluded.set_by,
        checked_at = now(), check_ok = excluded.check_ok, updated_at = now();
end $$;

-- The only way to read the ciphertext (the runner uses it, with the master key, per job).
create function load_tenant_anthropic_key(p_tenant uuid)
    returns table (out_nonce bytea, out_ciphertext bytea, out_master_key_id text)
    language plpgsql security definer set search_path = public, pg_temp
as $$
begin
    if p_tenant is null or p_tenant is distinct from current_tenant() then
        raise exception 'load_tenant_anthropic_key: tenant does not match the current tenant'
            using errcode = '42501';
    end if;
    return query select nonce, ciphertext, master_key_id from tenant_anthropic_keys
        where tenant_id = p_tenant and check_ok is not false;
end $$;

-- What the API may show: never the ciphertext.
create function tenant_anthropic_key_status(p_tenant uuid)
    returns table (out_last4 text, out_set_by uuid, out_updated_at timestamptz,
                   out_checked_at timestamptz, out_check_ok boolean)
    language plpgsql security definer set search_path = public, pg_temp
as $$
begin
    if p_tenant is null or p_tenant is distinct from current_tenant() then
        raise exception 'tenant_anthropic_key_status: tenant does not match the current tenant'
            using errcode = '42501';
    end if;
    return query select last4, set_by, updated_at, checked_at, check_ok
        from tenant_anthropic_keys where tenant_id = p_tenant;
end $$;

-- After Anthropic rejects the key at run time: say so, so the runner stops using it.
create function mark_tenant_anthropic_key_rejected(p_tenant uuid) returns void
    language plpgsql security definer set search_path = public, pg_temp
as $$
begin
    if p_tenant is null or p_tenant is distinct from current_tenant() then
        raise exception 'mark_tenant_anthropic_key_rejected: tenant does not match the current tenant'
            using errcode = '42501';
    end if;
    update tenant_anthropic_keys set check_ok = false, checked_at = now() where tenant_id = p_tenant;
end $$;

create function delete_tenant_anthropic_key(p_tenant uuid) returns boolean
    language plpgsql security definer set search_path = public, pg_temp
as $$
declare removed integer;
begin
    if p_tenant is null or p_tenant is distinct from current_tenant() then
        raise exception 'delete_tenant_anthropic_key: tenant does not match the current tenant'
            using errcode = '42501';
    end if;
    delete from tenant_anthropic_keys where tenant_id = p_tenant;
    get diagnostics removed = row_count;
    return removed > 0;
end $$;

alter function set_tenant_anthropic_key(uuid, bytea, bytea, text, text, uuid, boolean) owner to clearcut_vault;
alter function load_tenant_anthropic_key(uuid) owner to clearcut_vault;
alter function tenant_anthropic_key_status(uuid) owner to clearcut_vault;
alter function mark_tenant_anthropic_key_rejected(uuid) owner to clearcut_vault;
alter function delete_tenant_anthropic_key(uuid) owner to clearcut_vault;
revoke create on schema public from clearcut_vault;

revoke all on function set_tenant_anthropic_key(uuid, bytea, bytea, text, text, uuid, boolean) from public;
revoke all on function load_tenant_anthropic_key(uuid) from public;
revoke all on function tenant_anthropic_key_status(uuid) from public;
revoke all on function mark_tenant_anthropic_key_rejected(uuid) from public;
revoke all on function delete_tenant_anthropic_key(uuid) from public;
grant execute on function set_tenant_anthropic_key(uuid, bytea, bytea, text, text, uuid, boolean) to clearcut_app;
grant execute on function load_tenant_anthropic_key(uuid) to clearcut_app;
grant execute on function tenant_anthropic_key_status(uuid) to clearcut_app;
grant execute on function mark_tenant_anthropic_key_rejected(uuid) to clearcut_app;
grant execute on function delete_tenant_anthropic_key(uuid) to clearcut_app;

-- ----------------------------------------------------------------- erasure ----
-- The same function as in 0007 with two changes, closing a race found in review (#82): only jobs
-- that have ended are removed, so a document that still has a queued or running job (or one
-- queued in the same instant) fails the foreign key when its row is deleted, the whole erasure
-- rolls back, and the caller is told the document is busy (SQLSTATE 55006). It keeps its owner,
-- search_path and grants (create or replace changes none of them).
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
    delete from jobs          where document_id = p_document and tenant_id = p_tenant
                                and state in ('succeeded', 'needs_review', 'cancelled');
    delete from events        where document_id = p_document and tenant_id = p_tenant;
    begin
        delete from documents where id = p_document and tenant_id = p_tenant;
    exception when foreign_key_violation then
        raise exception 'erase_document: a job is queued or running' using errcode = '55006';
    end;
    insert into audit_log (tenant_id, actor, action, subject_type, subject_id, detail)
    values (p_tenant, '{"type":"system"}', 'document.erased', 'document', p_document::text, '{}');
end $$;
