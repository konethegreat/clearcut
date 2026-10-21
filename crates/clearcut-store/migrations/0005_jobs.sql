-- Written by Kone Tshivhinda & Claude
-- ADR-0008: the durable job queue.
--
-- A job is one stage of work on one document. All changes to a job after it
-- is queued go through the SECURITY DEFINER functions at the end of this file;
-- the application role can read jobs and add them, nothing more.
--
-- Run this as the owner role, never as the application role.

-- ---------------------------------------------------------------- role ----
-- A fourth login-less role, like the eraser and the authenticator: it exists
-- only to own the queue functions. Created the same race-safe way as in 0003
-- (the lock is its own statement so what follows sees a fresh snapshot).
select pg_advisory_xact_lock(hashtext('clearcut_roles'));

do $$
begin
    begin
        create role clearcut_queue nologin nosuperuser nocreatedb nocreaterole nobypassrls;
    exception when duplicate_object or unique_violation then
        null;
    end;
    if exists (select from pg_roles where rolname = 'clearcut_queue'
               and not (not rolcanlogin and not rolsuper and not rolcreatedb
                        and not rolcreaterole and not rolbypassrls)) then
        alter role clearcut_queue nologin nosuperuser nocreatedb nocreaterole nobypassrls;
    end if;
end $$;

grant clearcut_queue to current_user;

-- ----------------------------------------------------------------- table ----
create table jobs (
    id               uuid primary key default gen_random_uuid(),
    tenant_id        uuid not null,
    document_id      uuid not null,
    -- Validated by clearcut-core's JobKind, so a new kind needs no migration.
    kind             text not null check (btrim(kind) <> ''),
    -- Parameters only, never invoice content.
    payload          jsonb not null default '{}'
                     check (jsonb_typeof(payload) = 'object' and pg_column_size(payload) <= 16384),
    state            text not null default 'queued'
                     check (state in ('queued', 'running', 'succeeded', 'needs_review', 'cancelled')),
    priority         smallint not null default 0,                 -- higher runs first
    attempts         integer not null default 0 check (attempts >= 0),
    max_attempts     integer not null default 5 check (max_attempts between 1 and 20),
    run_after        timestamptz not null default now(),          -- retries and deferrals push this forward
    deadline_at      timestamptz not null default now() + interval '48 hours',

    -- The lease: present exactly while the job is running.
    lease_token      uuid,                                        -- the fencing token; the app role cannot read it
    leased_by        text check (btrim(leased_by) <> ''),
    lease_expires_at timestamptz,

    -- Progress and resumability. No invoice content in any of these.
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
    check (progress_total is null or progress_done <= progress_total),
    -- The lease columns and the state must agree: all set while running, all clear otherwise.
    check (
        (state = 'running' and lease_token is not null and leased_by is not null and lease_expires_at is not null)
        or (state <> 'running' and lease_token is null and leased_by is null and lease_expires_at is null)
    ),
    -- Finished exactly when terminal.
    check ((state in ('succeeded', 'needs_review', 'cancelled')) = (finished_at is not null))
);

-- At most one active job of a kind per document: enqueuing twice is harmless.
create unique index jobs_one_active on jobs (tenant_id, document_id, kind) where state in ('queued', 'running');
-- What a runner scans. Small, because finished jobs are not in it.
create index jobs_claimable on jobs (priority desc, run_after, created_at) where state in ('queued', 'running');
create index jobs_running_by_tenant on jobs (tenant_id) where state = 'running';
create index jobs_by_document on jobs (tenant_id, document_id, created_at);

alter table jobs enable row level security;

-- ------------------------------------------------------------ privileges ----
-- The application role may read everything about a job except its lease token,
-- and add a job by naming only what a caller decides. State, attempts, the
-- lease and the timestamps get their defaults and can be changed only through
-- the functions below. There is no UPDATE and no DELETE.
grant select (id, tenant_id, document_id, kind, payload, state, priority, attempts, max_attempts,
              run_after, deadline_at, leased_by, lease_expires_at, progress_done, progress_total,
              progress_note, checkpoint, last_error, cancel_requested_at, created_at, updated_at,
              started_at, finished_at)
    on jobs to clearcut_app;
grant insert (tenant_id, document_id, kind, payload, priority, max_attempts, run_after, deadline_at)
    on jobs to clearcut_app;

create policy app_read   on jobs for select to clearcut_app using (tenant_id = current_tenant());
create policy app_insert on jobs for insert to clearcut_app with check (tenant_id = current_tenant());

