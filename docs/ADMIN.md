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
    --scopes keys:admin,documents:read,documents:write,events:read > first-admin.key
```
- The **key is printed once**, on stdout alone; the notes go to stderr. Move it straight into a secret manager and delete the file. It cannot be shown again; if it is lost, issue a new one.
- The web page needs `documents:read` to list and open documents, `documents:write` to upload and confirm, `events:read` to show what was read, and `keys:admin` to set the account's Anthropic key in Settings. Give a key only the scopes its person needs.
- Keys live at most **365 days** (`--days` can shorten that, not extend it). Rotate by issuing a new key before the old one lapses, then revoking the old one.

## Inviting someone
ClearCut is invite-only in alpha: the public demo shows the journey, and a real account is made here, one at a time.

A new business gets its own tenant and first key:
```
export DATABASE_URL=<the OWNER connection string>
TENANT=$(clearcut-admin create-tenant "Their business")
clearcut-admin create-key --tenant "$TENANT" --name "Anna's first key" \
    --scopes keys:admin,documents:read,documents:write,events:read > anna.key
```
A second person inside an existing business gets a key against that tenant, not a new tenant:
```
clearcut-admin create-key --tenant <tenant id> --name "Ben's key" \
    --scopes documents:read,documents:write,events:read > ben.key
```
- Send the key through a channel you trust, such as a password-manager share. Not a group chat, and not the same email as the invoices.
- One key per person, named for them, so a departure is one `revoke-key --tenant ... --key ...` and nothing else is disturbed.
- Only give `keys:admin` to someone who must set the account's Anthropic key; every other key can read, upload and review without it.
- They open the site, paste the key, then go to Settings and add their own Anthropic key with a spend limit.
- The public demo at `#/demo` needs no key, calls no model and stores nothing. It is the right link to send before someone has a key.
- There is no keys API in the server today: issuing and revoking keys stays an operator action, and the tenant can never see another tenant's keys or documents.

## Running the service
The service reads `DATABASE_URL` too, but it must be the **service role** (`clearcut_app`), not the owner's. At startup it checks the role and **refuses to start** if the role is a superuser, can bypass row-level security, or owns the tenant tables, because running as the owner would silently switch tenant isolation off. It never migrates or creates tenants; that is this command's job. Callers present their key as `Authorization: Bearer ccut_...`; `GET /health` is the only route that needs no key, and `GET /v1/whoami` shows which tenant, key and scopes a key carries.

Three more settings belong to the service: `CLEARCUT_STORAGE_DIR` (**required**: the directory where uploaded files are kept, one sub-directory per tenant; the server will not start without it, so invoices are never written to a place nobody chose), `CLEARCUT_MAX_UPLOAD_BYTES` (optional, default 67108864 = 64 MiB, the most one file may be) and `CLEARCUT_TENANT_QUOTA_BYTES` (optional, default 1073741824 = 1 GiB, the most one account's files together may be; an upload that would overfill it is refused with `507`, deleting documents frees room, and a repeat of a document already stored costs no room). A fourth, `CLEARCUT_RATE_LIMIT_PER_MINUTE` (optional, default 120), caps requests per account; after it a caller gets `429` with `Retry-After` until the minute rolls over.

**In front of authentication.** The server also limits by address, checked before the database is asked about a key: `CLEARCUT_PER_IP_PER_MINUTE` (optional, default 300) caps requests from one address a minute; `CLEARCUT_AUTH_FAILURES_PER_MINUTE` (optional, default 20) caps missing or wrong credentials, after which the address is answered `429` for `CLEARCUT_BLOCK_SECONDS` (optional, default 60). `CLEARCUT_MAX_CONCURRENT` (optional, default 256) answers `503` with `Retry-After` when that many requests are already in flight, and `CLEARCUT_REQUEST_TIMEOUT_SECONDS` (optional, default 120) answers `408` to a request that runs too long. `CLEARCUT_TRUST_PROXY` must be `1` only when Caddy is in front (see [DEPLOY.md](DEPLOY.md)); otherwise the server uses the socket address it sees. The access log records one line per request with method, path, status, duration, address and the `X-Request-Id` the answer carries; it never records a key, a body or document text.

**Retention.** `CLEARCUT_RETENTION_DAYS` (optional, default 365, `0` turns it off) makes the server erase a document once it is that many days **older than when it was uploaded**, whether or not anyone read it again since. The erasure is the same one `DELETE /v1/documents/{id}` uses: the readings, events, jobs and row go in one step, the file is removed when no other document of the account uses the same bytes, and the audit entry and the call log's costs stay. The sweep runs a minute after the server starts and then every hour; a document still being read at that moment is left for the next pass, so retention can be delayed but not skipped. An account's documents therefore leave on their own: say so in the privacy notice, and set a period the responsible party can justify (tax records may need longer than a year; backups keep their own period, see [DEPLOY.md](DEPLOY.md)). Back up the storage directory together with the database: the database records each document, the directory holds its bytes.

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
