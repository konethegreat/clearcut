# Operating ClearCut: the operator runbook

_Author: Kone Tshivhinda · Written by Kone Tshivhinda & Claude_

`clearcut-admin` is the operator's command. It exists because the public API cannot bootstrap itself (a tenant needs its first key before anyone can call the API) and because some work spans tenants (revoking a leaked key). It is never exposed over the network. Design: [ADR-0007](adr/0007-auth-and-row-level-security.md).

## Two database connections, never mixed up
| | Used by | Can |
|---|---|---|
| **Owner** | migrations and `clearcut-admin` | everything, across tenants (it is not restricted by row-level security) |
| **`clearcut_app`** | the running service | only what row-level security and its narrow grants allow; it cannot create tenants, read secret hashes or delete the event log |

`clearcut-admin` reads `DATABASE_URL` and **must be given the owner connection**. If it is run with the service role it refuses with "this command needs the owner database connection". The service role's credentials are set out of band (the migrations create the role with no password).

## First-time setup of a tenant
```
export DATABASE_URL=<the OWNER connection string>

clearcut-admin migrate                                     # safe to repeat
TENANT=$(clearcut-admin create-tenant "Acme Advisory")     # stdout is just the tenant id
clearcut-admin create-key --tenant "$TENANT" --name "first admin" \
    --scopes keys:admin,documents:write,events:read > first-admin.key
```
- The **key is printed once**, on stdout alone; the notes go to stderr. Move it straight into a secret manager and delete the file. It cannot be shown again; if it is lost, issue a new one.
- After that, holders of `keys:admin` manage the tenant's keys through the API themselves. **An administrator can only hand out scopes it holds**, so give the first admin key every scope its tenant's administrators should be able to delegate.
- Keys live at most **365 days** (`--days` can shorten that, not extend it). Rotate by issuing a new key before the old one lapses, then revoking the old one.

## Running the service
The service reads `DATABASE_URL` too, but it must be the **service role** (`clearcut_app`), not the owner's. At startup it checks the role and **refuses to start** if the role is a superuser, can bypass row-level security, or owns the tenant tables, because running as the owner would silently switch tenant isolation off. It never migrates or creates tenants; that is this command's job. Callers present their key as `Authorization: Bearer ccut_...`; `GET /health` is the only route that needs no key, and `GET /v1/whoami` shows which tenant, key and scopes a key carries.

Three more settings belong to the service: `CLEARCUT_STORAGE_DIR` (**required**: the directory where uploaded files are kept, one sub-directory per tenant; the server will not start without it, so invoices are never written to a place nobody chose), `CLEARCUT_MAX_UPLOAD_BYTES` (optional, default 67108864 = 64 MiB, the most one file may be) and `CLEARCUT_TENANT_QUOTA_BYTES` (optional, default 1073741824 = 1 GiB, the most one account's files together may be; an upload that would overfill it is refused with `507`, deleting documents frees room, and a repeat of a document already stored costs no room). A fourth, `CLEARCUT_RATE_LIMIT_PER_MINUTE` (optional, default 120), caps requests per account; after it a caller gets `429` with `Retry-After` until the minute rolls over. Back up the storage directory together with the database: the database records each document, the directory holds its bytes.

The quota and the rate limit are counted per **account** (tenant), not per key, and both must be a positive whole number or the service refuses to start.

## When a key leaks
```
clearcut-admin revoke-key --tenant <tenant id> --key <key id>
```
It takes effect at once and does not need an admin key. The key id is the 32 hex characters after `ccut_` in the key text (with dashes added), and is also in the output of `create-key` and in the tenant's key listing. Everything the operator does is written to the audit log as `{"type":"operator"}`.

## Scopes
`documents:read`, `documents:write`, `events:read`, `review:write`, `keys:admin`. No scope implies another.

## What it does not do (yet)
Erasing a document (POPIA) is done by the service through the `erase_document` database function, not here. Listing every tenant's keys and rotating the service role's password are not implemented; use SQL with the owner connection for now.
