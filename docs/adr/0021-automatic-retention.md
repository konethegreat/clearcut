# ADR-0021: Automatic retention

Status: **Accepted** (built 28 Oct 2026 as the third item in Kone Tshivhinda's order; Kone chose the one-year period and can overrule any part) · Date: 2026-10-28 · Deciders: Kone Tshivhinda, Claude
Builds on [ADR-0007](0007-auth-and-row-level-security.md) (tenants, roles and row-level security) and the erasure a person's delete already uses.

## Context
POPIA expects personal information to be kept no longer than the purpose needs, and a person who uploads an invoice should not have to remember to delete it. Until now nothing was deleted automatically: a document lived until its account deleted it. The erasure itself already existed, with the properties that matter (a waiting job is cancelled, a running one is refused, the file goes once no document of the account uses those bytes, the audit entry and the call log stay), so the missing piece was only *when*: a decision that a document is old enough, found across every account.

The application role reads one tenant at a time (row-level security). A sweep has to look at every account, which the application role cannot do without weakening it for every request.

## Decision
1. **Age is counted from upload** (`documents.created_at`), not from the last read. Reading or re-analysing a document does not extend its life. Otherwise a document could be kept forever by opening it, which defeats the purpose.
2. **One period for the whole server**, `CLEARCUT_RETENTION_DAYS`: unset or empty means 365 days, `0` turns the sweep off, and the accepted range is 1 to 36,500 days. A per-account setting is not built; a customer who needs a different period runs their own server or asks for one later.
3. **The sweep reuses the person's erasure.** It calls the same `erase_document` a delete calls, so the behaviour (cancel a waiting job, refuse a running one, remove the file only when unused, keep audit and call log) cannot drift between the two paths. The audit entry records actor `{"type":"retention"}` and reason `{"retention_days": N}`, so an erasure nobody clicked is still attributable.
4. **Queued jobs do not protect a document; running jobs do.** A queued job has not started, and the erasure cancels it first, exactly as a person's delete does. This matters for a deployment with no worker, where jobs would otherwise stay queued forever and the documents would never expire. A running job is work in progress: the erasure refuses it, the sweep leaves the document, and the next pass tries again.
5. **One login-less function finds the candidates.** Migration 0011 adds role `clearcut_retention`, owned by a `security definer` function `expired_documents(days, limit)`. The role sees only three columns of `documents` and two of `jobs` (never a lease token), and the function returns **only two ids** per document, no content, at most `limit` (1 to 1000) and only for documents past the threshold with no running job. Only `clearcut_app` may execute it. `documents (created_at)` gains an index so the sweep does not scan the table each hour.
6. **Schedule:** the first pass runs 60 seconds after the server starts (so a restart does not compete with serving requests), then every hour. A pass asks for batches of 100 and stops when a batch is not full or a round erased nothing, so failures are not retried in a tight loop; whatever is left is tried again on the next pass, and a failure is logged without content.

## What this protects, and what it does not
- A document of any account is erased after the period even if nobody opens ClearCut, and the only thing that can stop that is a **running** job, which is short-lived work.
- The sweep cannot read content: its function returns ids only, its role sees no other column, and the erasure it calls goes through the intake crate's normal path.
- It does **not** reach into backups. A backup taken before an erasure keeps the document until that backup expires; this is stated in the privacy documents rather than hidden.
- It does not delete the audit entry or the call log: those hold no content, and POPIA (and a customer's own audit) wants the record of the erasure to remain.

## Tests
In the store, against a real database: only aged documents are candidates, every account is covered, oldest first, the batch limit is respected, a running job excludes a document while a queued one does not, the function refuses a period below one day, the function is a vetted security-definer owned by the retention role, and only the application role may call it. In intake: the sweep's erasure removes document, job and file and records the retention actor and reason, refuses a running job, and reports an already-gone document as not found. In the server: the period is 365 unless set and `0` turns it off, and a sweep erases only documents past the period.
