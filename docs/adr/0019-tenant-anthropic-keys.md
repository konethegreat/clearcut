# ADR-0019: Each tenant's own Anthropic key (ADR-0011 mode B)

Status: **Proposed** (awaiting Kone Tshivhinda's approval of the design and the migration SQL; nothing is applied until then) · Date: 2026-10-26 · Deciders: Kone Tshivhinda, Claude
Builds on [ADR-0011](0011-bring-your-own-anthropic-key.md) (mode B was designed there and deferred "until the key store has had its own review": this is that review), [ADR-0007](0007-auth-and-row-level-security.md) (roles and row-level security), [ADR-0015](0015-try-it-deployment.md) (mode C, still deferred). Task: LAUNCH.md item 1, P2-23 mode B.

## Context
Today the server uses **one** Anthropic key from its environment for every tenant (mode A). That is right on your own computer and wrong for a service: other people's documents would be read on one person's key and money. Kone's requirement (26 Oct 2026): *when deployed, users input their own key.* TimeLex needs the same: set a tenant's key once through the API.

## Decision in brief
1. **Each tenant stores its own key**, **encrypted by the service before it reaches the database**, **write-only** over the API, used by the job runner for that tenant's documents only.
2. **An operator setting decides whether the server's own key may be used at all.** `CLEARCUT_KEY_MODE=shared` (the default; today's behaviour, a tenant key wins if set) or `tenant-only` (the deployment setting: the server key is never used; a tenant with no key has its documents wait). The project still has no key and never pools users' keys.
3. **The master key** that encrypts tenant keys comes from `CLEARCUT_MASTER_KEY` (32 random bytes, base64) or `CLEARCUT_MASTER_KEY_FILE`, never from the database, and is a secret type that cannot be printed. Without it, mode B is off: setting a key answers `503`.

## 1. Encryption
- **AES-256-GCM** (RustCrypto `aes-gcm`, MIT or Apache-2.0), a fresh random 96-bit nonce for every write.
- **The tenant id is bound in as associated data** (with a fixed label `clearcut/tenant-anthropic-key/v1`), so a ciphertext copied to another tenant's row **fails to decrypt**.
- The row records a `master_key_id` (default `k1`) so a master key can be rotated later: re-encrypt, or ask each tenant to enter the key again. Rotation tooling is not part of this step.
- The decrypted key exists only as the existing `ApiKey` secret type (unprintable, zeroed on drop) for the length of a job, and is never given to the worker, a log, an error, a URL, a job parameter or a report.

## 2. API (all need `keys:admin`; none is ever answered with the key)
| Call | Does |
|---|---|
| `PUT /v1/anthropic-key` with `{ "key": "sk-ant-..." }` | Refuses an admin key or a login token (the existing key-shape check). Runs the **free** key check (`GET /v1/models`, no tokens spent), refuses a key Anthropic rejects, otherwise encrypts and stores it and answers `{ last4, models, checked_at }`. Audited (who, when; never the value). |
| `GET /v1/anthropic-key` | `{ configured, last4, set_at, checked_at, source }` where `source` is `tenant`, `server` (the shared key would be used) or `none`. |
| `DELETE /v1/anthropic-key` | Removes it. Audited. |

The body is never logged (the server logs no bodies), the response never echoes it, and a test with a sentinel key asserts it appears in no response, header, log line, audit row or error.

## 3. What the runner does
- For each job it resolves the tenant's key: a tenant key if one is stored; else the server key **only** in `shared` mode; else **no key**.
- **No key, or a key Anthropic now rejects (401):** the job does not fail and burn attempts. It **waits** (the queue's "provider unavailable" kind, long delay) and the document shows *waiting for your Anthropic key*. Setting a key makes the tenant's waiting jobs due at once. A job still waiting at its 48-hour deadline ends `needs_review` as for any provider outage (ADR-0008).
- A rejected key is marked invalid in the row (so `GET` says so) and the user is told; ClearCut never retries a bad key.
- Budgets, escalation and rate-limit rules are unchanged and apply per tenant; they now protect that tenant's own money.

## 4. The web page
A plain **Settings** screen: the key's status (last four characters, checked or not, whether the server's key is in use), a password field, *Save* (which runs the check and says what the key can use), *Remove*, and the advice from ADR-0011 in plain words: use a **dedicated workspace key with a monthly spend limit and an expiry**, and disable it when you are done. The key is sent once in the body of the `PUT` and the field is cleared; the page never keeps it.

## 5. The database (the migration, `0010`)
Shown here in full for review. It follows the pattern of `0004` (a login-less role that owns security-definer functions; the application role gets **no privilege on the table**).

```sql
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
```

**What this protects, and what it does not.**
- A **stolen database or backup** holds ciphertext and no master key: the keys are not readable. A **stolen master key** without the database is useless. Both together expose the keys, so the operator keeps them apart (the master key in the host's secret store, the database elsewhere).
- A ciphertext **cannot be moved to another tenant** (the tenant id is bound in), and a tenant can reach **only its own row** (row-level security, and every function checks the tenant).
- The application role cannot read the table; it can only call the five functions. This is defence in depth against a **database-only** compromise. **It does not protect against a compromised service process**, which holds the master key and can decrypt any tenant's key as the runner does. That is inherent in a service that uses the key on the tenant's behalf, and it is why the key advice is *a dedicated key with a spend limit and an expiry*.

## 6. Tests (kept to the paths that matter)
Round trip; a ciphertext moved to another tenant fails; a wrong master key fails; the sentinel key is in no response, header, log, audit row or error; `PUT` refuses an admin key and a key Anthropic rejects; `GET` never returns the key; `DELETE` removes it and the runner then waits; `tenant-only` never uses the server key; a job waiting for a key runs once a key is set; the application role cannot select from the table (tried); the erase race (a queued job makes erasure fail and roll back, nothing deleted).

## 7. Build order (four small pull requests)
1. The migration, the crypto and the store functions, with their tests.
2. The endpoints and the key check, with the sentinel tests.
3. The runner: a tenant's key per job, the `tenant-only` mode, waiting for a key, making waiting jobs due.
4. The Settings screen, `docs/BYOK.md`, `docs/RUN-LOCALLY.md` and the compose file (the master key as a secret).

## Alternatives considered
- *Keys held in server memory only (mode C) for every tenant.* Every restart wipes every key and stalls every queued job, and a TimeLex-driven tenant has no browser to paste it into again. Fine for a trial, wrong for a service.
- *A per-request key header.* Jobs run after the request has ended and may resume days later, so the key would have to be stored anyway.
- *Storing the key in the clear and relying on the database's protections.* One SQL-injection or one backup leaks every user's money.
- *A hardware or cloud key-management service.* The right answer for a large operator, and the `master_key_id` leaves room for it; too heavy as the first step for a self-hosted open-source project.
- *Pgcrypto inside the database.* The key would reach the database in the clear, and the master key would sit in a SQL session.

## Questions for Kone
1. **Approve the design and the migration SQL as written?** (Any change to a column, a grant or a function is easy now and awkward later.)
2. **`CLEARCUT_KEY_MODE` default `shared` (today's behaviour, for your own computer) with `tenant-only` as the deployment setting:** agreed, or should `tenant-only` be the default and `shared` something you opt into?
3. **Who may set the key: `keys:admin` only,** agreed? (A tenant's ordinary keys with `documents:write` could upload and read but not change whose money is spent.)
