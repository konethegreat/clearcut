-- Written by Kone Tshivhinda & Claude
-- ADR-0007: API keys and the function that authenticates them.

-- A third database role, login-less like the eraser: it exists only to own
-- authenticate_api_key. Created the same race-safe way as in 0003 (the lock is
-- its own statement so what follows sees a fresh snapshot).
select pg_advisory_xact_lock(hashtext('clearcut_roles'));

do $$
begin
    begin
        create role clearcut_authenticator nologin nosuperuser nocreatedb nocreaterole nobypassrls;
    exception when duplicate_object or unique_violation then
        null;
    end;
    if exists (select from pg_roles where rolname = 'clearcut_authenticator'
               and not (not rolcanlogin and not rolsuper and not rolcreatedb
                        and not rolcreaterole and not rolbypassrls)) then
        alter role clearcut_authenticator nologin nosuperuser nocreatedb nocreaterole nobypassrls;
    end if;
end $$;

grant clearcut_authenticator to current_user;

-- ------------------------------------------------------------------ table ----
-- The key id is part of the key's text, so the application supplies it.
-- expires_at is NOT NULL: there is no such thing as a key that never expires.
create table api_keys (
    id           uuid primary key,
    tenant_id    uuid not null references tenants (id),
    name         text not null check (btrim(name) <> ''),
    secret_hash  bytea not null check (octet_length(secret_hash) = 32),  -- SHA-256 of the secret; the secret is never stored
    scopes       text[] not null check (cardinality(scopes) > 0),
    created_at   timestamptz not null default now(),
    expires_at   timestamptz not null,
    last_used_at timestamptz,
    revoked_at   timestamptz,
    check (expires_at > created_at),
    unique (tenant_id, id)
);
create index api_keys_by_tenant on api_keys (tenant_id, created_at, id);

alter table api_keys enable row level security;

-- ------------------------------------------------------------- privileges ----
-- The application role manages keys but can never read a secret hash (not even
-- its own tenant's), change a key's scopes, expiry or name, or delete a key.
-- All it can do to an existing key is revoke it.
grant select (id, tenant_id, name, scopes, created_at, expires_at, last_used_at, revoked_at)
    on api_keys to clearcut_app;
grant insert on api_keys to clearcut_app;
grant update (revoked_at) on api_keys to clearcut_app;

create policy app_read   on api_keys for select to clearcut_app using (tenant_id = current_tenant());
create policy app_insert on api_keys for insert to clearcut_app with check (tenant_id = current_tenant());
create policy app_revoke on api_keys for update to clearcut_app
    using (tenant_id = current_tenant()) with check (tenant_id = current_tenant());

-- The authenticator is the only role that reads the hashes, and it may update
-- nothing but last_used_at. It acts across tenants because it cannot know the
-- tenant before it has checked the key.
grant usage on schema public to clearcut_authenticator;
grant select on api_keys to clearcut_authenticator;
grant update (last_used_at) on api_keys to clearcut_authenticator;
create policy auth_read  on api_keys for select to clearcut_authenticator using (true);
create policy auth_touch on api_keys for update to clearcut_authenticator using (true) with check (true);

-- --------------------------------------------------------- authentication ----
-- Returns the tenant and scopes if, and only if, the key exists, the secret
-- matches, the key is not revoked and has not expired; otherwise no rows, with
-- no hint as to which of those failed.
--
-- The hashes are compared as hashes of hashes, so how long the comparison takes
-- cannot reveal how many leading bytes of a guess were right. An unknown key id
-- does equivalent work. last_used_at is refreshed at most once a minute, to
-- avoid a write on every request.
create function authenticate_api_key(p_key_id uuid, p_secret_hash bytea)
    returns table (out_tenant_id uuid, out_scopes text[])
    language plpgsql
    security definer
    set search_path = public, pg_temp
as $$
declare
    k api_keys%rowtype;
begin
    select * into k from api_keys where id = p_key_id;
    if not found then
        perform sha256(p_secret_hash) = sha256(decode(repeat('00', 32), 'hex'));
        return;
    end if;
    if sha256(k.secret_hash) <> sha256(p_secret_hash)
       or k.revoked_at is not null
       or k.expires_at <= now() then
        return;
    end if;
    update api_keys set last_used_at = now()
        where id = k.id and (last_used_at is null or last_used_at < now() - interval '1 minute');
    return query select k.tenant_id, k.scopes;
end $$;

grant create on schema public to clearcut_authenticator;
alter function authenticate_api_key(uuid, bytea) owner to clearcut_authenticator;
revoke create on schema public from clearcut_authenticator;
revoke all on function authenticate_api_key(uuid, bytea) from public;
grant execute on function authenticate_api_key(uuid, bytea) to clearcut_app;
