# ADR-0007: Authentication, authorization and tenant isolation (API keys, roles, row-level security)

Status: **Accepted** (approved by Kone Tshivhinda, 2026-10-21) · Date: 2026-10-21 · Deciders: Kone Tshivhinda, Claude
Implements task P1-04, to be built in small PRs (core `auth`; roles, RLS and tenant-scoped store; API keys; admin bootstrap command; server Bearer auth). The four open questions were decided on approval; see "Decisions".

## Context
ClearCut will hold other people's invoices (personal and business information, POPIA) for several customers in one database. Today anyone who can reach the store can ask for any tenant's rows; isolation depends on every query remembering `tenant_id`. We also have open items from earlier work: the upload endpoint must not exist without authentication (ADR-0006 follow-up, P1-02), and the `clearcut.erasure` switch can be set by any session that can run SQL ([ADR-0006](0006-postgres-schema.md) amendment).

The goal is that a mistake in application code **cannot** leak one tenant's data to another, and that dangerous powers (erasure) are held by a role the application cannot become.

## Proposal in brief
1. **API keys first** (OAuth 2.1 for the MCP server comes with P1-06; the scopes below are chosen to map onto OAuth scopes later).
2. **Keys are random, shown once, and stored only as a hash.** Format `ccut_<key id>_<secret>`; the database keeps a SHA-256 of the secret.
3. **Scopes, not roles-of-roles.** A key carries a small list of scopes; the permission model is pure code in `clearcut-core`.
4. **Row-level security (RLS) in Postgres** is the backstop: the application connects as a restricted role and every query runs inside a transaction that has said which tenant it is acting for. A query that forgets `tenant_id` returns nothing instead of everything.
5. **Three database roles** with separate powers: an owner that runs migrations, a restricted application role, and an erasure role that only a vetted function can use.
6. **Erasure becomes a function**, not a switch: the application role may *call* `erase_document(...)`, and cannot delete events itself.

## Details

### API keys
- Format: `ccut_<32 hex key id>_<64 hex secret>`. The secret is 256 bits from the operating system's random source, so a fast hash is the right tool (slow password hashes protect low-entropy passwords; they add nothing here). The `ccut_` prefix lets secret scanners recognise a leaked key.
- Stored: key id, tenant, a name, `SHA-256(secret)`, scopes, `created_at`, `last_used_at`, optional `expires_at`, `revoked_at`. The secret itself is never stored and cannot be shown again; losing it means issuing a new key. Rotation = create a new key, move clients, revoke the old.
- Checking a key: look up by key id, compare hashes **in constant time**, then check not revoked and not expired. Every failure (unknown id, wrong secret, revoked, expired) returns the **same** error with the same timing, so a caller learns nothing about which keys exist. An unknown id still performs a dummy comparison.
- Keys are only ever sent in an `Authorization: Bearer` header over TLS, and are never logged; request logging redacts the header.
- `last_used_at` is updated at most once a minute per key to avoid a write on every request.

### Scopes (initial list)
| Scope | Allows |
|---|---|
| `documents:read` | list and read documents and their status |
| `documents:write` | upload documents |
| `events:read` | read events, narration and exports |
| `review:write` | confirm or correct extractions (creates `Human` events) |
| `keys:admin` | create and revoke API keys for the tenant |

A request needs the scope its endpoint names; having `keys:admin` does not imply the others. Scopes live in `clearcut-core` (`Scope`, `Principal`) with tests, because they are a business rule; the REST and MCP layers only call `principal.require(Scope::DocumentsWrite)`.

### Row-level security
Every tenant-owned table (`documents`, `events`, `audit_log`, `api_keys`) gets:

```sql
alter table documents enable row level security;
alter table documents force  row level security;     -- applies to the owner too
create policy tenant_isolation on documents
    using      (tenant_id = nullif(current_setting('clearcut.tenant', true), '')::uuid)
    with check (tenant_id = nullif(current_setting('clearcut.tenant', true), '')::uuid);
```