-- The queue role acts across tenants (a runner claims whatever is due), so its
-- policies match every row. That makes each function below responsible for
-- scoping itself: the tenant-scoped ones check p_tenant against the current
-- tenant and filter on it, exactly as erase_document does.
grant usage on schema public to clearcut_queue;
grant select, update on jobs to clearcut_queue;
grant select, update (status, updated_at) on documents to clearcut_queue;
grant insert on audit_log to clearcut_queue;
grant usage on all sequences in schema public to clearcut_queue;

create policy queue_read   on jobs      for select to clearcut_queue using (true);
create policy queue_write  on jobs      for update to clearcut_queue using (true) with check (true);
create policy queue_read   on documents for select to clearcut_queue using (true);
create policy queue_write  on documents for update to clearcut_queue using (true) with check (true);
create policy queue_insert on audit_log for insert to clearcut_queue with check (true);

-- Erasure must take a document's jobs with it (the foreign key would otherwise
-- stop it). The eraser gets only what that needs.
grant select, delete on jobs to clearcut_eraser;
create policy eraser_read   on jobs for select to clearcut_eraser using (true);
create policy eraser_delete on jobs for delete to clearcut_eraser using (true);

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
    delete from jobs      where document_id = p_document and tenant_id = p_tenant;
    delete from events    where document_id = p_document and tenant_id = p_tenant;
    delete from documents where id = p_document and tenant_id = p_tenant;
    insert into audit_log (tenant_id, actor, action, subject_type, subject_id, detail)
    values (p_tenant, '{"type":"system"}', 'document.erased', 'document', p_document::text, '{}');
end $$;

-- ------------------------------------------------------------- functions ----
-- Error codes the Rust side recognises:
--   CL001  the caller no longer holds the lease (wrong token, or the job is not running)
--   CL002  no such job for this tenant
-- A function that is a helper only (finish_job_internal, lock_leased_job) is
-- owned by the queue role and not granted to anyone else.

-- Moves a job to a terminal state, moves its document with it, and audits it,
-- all in the caller's transaction. A job that never started and is cancelled
-- leaves its document alone (it may be finished from an earlier run).
create function finish_job_internal(p_job uuid, p_state text, p_error text, p_actor jsonb)
    returns void
    language plpgsql
    security definer
    set search_path = public, pg_temp
as $$
declare
    j jobs%rowtype;
begin
    select * into j from jobs where id = p_job for update;
    update jobs
        set state = p_state, finished_at = now(),
            lease_token = null, leased_by = null, lease_expires_at = null,
            last_error = coalesce(p_error, last_error), updated_at = now()
        where id = p_job;
    if not (p_state = 'cancelled' and j.state = 'queued') then
        update documents
            set status = case p_state
                             when 'succeeded' then 'done'
                             when 'needs_review' then 'needs_review'
                             else 'received'
                         end,
                updated_at = now()
            where id = j.document_id and tenant_id = j.tenant_id;
    end if;
    insert into audit_log (tenant_id, actor, action, subject_type, subject_id, detail)
    values (j.tenant_id, p_actor, 'job.' || p_state, 'job', p_job::text,
            jsonb_strip_nulls(jsonb_build_object(
                'document_id', j.document_id, 'kind', j.kind,
                'attempts', j.attempts, 'worker', j.leased_by)));
end $$;

-- Finds a job the caller holds the lease on, and locks it. Checks the tenant,
-- the fencing token and that the job is still running. A lease that has run
-- out but has not been taken over still counts: the token is the proof, and
-- nobody else holds the job.
create function lock_leased_job(p_tenant uuid, p_job uuid, p_token uuid)
    returns jobs
    language plpgsql
    security definer
    set search_path = public, pg_temp
as $$
declare
    j jobs%rowtype;
begin
    if p_tenant is null or p_tenant is distinct from current_tenant() then
        raise exception 'job: tenant does not match the current tenant' using errcode = '42501';
    end if;
    select * into j from jobs where id = p_job and tenant_id = p_tenant for update;
    if not found then
        raise exception 'job not found' using errcode = 'CL002';
    end if;
    if j.state <> 'running' or j.lease_token is distinct from p_token then
        raise exception 'the lease on this job is no longer held' using errcode = 'CL001';
    end if;
    return j;
end $$;

-- Takes the best job that is due (or whose lease ran out) for a runner, across
-- all tenants. Sweeps as it goes: a candidate that is past its deadline, out of
-- attempts after a lost lease, or has had cancellation requested is finished
-- (document and audit included) instead of run, and the scan moves on.
-- p_per_tenant_cap, when given, skips tenants that already have that many jobs
-- running; sweeps ignore the cap. The cap is soft: two simultaneous claims
-- can pass it by one.
create function claim_job(p_worker text, p_lease_seconds integer, p_kinds text[], p_per_tenant_cap integer)
    returns setof jobs
    language plpgsql
    security definer
    set search_path = public, pg_temp
