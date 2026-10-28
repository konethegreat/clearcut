-- Written by Kone Tshivhinda & Claude
-- ADR-0021: automatic retention.
--
-- A document is erased automatically once it is older than a number of days (the server's
-- CLEARCUT_RETENTION_DAYS). The erasure itself reuses erase_document, so a document's events,
-- readings, jobs, row and file go exactly as they do for a deletion a person asks for, and the
-- call log and audit log stay.
--
-- The application role reads one tenant at a time (RLS), but the sweep must find aged documents
-- of every account. This adds a login-less role like the queue's, owning one function that
-- returns candidates; the function returns no content, only two ids, and only for documents past
-- the threshold that no running job is using.
--
-- Run this as the owner role, never as the application role.

-- ---------------------------------------------------------------- role ----
-- Created the race-safe way as in 0003 and 0005 (the lock is its own statement so what follows
-- sees a fresh snapshot).
select pg_advisory_xact_lock(hashtext('clearcut_roles'));

do $$
begin
    begin
        create role clearcut_retention nologin nosuperuser nocreatedb nocreaterole nobypassrls;
    exception when duplicate_object or unique_violation then
        null;
    end;
    if exists (select from pg_roles where rolname = 'clearcut_retention'
               and not (not rolcanlogin and not rolsuper and not rolcreatedb
                        and not rolcreaterole and not rolbypassrls)) then
        alter role clearcut_retention nologin nosuperuser nocreatedb nocreaterole nobypassrls;
    end if;
end $$;

grant clearcut_retention to current_user;

-- --------------------------------------------------------------- grants ----
-- What the candidate query reads, and nothing more: the document's age and its jobs' state.
-- Column grants keep the job lease token out of the retention role's reach.
grant usage on schema public to clearcut_retention;
grant select (tenant_id, id, created_at) on documents to clearcut_retention;
grant select (tenant_id, document_id, state) on jobs to clearcut_retention;

create policy retention_read on documents for select to clearcut_retention using (true);
create policy retention_read on jobs      for select to clearcut_retention using (true);

-- The sweep walks documents oldest first; without this it would scan every document each time.
create index documents_by_age on documents (created_at);

-- ------------------------------------------------------------- function ----
-- Documents older than p_days that no **running** job is using, oldest first, at most p_limit.
-- A queued job is not a reason to keep a document past its period: nothing has started, and the
-- erasure cancels it first (a deployment with no worker leaves jobs queued forever, and those
-- documents must still expire). A running job is work in progress; erasure refuses it, so it is
-- skipped here and the document is tried again on the next sweep.
create function expired_documents(p_days integer, p_limit integer)
    returns table (tenant_id uuid, document_id uuid)
    language plpgsql
    security definer
    set search_path = public, pg_temp
as $$
begin
    if p_days is null or p_days < 1 then
        raise exception 'expired_documents: retention must be at least one day' using errcode = '22023';
    end if;
    if p_limit is null or p_limit < 1 or p_limit > 1000 then
        raise exception 'expired_documents: the batch must be 1 to 1000' using errcode = '22023';
    end if;
    return query
        select d.tenant_id, d.id
        from documents d
        where d.created_at < now() - make_interval(days => p_days)
          and not exists (
              select 1 from jobs j
              where j.tenant_id = d.tenant_id
                and j.document_id = d.id
                and j.state = 'running'
          )
        order by d.created_at
        limit p_limit;
end $$;

-- Ownership and access, as in 0005. The owner needs CREATE on the schema for this step only.
grant create on schema public to clearcut_retention;
alter function expired_documents(integer, integer) owner to clearcut_retention;
revoke create on schema public from clearcut_retention;

revoke all on function expired_documents(integer, integer) from public;
grant execute on function expired_documents(integer, integer) to clearcut_app;