The store opens every operation with `select set_config('clearcut.tenant', $1, true)` (the `true` makes it local to the transaction). If the setting is missing, the comparison is `NULL`, which matches no rows: **fail closed**. The existing composite foreign keys stay as a second layer.

- `events` and `audit_log` get separate `select` and `insert` policies and **no** `update` or `delete` policy for the application role, on top of the existing append-only triggers.
- `tenants`: a tenant can select its own row only.
- Authentication has to find a key *before* it knows the tenant, so it goes through one `security definer` function, `authenticate_api_key(key_id, secret_hash)`, that returns the tenant id and scopes (or nothing). The application role cannot read `api_keys` directly.

### Database roles
| Role | Can | Cannot |
|---|---|---|
| `clearcut_owner` (or the existing admin) | create and migrate the schema | is never used by the running service |
| `clearcut_app` | `select`/`insert` on tenant tables through RLS; `update` on `documents` (status) | delete or update events/audit rows, `truncate`, bypass RLS, create objects |
| `clearcut_eraser` | `delete` on events/documents rows; owns `erase_document` | log in (no login, no password) |

- Roles are created by a migration (`create role ... if not exists`), with **no password**: a role without a password cannot authenticate by password, so operations set credentials out of band (secret manager). Migrations run as the owner; the service connects as `clearcut_app`. Two connection strings: `DATABASE_URL` (service) and `MIGRATE_DATABASE_URL` (migrations).
- **`erase_document(tenant uuid, document uuid)`** is `security definer`, owned by `clearcut_eraser`, with `execute` granted to `clearcut_app`. It checks that the document belongs to the tenant, deletes its events and the document row, and writes an audit entry (`document.erased`, naming the document and tenant but containing no invoice data). The event trigger then allows a delete only when `current_user` is `clearcut_eraser`, i.e. only from inside that function. The `clearcut.erasure` setting is removed. The audit log still has no erasure path.
- The object in storage (`clearcut-objects`) is deleted by the application after the database call succeeds, and only if no other document row of that tenant references the same content hash.

### Where it lives in code
- `clearcut-core::auth`: `Scope`, `Principal { tenant, key_id, scopes }`, `Principal::require`. Pure, tested.
- `clearcut-store`: key creation (`create_api_key` returns the full key once), `authenticate`, `revoke_api_key`, tenant-scoped transactions, `erase_document`. Tests prove, against a real Postgres, that a connection as `clearcut_app` with the wrong or no tenant sees nothing, cannot delete events, cannot truncate, and that RLS holds even for a deliberately sloppy query that omits `tenant_id`.
- `clearcut-server`: a `Bearer` extractor that authenticates and returns a `Principal`; handlers declare the scope they need. Unauthenticated routes: `/health` only.
- First tenant and first key: a small `clearcut-admin` command (`create-tenant`, `create-key`) run with the owner connection, since the API cannot bootstrap itself. It prints the key once.

## Why these choices
- **RLS in addition to application checks**, because the failure we fear is a forgotten `where tenant_id = ...` in code written months from now. RLS turns that bug into an empty result instead of a data breach.
- **A function for erasure instead of a role-checked setting**: Postgres custom settings can be changed by any session, so "who may set it" cannot be enforced. A `security definer` function is the standard way to give a restricted role exactly one vetted power.
- **Scopes over named roles** (admin/editor/...): fewer concepts, and they translate directly to OAuth scopes for MCP.
- **SHA-256 for key secrets**: the secret is a 256-bit random value, not a human-chosen password, so brute force is infeasible and a fast hash avoids adding a slow-hash cost to every request.

