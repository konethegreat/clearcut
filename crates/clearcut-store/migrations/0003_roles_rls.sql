-- Written by Kone Tshivhinda & Claude
-- ADR-0007: database roles, row-level security, and erase_document.
--
-- Run this as the owner role (the one that created the tables), never as the
-- application role. The service connects as `clearcut_app`.

-- ---------------------------------------------------------------- roles ----
-- Roles are cluster-wide, so they are created idempotently. Several databases
-- (for example the throwaway test databases) can run this at the same moment;
-- the advisory lock makes them take turns. The roles have NO password: a role
-- without one cannot log in by password, so operations sets credentials out of
-- band (secret manager) when it deploys.
-- Take the lock as a statement of its own: statements after it get a fresh
-- snapshot, so they see what other migrations committed while this one waited.
-- (Inside a single DO block the snapshot would predate the wait, and the block
-- would act on a stale view of the roles.)
select pg_advisory_xact_lock(hashtext('clearcut_roles'));

do $$
begin
    -- Try to create, and treat "already there" as success.
    begin
        create role clearcut_app login nosuperuser nocreatedb nocreaterole nobypassrls;
    exception when duplicate_object or unique_violation then
        null;
    end;
    begin
        -- Never logs in; it exists only to own erase_document.
        create role clearcut_eraser nologin nosuperuser nocreatedb nocreaterole nobypassrls;
    exception when duplicate_object or unique_violation then
        null;
    end;
    -- Roles are shared by every database in the cluster, so one may already
    -- exist with other attributes. State the security-critical ones explicitly
    -- instead of trusting what was there, but only touch a role that differs.
    if exists (select from pg_roles where rolname = 'clearcut_app'
               and not (rolcanlogin and not rolsuper and not rolcreatedb
                        and not rolcreaterole and not rolbypassrls)) then
        alter role clearcut_app login nosuperuser nocreatedb nocreaterole nobypassrls;
    end if;
    if exists (select from pg_roles where rolname = 'clearcut_eraser'
               and not (not rolcanlogin and not rolsuper and not rolcreatedb
                        and not rolcreaterole and not rolbypassrls)) then
        alter role clearcut_eraser nologin nosuperuser nocreatedb nocreaterole nobypassrls;
    end if;
end $$;

-- The migrating role must be able to hand ownership of the function to the
-- eraser role (a no-op for a superuser).
grant clearcut_eraser to current_user;

-- ------------------------------------------------------------ privileges ----
-- Deliberately narrow. The application can read and add; it cannot change or
-- remove events or audit rows, cannot delete documents, cannot create
-- tenants (that is the owner's `clearcut-admin` command), and has no TRUNCATE.
-- New tables get no privileges until a migration grants them.
grant usage on schema public to clearcut_app, clearcut_eraser;

grant select on tenants to clearcut_app;
grant select, insert on documents to clearcut_app;
grant update (status, updated_at, page_count) on documents to clearcut_app;
grant select, insert on events to clearcut_app;
grant select, insert on audit_log to clearcut_app;
grant usage on all sequences in schema public to clearcut_app, clearcut_eraser;

-- The eraser can only do what erase_document needs.
grant select, delete on documents to clearcut_eraser;
grant select, delete on events to clearcut_eraser;
grant insert on audit_log to clearcut_eraser;

-- ------------------------------------------------- row-level security ----
-- Which tenant is acting is a setting every service transaction makes with
-- `set_config('clearcut.tenant', <uuid>, true)`. If it is missing the policies
-- compare against NULL and match nothing: they fail closed.
create function current_tenant() returns uuid
    language sql stable
    as $$ select nullif(current_setting('clearcut.tenant', true), '')::uuid $$;

-- Not FORCEd: the owner role (migrations, the operator's admin command) is
-- deliberately not restricted; the service never runs as the owner.
alter table tenants   enable row level security;
alter table documents enable row level security;
alter table events    enable row level security;
alter table audit_log enable row level security;

create policy app_read   on tenants   for select to clearcut_app using (id = current_tenant());

create policy app_read   on documents for select to clearcut_app using (tenant_id = current_tenant());
create policy app_insert on documents for insert to clearcut_app with check (tenant_id = current_tenant());
create policy app_update on documents for update to clearcut_app
    using (tenant_id = current_tenant()) with check (tenant_id = current_tenant());

create policy app_read   on events    for select to clearcut_app using (tenant_id = current_tenant());
create policy app_insert on events    for insert to clearcut_app with check (tenant_id = current_tenant());

create policy app_read   on audit_log for select to clearcut_app using (tenant_id = current_tenant());
create policy app_insert on audit_log for insert to clearcut_app with check (tenant_id = current_tenant());

-- The eraser acts across rows, but only inside erase_document, which checks the
-- tenant itself.
create policy eraser_read   on documents for select to clearcut_eraser using (true);
create policy eraser_delete on documents for delete to clearcut_eraser using (true);
create policy eraser_read   on events    for select to clearcut_eraser using (true);
create policy eraser_delete on events    for delete to clearcut_eraser using (true);
create policy eraser_insert on audit_log for insert to clearcut_eraser with check (true);

-- ------------------------------------------------------------- erasure ----
-- Events may be deleted only by the eraser role, i.e. only from inside
-- erase_document. This replaces the `clearcut.erasure` session setting, which
-- any session could set. (The audit log still has no erasure path.)
create or replace function forbid_event_changes() returns trigger language plpgsql as $$
begin
    if tg_op = 'DELETE' and current_user = 'clearcut_eraser' then
        return old;
    end if;
    raise exception '% is append-only (% refused)', tg_table_name, tg_op;
end $$;

-- A function's new owner needs CREATE on the schema; give it only for this step.
grant create on schema public to clearcut_eraser;

-- POPIA erasure of one document: its events and its row, plus one audit entry
-- that names the document but holds no invoice content. SECURITY DEFINER, so it
-- runs with the eraser's powers; that is why it checks the caller's tenant
-- itself and pins its search_path.
create function erase_document(p_tenant uuid, p_document uuid) returns void
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
    delete from events    where document_id = p_document and tenant_id = p_tenant;
    delete from documents where id = p_document and tenant_id = p_tenant;
    insert into audit_log (tenant_id, actor, action, subject_type, subject_id, detail)
    values (p_tenant, '{"type":"system"}', 'document.erased', 'document', p_document::text, '{}');
end $$;

alter function erase_document(uuid, uuid) owner to clearcut_eraser;
revoke create on schema public from clearcut_eraser;
revoke all on function erase_document(uuid, uuid) from public;
grant execute on function erase_document(uuid, uuid) to clearcut_app;
