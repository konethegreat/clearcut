# ADR-0015: The try-it deployment

Status: **Accepted** (approved by Kone Tshivhinda, 2026-10-23, with all seven recommended answers; the first release is the local path only) · Date: 2026-10-23 · Deciders: Kone Tshivhinda, Claude
Task P1-09 (with P1-07). Builds on [ADR-0011](0011-bring-your-own-anthropic-key.md) (the key; this adds a third way to hold it), [ADR-0007](0007-auth-and-row-level-security.md) (tenants and row-level security), [ADR-0009](0009-worker-and-provider-boundary.md) (the worker never sees a key and has no internet), [ADR-0014](0014-context-pack-builder.md) (redaction policy). Free-tier facts were read on 23 Oct 2026 from the providers' pages and third-party summaries; several have changed recently, so **each is to be re-checked on the provider's own site before relying on it**. Design only; nothing is built until this is approved, and the trial is only useful once the end-to-end slice (upload to events) exists, so the build order puts this after it.

## What it is
A person should be able to try ClearCut with their own Anthropic key, see whether it meets their needs, and then **remove the key and everything they uploaded**. That is two ways to run the same program:

```text
local   (recommended)   docker compose on your own computer          your key never leaves it
trial   (hosted, free)  one small public instance, no accounts         your key lives in the server's memory for the length of your session, then is wiped
```

Both come from one codebase and one image. A setting, `CLEARCUT_MODE` (`local` or `trial`, no default for `trial`: it must be asked for), switches on the trial behaviour. Neither costs anything to run; the only money is the user's own tokens (R-figures as in ADR-0011).