## Decisions on the open questions (Kone, 2026-10-21)
1. **Scope list: as written** (`documents:read`, `documents:write`, `events:read`, `review:write`, `keys:admin`). Reading narration and exports stays under `events:read` for now; splitting upload from review between machine and human keys is already possible because they are separate scopes.
2. **No cross-tenant "super key".** Anything an operator must do across tenants (support, billing) uses the owner database role and the `clearcut-admin` command, never the public API.
3. **Default key expiry of 365 days.** `create_api_key` sets `expires_at` to a year from creation unless a different expiry is given explicitly; there is no "never expires" default. Rotation means issuing a new key before the old one lapses.
4. **Failed-login throttling is left to the deployment** (reverse proxy or gateway). The service only logs failed attempts, without the presented key.

## Implementation notes (step 2, migration 0003)
- **Row-level security is enabled but not `FORCE`d**, a small deviation from the sketch above. With `FORCE` even the owner role would be restricted, and the operator's `clearcut-admin` command (and migrations) deliberately run as the owner and must see across tenants. The protection comes from the service never running as the owner: it connects as `clearcut_app`, which owns nothing and cannot bypass RLS. Tests assert both properties.
- **The `clearcut.erasure` setting is gone.** `forbid_event_changes()` now allows a `DELETE` only when `current_user` is `clearcut_eraser`, i.e. only inside `erase_document`. The application role has no `DELETE`, `UPDATE` or `TRUNCATE` on events or audit rows at all; the triggers remain as a second layer, including against a superuser.
- **`erase_document`** checks that its tenant argument is the current tenant (so the SECURITY DEFINER power cannot be pointed at another tenant), raises `no_data_found` for a missing or foreign document (the store maps that to `DocumentNotFound`), and writes an audit entry with no invoice content.
- **Roles are cluster-wide**, so migration 0003 creates them tolerantly and then states their security-critical attributes explicitly (a pre-existing role with `bypassrls` is repaired, not trusted). The lock is taken as its own statement so later statements see a fresh snapshot; a single `DO` block would act on a stale view when several databases migrate at once.
- The migrating role needs `CREATEROLE` (or to be a superuser). Credentials for `clearcut_app` are set out of band; the migration gives it none.
- `clearcut-store` gained a `build.rs` so Cargo rebuilds when a migration file changes (`sqlx::migrate!` embeds them at compile time).

## Implementation notes (step 3, migration 0004: API keys)
- **A fourth database role, `clearcut_authenticator`** (login-less), owns `authenticate_api_key(key_id, secret_hash)`. It is the only role that can read secret hashes; it may update nothing but `last_used_at`. The application role gets **column-level privileges** on `api_keys`: it can read every column except `secret_hash`, insert, and update only `revoked_at`. So even a flaw in application code cannot read hashes, change a key's scopes, name or expiry, or delete a key.
- **Constant-shape authentication.** The function returns rows only when the key exists, the secret matches, and the key is neither revoked nor expired, with no hint which check failed; the store maps every failure (including a malformed key, which never reaches the database) to the same `Unauthenticated` error. Hashes are compared as hashes of hashes, and an unknown key id does equivalent work. This is a small deviation from "compare in constant time in Rust": the comparison happens in the function so that the application role never needs to read the stored hash at all.
- **Expiry is mandatory and capped.** `expires_at` is `NOT NULL` with `expires_at > created_at`; a key's life defaults to 365 days and can be made shorter but never longer or absent (the store caps it).
- **Management needs `keys:admin`**, checked in the store through the core `Principal` (`create_api_key`, `revoke_api_key`, `list_api_keys`), and acts only on the caller's own tenant. **A caller can only hand out scopes it holds** (found by the automated security review of the first version, which let a `keys:admin`-only key mint a key with any scope): one scope the caller lacks refuses the whole request. Only the operator path can issue scopes nobody holds yet. A tenant's *first* key is made by `create_api_key_as_operator` (for the `clearcut-admin` command), which has no scope check and is never called by the public API. Creation and revocation are written to the audit log without the secret; the key text is only ever returned once, and its `Debug` output hides the secret.
- A stored scope this version does not know makes authentication fail with an error instead of being quietly dropped.
- **Decided by Kone, 2026-10-21: no expiry cap.** Question was: should a key created through the API be capped to expire no later than the key that created it? It would stop a stolen, nearly-expired admin key from minting year-long keys, but it would also stop an admin key from creating its own successor, so every yearly rotation of an administrator would need the operator. New keys therefore get up to the full 365 days regardless of the creator's expiry; the creator's authority is bounded by scopes, tenant and revocation. Revisit if admin keys are issued to people rather than systems.