as $$
declare
    j jobs%rowtype;
begin
    if p_worker is null or btrim(p_worker) = '' then
        raise exception 'claim_job: a worker name is required' using errcode = '22023';
    end if;
    if p_lease_seconds is null or p_lease_seconds < 3 then
        raise exception 'claim_job: a lease must be at least 3 seconds' using errcode = '22023';
    end if;
    loop
        select * into j from jobs
            where kind = any (p_kinds)
              and ((state = 'queued' and run_after <= now())
                   or (state = 'running' and lease_expires_at < now()))
              and (p_per_tenant_cap is null
                   or deadline_at <= now()
                   or cancel_requested_at is not null
                   or (state = 'running' and attempts >= max_attempts)
                   or (select count(*) from jobs r
                        where r.tenant_id = jobs.tenant_id and r.state = 'running'
                          and r.lease_expires_at >= now()) < p_per_tenant_cap)
            order by priority desc, run_after, created_at
            limit 1
            for update skip locked;
        if not found then
            return;
        end if;

        if j.cancel_requested_at is not null then
            perform finish_job_internal(j.id, 'cancelled', null, '{"type":"system"}');
        elsif j.deadline_at <= now() then
            perform finish_job_internal(j.id, 'needs_review', 'deadline passed before the job could finish', '{"type":"system"}');
        elsif j.state = 'running' and j.attempts >= j.max_attempts then
            perform finish_job_internal(j.id, 'needs_review', 'worker lost on the final attempt', '{"type":"system"}');
        else
            update jobs
                set state = 'running', attempts = attempts + 1,
                    lease_token = gen_random_uuid(), leased_by = p_worker,
                    lease_expires_at = now() + make_interval(secs => p_lease_seconds),
                    started_at = coalesce(started_at, now()), updated_at = now()
                where id = j.id
                returning * into j;
            update documents set status = 'processing', updated_at = now()
                where id = j.document_id and tenant_id = j.tenant_id;
            return next j;
            return;
        end if;
    end loop;
end $$;

-- A sign of life from the worker: extends the lease, stores progress and an
-- optional note and checkpoint (null keeps what is stored), and says whether
-- cancellation was requested.
create function heartbeat_job(p_tenant uuid, p_job uuid, p_token uuid, p_lease_seconds integer,
                              p_done integer, p_total integer, p_note text, p_checkpoint jsonb)
    returns boolean
    language plpgsql
    security definer
    set search_path = public, pg_temp
as $$
declare
    j jobs%rowtype;
begin
    if p_lease_seconds is null or p_lease_seconds < 3 then
        raise exception 'heartbeat_job: a lease must be at least 3 seconds' using errcode = '22023';
    end if;
    select * into j from lock_leased_job(p_tenant, p_job, p_token);
    update jobs
        set lease_expires_at = now() + make_interval(secs => p_lease_seconds),
            progress_done = p_done, progress_total = p_total,
            progress_note = coalesce(p_note, progress_note),
            checkpoint = coalesce(p_checkpoint, checkpoint),
            updated_at = now()
        where id = p_job;
    return j.cancel_requested_at is not null;
end $$;

-- The worker finishes the job: succeeded, needs_review, or cancelled
-- (acknowledging a cancellation request).
create function complete_job(p_tenant uuid, p_job uuid, p_token uuid, p_outcome text, p_reason text)
    returns void
    language plpgsql
    security definer
    set search_path = public, pg_temp
as $$
begin
    if p_outcome is null or p_outcome not in ('succeeded', 'needs_review', 'cancelled') then
        raise exception 'complete_job: unknown outcome' using errcode = '22023';
    end if;
    perform lock_leased_job(p_tenant, p_job, p_token);
    perform finish_job_internal(p_job, p_outcome, p_reason, '{"type":"system"}');
end $$;

-- A failed run. p_counts says whether it stays counted against the attempts
-- (a provider outage does not). Returns the state the job ends up in, which can
-- differ from what the caller hoped for: out of attempts -> needs_review,
-- cancellation requested -> cancelled, past the deadline -> needs_review.
-- Otherwise the job goes back to queued, to run again after the delay.
create function fail_job(p_tenant uuid, p_job uuid, p_token uuid, p_retry_after_seconds integer,
                         p_counts boolean, p_error text)
    returns text
    language plpgsql
    security definer
    set search_path = public, pg_temp