## 1. Mode C: the session key
ADR-0011 has mode A (the operator's environment or file) and mode B (per-tenant, encrypted at rest). A public trial needs a third, **mode C: a key that exists only for a session and only in memory.**

- **Giving the key.** The user pastes it into a password field on the try page. The page sends it **once**, in the body of `PUT /v1/session/key` over HTTPS, and clears the field. It is never in a URL, a query string, a header on later requests, `localStorage`, `sessionStorage`, a cookie, or any file.
- **What the browser keeps.** Only an opaque 256-bit session id in a cookie (`HttpOnly; Secure; SameSite=Strict`, no `Domain`). The key itself is **not kept in the browser at all** after the paste. This is stricter than "in browser memory": a page script that holds a key is a key that any injected script can read, and it gains nothing, because the server needs the key anyway to finish jobs after the request ends.
- **What the server keeps.** A map in memory from session id to the key (the existing `ApiKey` secret type: unprintable, zeroed on drop), when the session was made and last used, and the money spent. It is **never written** to the database, a file, a log, a metric or a core dump (core dumps are switched off in trial mode at start-up). A restart wipes every key, and users simply paste again.
- **Checking it.** On paste the server runs the free key check (`GET /v1/models`, ADR-0011) and shows what it found: the last four characters, the models the key can use, and a reminder to set a spend limit. An admin key or a login token is refused, as today.
- **Removing it.** A **Remove my key** button calls `DELETE /v1/session`: the key is zeroed and dropped at once, the session ends, and everything that session uploaded or produced is deleted (§3). Closing the tab does not do this by itself; the idle timeout does (§3).
- **Time limits.** Idle for 30 minutes or 4 hours in all ends the session the same way. At most 50 sessions exist at once (a bound on memory and on abuse); a new one past that is refused politely.
- **Using it.** The job runner builds the adapter for the session's key when it runs that session's jobs. If the session has gone, its jobs fail as a key problem that uses no attempts, and their documents are already deleted.
- **No oracle for stolen keys.** A public endpoint that says "this key works" is a free way to test stolen keys. So `PUT /v1/session/key` is limited per address (ten a day by default), refuses cross-origin requests (the `Origin` must match; no CORS), and answers a bad key with the same fixed sentences the key check already produces. An optional proof-of-work or a free bot check can be added if abuse appears.
- **Never in the output.** The key is a secret type in every struct that holds it; the tests use a sentinel key and assert it is in no response body, header, log line or error, including panics and debug output. The reverse proxy is configured not to log request bodies.
- **Honest limit, stated on the try page:** the operator's process holds your key in memory while your session lasts, so you are trusting the operator for that time. That is why the page tells you to give it a **dedicated Anthropic workspace key with a low monthly spend limit and a short expiry (Anthropic allows as little as three hours)**, and to disable the key when you are done. Mode C is a convenience for trying, not a place to keep a production key.

## 2. The local path (P1-07)
This is the answer to "where can I put my key safely": on your own computer.

- `docker compose up` starts the **server**, the **worker**, **Postgres**, and a volume for uploaded files. One image for the server and one for the worker, both running as a non-root user with a read-only root file system.
- **Networks:** the worker and Postgres sit on an internal network with **no internet access** (ADR-0009 already says the worker has no egress); only the server is on both. The server's port is published on `127.0.0.1` only, so nothing outside the computer can reach it.
- **The key:** put it in a file (`./secrets/anthropic_key`, mounted as a Docker secret and read through `ANTHROPIC_API_KEY_FILE`, mode A), or leave it out and paste it into the page (mode C also works locally, in a single persistent tenant). Never as an environment value in the compose file, never on a command line.
- **First run:** the migration runs with the owner login; the server then runs as the restricted role (ADR-0007). A one-line command creates the tenant and prints its API key once.
- **Removing everything:** `docker compose down -v` deletes the data; deleting the secret file removes the key.
- The same compose file has a `trial` profile for the hosted path.

## 3. Ephemeral demo data (trial mode)
- **No accounts.** A session is an **ephemeral tenant**: it is created with the session, isolated from every other by row-level security like any tenant, and removed with it.
- **Deleted when the session ends** (Remove, idle timeout or the four-hour limit): its documents and page images (object storage prefix), jobs, extractions, events, exports and audit rows. A janitor also sweeps anything older than four hours, so a crash cannot leave data behind. A test proves a removed session leaves no row and no object. **This needs a way to delete a tenant's data; if that needs a schema change, it will be shown to you before it is applied.**
- **Try it with samples first.** The page offers the invented sample invoices (the same 15 used for the gold-set tooling) with one click, so a person can see the whole flow without uploading anything of theirs.
- **Their own documents:** only with a tick-box (and a link to the privacy notice, P2-19) saying these are documents they are entitled to upload, and asking them not to upload other people's personal data. Uploads are small (5 MB, 20 pages) in this mode.
- **Spending:** a session total of **R20** by default (enforced by the cost guard, P2-12), shown as it runs, on top of whatever limit the user set in the Console. At R18 per US dollar that is about a thousand Haiku pages at the ADR-0005 illustration, so it is a ceiling that rarely binds, not a target.
- **Redaction:** policy `best_effort` (ADR-0014) until the image masker exists in the worker; the page says what was and was not hidden for each document. Setting `strict` is one line for an operator who wants it, at the price that any page with an account number is refused.
- **Nothing else is kept:** the server writes no document text to logs, and the operator keeps only anonymous counters (sessions, documents, errors), no content.

## 4. Where to host the free trial
Needs: the Rust server (small), the Python worker (a few hundred megabytes, more once OCR exists), a small Postgres, ephemeral files, **one always-on instance** (sessions live in memory, so scale-to-zero and several instances both break them), and HTTPS. Ephemeral data suits the free tiers' non-persistent disks.

| Option | Fit | Catch |
|---|---|---|
| **Oracle Cloud Always Free VM** (recommended) | One VM runs the whole compose stack, always on, enough memory for OCR later | The Arm allowance was reportedly cut to 2 CPUs / 12 GB in June 2026 (sources conflict); a card is needed for verification; free capacity is sometimes unavailable; you patch the VM yourself |
| **Google Cloud Run**, at most one instance, with Neon and R2 (fallback) | Managed HTTPS; free quota of 2M requests, 180 000 vCPU-seconds and 360 000 GiB-seconds a month | Needs a billing account with **no hard spend cap** (set budget alerts and a quota); a cold start wipes every session; the worker needs its own service |
| Render free | Easy | Sleeps after 15 minutes (wiping sessions), no disk, free Postgres expires after 30 days |
| Koyeb free | Easy | 512 MB and 0.1 vCPU is too small for the worker |
| Hugging Face Spaces | 2 vCPU, 16 GB | Free Docker spaces reported to be changing; disk not persistent |
| Fly.io | | No free tier for new customers |

**Proposal:** the Oracle VM running `docker compose --profile trial`, with **Caddy** for automatic HTTPS and a **Cloudflare Tunnel** (free) in front so the VM exposes no public port and the origin address stays hidden; firewall closed except the tunnel; swap off so a key is never paged to disk; unattended security updates. If Oracle is not available, fall back to Cloud Run with `max-instances=1`. Either way a short guide (`docs/DEPLOY.md`) will give the exact steps. **Cost: R0**, apart from a domain name if you want one (a free subdomain works).

## 5. The pieces to build (after approval, after the end-to-end slice)
1. **P1-07** Dockerfiles and the compose file (server, worker, Postgres, volume, networks, secrets), a first-run command, a CI job that builds the images and starts the stack.
2. **Session store and endpoints** in `clearcut-server`, trial mode only: `PUT /v1/session/key`, `GET /v1/session`, `DELETE /v1/session`, the cookie, the `Origin` check, the per-address limit, the timeouts, the cap on sessions, all tests-first with a sentinel key.
3. **Ephemeral tenants and the janitor** (`clearcut-store`: create a tenant for a session, delete everything of a tenant).
4. **The runner uses the session's adapter** (with the end-to-end slice and the cost guard).
5. **The try page**: one small TypeScript page served by the server: paste key, samples, upload, progress, results, exports, Remove. Strict headers: `Content-Security-Policy` with `script-src 'self'` and nothing from other sites, `Referrer-Policy: no-referrer`, `Cache-Control: no-store` on every session response.
6. **`docs/DEPLOY.md`** and the README's two paths.
7. A security review of the whole trial mode before it goes public (a slice of P8-05).

## 6. Open points that are yours to decide before launch
- **Who runs the hosted trial.** Whoever hosts it handles other people's invoices and becomes the responsible party under POPIA (ADR-0011): the privacy notice and consent text (P2-19), and whether an Information Officer must be registered. This is the open item in CLAUDE.md; the local path avoids it entirely.
- **The domain** (a free subdomain works to start).

## Alternatives considered
- *Key in browser memory, sent with every request.* Every request carries the key (more places to leak it: proxies, logs), it is exposed to any injected script, and background jobs still need it on the server. Rejected.
- *Server-side storage of an encrypted key (mode B) for the trial.* Needs a master key and a database migration and leaves keys at rest on a stranger's server; the point of the trial is that nothing is kept.
- *Accounts and a database of users.* Far more to secure and to be responsible for, for a try-out.
- *Only the local path.* The safest, and the default recommendation, but "install Docker" stops many people from trying it.
- *Serverless free tiers as the primary choice.* Scale-to-zero wipes in-memory sessions; one always-on instance suits the design better.

## Questions for Kone
1. **One codebase, two modes** (`local` and `trial`, the second only when asked for), with the local compose path as the recommended way to try it: agreed?
2. **Mode C keeps the key in server memory only; the browser keeps just an opaque cookie** (not the key, not even in memory): agreed, or do you want the key kept in the browser as well?
3. **Host the trial on an Oracle Always Free VM** with Caddy and a Cloudflare Tunnel, falling back to Cloud Run with one instance: agreed?
4. **Ephemeral rules:** delete on Remove, after 30 idle minutes or 4 hours, samples first, a consent tick-box for own documents, R20 per session: agreed?
5. **Redaction policy `best_effort`** for the trial until the image masker exists: agreed?
6. **Per-address limit and an `Origin` check** on the key endpoint so it cannot be used to test stolen keys: agreed?
7. **Who runs the hosted trial** (you, as the responsible party), or should the first release be the local path only? (Not blocking approval of the design.)

## Decisions (Kone's answers to the seven questions)
1. **One codebase, two modes** (`local` and `trial`); the local compose path is the recommended way to try it.
2. **Mode C keeps the key in server memory only;** the browser keeps just an opaque cookie.
3. **Hosted trial on an Oracle Always Free VM** with Caddy and a Cloudflare Tunnel, Cloud Run with one instance as fallback.
4. **Ephemeral rules:** delete on Remove, after 30 idle minutes or 4 hours, samples first, a consent tick-box, R20 per session.
5. **Redaction policy `best_effort`** for the trial until the image masker exists.
6. **Per-address limit and an `Origin` check** on the key endpoint.
7. **The first release is the local path only.** The hosted trial waits until Kone decides who runs it (the POPIA responsible party, privacy notice, Information Officer); the design stays accepted so it can follow without a new decision.