## Implementation notes (step 4, the `clearcut-admin` command)
- A small `clearcut-admin` crate (library + thin `main`): `migrate`, `create-tenant`, `create-key`, `revoke-key`. It runs on the **owner** connection from `DATABASE_URL` and refuses the service role with a message that says so (it maps Postgres "insufficient privilege" to that message).
- **Scriptable by design:** stdout carries only the machine-readable result (a tenant id, or the new key's token), stderr the human notes; a failed command writes nothing to stdout; exit codes are 0 success, 1 failure, 2 usage. The notes never contain a secret.
- Argument checking is strict and early: scopes must be exact names (the error lists the valid ones), `--days` must be 1 to 365.
- `revoke-key` is the break-glass response to a leaked key and needs no admin key; it uses `Store::revoke_api_key_as_operator`. Everything the command does is audited with actor `{"type":"operator"}`, including tenant creation.
- Runbook: [ADMIN.md](../ADMIN.md).

## Implementation notes (step 5, server authentication)
- `clearcut-server` gets an `Authed` extractor (`Authorization: Bearer ...`). **Every authentication failure is the identical 401** (`WWW-Authenticate: Bearer`, the same body), whether the header is missing, malformed, repeated or wrong, or the key unknown, revoked or expired. A database outage is a **500**, never a 401, with a generic body. The extractor runs **before the request body is read** (axum itself enforces that order), so an anonymous caller cannot probe how the API parses input. A scoped route answers **403 only to an authenticated caller**, naming the missing scope; an anonymous caller gets 401, which does not reveal what the route needs.
- The key is never logged: failures log only a reason, and the `Authorization` header is marked sensitive so a `Debug` print of the headers shows it redacted. The scheme name is case-insensitive (RFC 7235); nothing else about the header is lenient.
- Routes: `/health` public; `/v1/whoami` (any valid key); `/v1/invoices/calculate` now needs a valid key but no particular scope, since it touches no stored data.
- **Startup guard:** the service calls `Store::verify_service_role` and refuses to start if its database role is a superuser, has `BYPASSRLS`, or owns the tenant tables. A misconfigured connection string would otherwise disable tenant isolation without any visible symptom.
- The TypeScript SDK takes `apiKey` in an options object, sends it only on authenticated calls (never to `/health`), holds it in an ES private field so it does not appear if the client is logged or serialised, and refuses a key containing whitespace or control characters (header injection) without repeating the key in the error.

## Consequences
- Local development needs the restricted role, so the dev container instructions gain one step (connect as `clearcut_app`; migrations run as `clearcut`). Tests that exercise RLS switch role inside a transaction.
- Every `Store` method becomes a tenant-scoped transaction, a small mechanical refactor with tests already in place to protect it.
- `ADR-0006`'s `clearcut.erasure` setting is retired; its follow-up (restricting who may set it) is closed by this change.

## Rejected
- *Signed tokens (JWT) as the only credential:* cannot be revoked before they expire without extra state, and the key table already gives revocation and `last_used_at`.
- *Plaintext or reversibly encrypted keys:* a database leak would hand out working credentials.
- *Argon2/bcrypt for key secrets:* right for passwords, wasteful for 256-bit random secrets.
- *Application-only tenant filtering:* one forgotten `where` is a breach.
- *A cross-tenant superuser key in the public API:* the most valuable thing to steal.