as $$
declare
    j jobs%rowtype;
begin
    if p_retry_after_seconds is null or p_retry_after_seconds < 0 or p_retry_after_seconds > 86400 then
        raise exception 'fail_job: the retry delay must be 0 to 86400 seconds' using errcode = '22023';
    end if;
    select * into j from lock_leased_job(p_tenant, p_job, p_token);
    if p_counts and j.attempts >= j.max_attempts then
        perform finish_job_internal(p_job, 'needs_review', p_error, '{"type":"system"}');
        return 'needs_review';
    elsif j.cancel_requested_at is not null then
        perform finish_job_internal(p_job, 'cancelled', p_error, '{"type":"system"}');
        return 'cancelled';
    elsif j.deadline_at <= now() then
        perform finish_job_internal(p_job, 'needs_review', p_error, '{"type":"system"}');
        return 'needs_review';
    end if;
    update jobs
        set state = 'queued',
            run_after = now() + make_interval(secs => p_retry_after_seconds),
            attempts = attempts - case when p_counts then 0 else 1 end,
            last_error = coalesce(p_error, last_error),
            lease_token = null, leased_by = null, lease_expires_at = null,
            updated_at = now()
        where id = p_job;
    return 'queued';
end $$;

-- Cancels a job. Queued: cancelled at once. Running: cancellation is requested
-- and the worker stops at its next heartbeat. Already finished: nothing
-- changes. Returns the state afterwards.
create function cancel_job(p_tenant uuid, p_job uuid, p_actor jsonb)
    returns text
    language plpgsql
    security definer
    set search_path = public, pg_temp
as $$
declare
    j jobs%rowtype;
begin
    if p_tenant is null or p_tenant is distinct from current_tenant() then
        raise exception 'cancel_job: tenant does not match the current tenant' using errcode = '42501';
    end if;
    select * into j from jobs where id = p_job and tenant_id = p_tenant for update;
    if not found then
        raise exception 'job not found' using errcode = 'CL002';
    end if;
    if j.state = 'queued' then
        perform finish_job_internal(p_job, 'cancelled', null, p_actor);
        return 'cancelled';
    elsif j.state = 'running' then
        if j.cancel_requested_at is null then
            update jobs set cancel_requested_at = now(), updated_at = now() where id = p_job;
            insert into audit_log (tenant_id, actor, action, subject_type, subject_id, detail)
            values (p_tenant, p_actor, 'job.cancel_requested', 'job', p_job::text,
                    jsonb_build_object('document_id', j.document_id, 'kind', j.kind));
        end if;
        return 'running';
    end if;
    return j.state;
end $$;

-- Ownership and access. A function's new owner needs CREATE on the schema, for
-- this step only. Only the five public functions are executable by the
-- application role; the two helpers are callable only from inside the others.
grant create on schema public to clearcut_queue;
alter function finish_job_internal(uuid, text, text, jsonb) owner to clearcut_queue;
alter function lock_leased_job(uuid, uuid, uuid) owner to clearcut_queue;
alter function claim_job(text, integer, text[], integer) owner to clearcut_queue;
alter function heartbeat_job(uuid, uuid, uuid, integer, integer, integer, text, jsonb) owner to clearcut_queue;
alter function complete_job(uuid, uuid, uuid, text, text) owner to clearcut_queue;
alter function fail_job(uuid, uuid, uuid, integer, boolean, text) owner to clearcut_queue;
alter function cancel_job(uuid, uuid, jsonb) owner to clearcut_queue;
revoke create on schema public from clearcut_queue;

revoke all on function finish_job_internal(uuid, text, text, jsonb) from public;
revoke all on function lock_leased_job(uuid, uuid, uuid) from public;
revoke all on function claim_job(text, integer, text[], integer) from public;
revoke all on function heartbeat_job(uuid, uuid, uuid, integer, integer, integer, text, jsonb) from public;
revoke all on function complete_job(uuid, uuid, uuid, text, text) from public;
revoke all on function fail_job(uuid, uuid, uuid, integer, boolean, text) from public;
revoke all on function cancel_job(uuid, uuid, jsonb) from public;

grant execute on function claim_job(text, integer, text[], integer) to clearcut_app;
grant execute on function heartbeat_job(uuid, uuid, uuid, integer, integer, integer, text, jsonb) to clearcut_app;
grant execute on function complete_job(uuid, uuid, uuid, text, text) to clearcut_app;
grant execute on function fail_job(uuid, uuid, uuid, integer, boolean, text) to clearcut_app;
grant execute on function cancel_job(uuid, uuid, jsonb) to clearcut_app;
