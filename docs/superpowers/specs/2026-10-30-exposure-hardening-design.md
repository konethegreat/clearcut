# Exposure hardening design

_Kone Tshivhinda & Claude, 30 Oct 2026._

## Goal

Make the public ClearCut server refuse abusive traffic cheaply, before it costs a database
lookup, and close the one upload hole the threat model already names: a file is trusted to be
what its `Content-Type` says. Add the honest alpha label the site is missing.

This is the first of four tracks. The other three are recorded in
[docs/LAUNCH.md](../../LAUNCH.md#6-next-tracks) so they are not lost: scale readiness, a
universal invoice core, and published legal and trust pages.

## Scope

**In this slice**

1. A per-IP request limit and a failed-authentication budget that run **before** the key lookup.
2. A global concurrency cap that sheds load, and a request timeout.
3. Response headers the site does not send yet: `Permissions-Policy`, `Cross-Origin-Opener-Policy`,
   `Cross-Origin-Resource-Policy` (HSTS, `nosniff` and `Referrer-Policy` already exist).
4. One structured access line per request with a request id, status, duration and client IP.
   The key, the body and the query string are never logged.
5. Magic-byte validation for PDF, PNG, JPEG, TIFF and XML at intake, so a renamed file never
   reaches storage or the worker.
6. A persistent alpha banner on every screen.

**Out of this slice**

- A WAF, a CDN or a bot challenge. Those need a domain and an edge provider; the app and Caddy
  are the only layers today. Recorded as the next step for bot protection.
- Metrics, readiness, connection-pool tuning, pagination and a multi-instance rate limiter.
  That is the scale track.
- Jurisdiction profiles and per-currency minor units. That is the universal-core track.
- Terms, a published privacy notice, ownership and trademark pages. That is the legal track and
  needs Kone's entity details plus an attorney.

## Current posture

The defences that stay unchanged are in [docs/THREAT-MODEL.md](../../THREAT-MODEL.md): row-level
security per account, hashed 256-bit keys, encrypted Anthropic keys, per-account request limits,
streaming upload and quota caps, worker confinement, CSP with no inline script, text-only
rendering, prompt-injection rules, export formula neutralising and dependency audits.

The gaps this slice closes:

- `Authed` calls the database for every presented key, including a wrong one, and only counts
  requests per account **after** a key is accepted ([auth.rs:251](../../../crates/clearcut-server/src/auth.rs#L251)).
- Nothing bounds a slow request, a flood of requests or the number of requests in flight.
- The server never sees a client address: it does not read `X-Forwarded-For` or `ConnectInfo`.
- The upload path accepts the declared media type and never looks at the first bytes
  ([intake/lib.rs:32](../../../crates/clearcut-intake/src/lib.rs#L32), P2-02).
- Responses lack `Permissions-Policy`, COOP and CORP.
- There is no access log, so a flood is invisible after the fact.
- The page says "alpha" in the footer and the hero chip, but nothing at the top says the system
  is in development.

## Design

### 1. Client identity

A new `guard` module in `clearcut-server` derives one client address per request:

- If `CLEARCUT_TRUST_PROXY` is **on**, take the **last** entry of `X-Forwarded-For`. Caddy appends
  the address it saw, so the last entry is the real peer even if a client sent fake earlier
  entries. If it is missing or unparseable, fall back to the socket address.
- Otherwise use `ConnectInfo<SocketAddr>`, which requires serving with
  `into_make_service_with_connect_info`.
- If neither exists (unit tests, a transport with no peer), the address is `None` and the per-IP
  controls are skipped. The global controls still apply.

`CLEARCUT_TRUST_PROXY` is **off by default**. It is turned on only when the server is reachable
solely through Caddy, as in the HTTPS deployment. A direct client could otherwise spoof the
header.

### 2. Limits before authentication

The guard keeps a fixed one-minute window per address:

- `requests` counts every request to a route that extracts `Authed`.
- `failures` counts missing, malformed and rejected credentials.
- At `CLEARCUT_AUTH_FAILURES_PER_MINUTE` failures (default **20**) the address is blocked for
  `CLEARCUT_AUTH_BLOCK_SECONDS` (default **60**). A blocked request gets `429` with
  `Retry-After` **before** the authenticator is called, so it costs no database lookup.
- A successful authentication does not clear the failure count. One valid key must not erase an
  attacker's history.
- At `CLEARCUT_RATE_LIMIT_PER_IP_PER_MINUTE` requests (default **300**) further requests get
  `429` with `Retry-After`, also before the lookup. The limit is generous enough for a shared
  office address; the per-account limit (default 120) remains the tighter control for one key.

The map is bounded at 65 536 addresses. When it is full, the entry with the oldest window is
evicted, so memory cannot be exhausted by a distributed flood. Evicting an attacker's entry only
costs a fresh window.

### 3. Global guards

One middleware wraps every route, including static files and `/health`:

- **Concurrency**: at most `CLEARCUT_MAX_CONCURRENT_REQUESTS` (default **256**) requests in
  flight. The permit is held until the handler returns, which includes reading an upload. Over
  the cap the answer is `503` with `Retry-After: 1`; it does not wait and pile up.
- **Timeout**: `CLEARCUT_REQUEST_TIMEOUT_SECS` (default **120**). A handler that outlasts it is
  cancelled and the answer is `408`. 120 s is enough for a 64 MiB upload on a slow link.
- Both are per process. The deployment runs one server; a multi-instance design is in the scale
  track.

### 4. Headers and request id

Every response gains, without replacing what a handler already set:

| Header | Value |
|---|---|
| `Permissions-Policy` | `camera=(), microphone=(), geolocation=(), payment=(), usb=()` |
| `Cross-Origin-Opener-Policy` | `same-origin` |
| `Cross-Origin-Resource-Policy` | `same-origin` |
| `X-Content-Type-Options` | `nosniff` (if absent) |
| `Referrer-Policy` | `no-referrer` (if absent) |
| `X-Request-Id` | the request's id |

The request id is taken from an incoming `X-Request-Id` only if it is 1 to 64 characters of ASCII
letters, digits or `-`; otherwise a fresh UUID is used. The same id goes in the response and the
access line, so a report from a person can be matched to the server's record.

### 5. Access log

One line per request, at `info` for successes, `warn` for 4xx, `error` for 5xx, and `debug` for a
successful `/health`:

```text
request method=POST path=/v1/documents status=201 ms=842 ip=203.0.113.7 id=...
```

Never logged: the `Authorization` header, the body, the query string (it can carry a file name),
or any document text.

### 6. Upload content check

`clearcut-intake` reads up to 16 bytes before storing anything and checks them against the
declared type:

| Type | Accepted start |
|---|---|
| PDF | `%PDF-` |
| PNG | `89 50 4E 47 0D 0A 1A 0A` |
| JPEG | `FF D8 FF` |
| TIFF | `II*\0` or `MM\0*` |
| XML | an optional UTF-8 or UTF-16 BOM, then whitespace, then `<` |

An empty body is `Empty` (400). A mismatch is `ContentMismatch` (415). The bytes read are replayed
to the object store, so a valid file is stored unchanged. The check lives in `Intake::upload`, so
the REST API and the MCP upload path get it together. The worker still parses hostile files in
its container; this check only stops non-documents earlier.

### 7. Alpha banner

A strip above the header on every screen:

> **Alpha** ClearCut is in active development. Check every figure that matters against the
> original document.

"Alpha" is used rather than "beta" because the project's own docs, README and hero chip already
say alpha, and the software is pre-release. The banner is static HTML, styled in `app.css`, with
no inline style and no script, so the CSP tests still hold.

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `CLEARCUT_TRUST_PROXY` | off | Trust the last `X-Forwarded-For` entry (only behind Caddy) |
| `CLEARCUT_RATE_LIMIT_PER_IP_PER_MINUTE` | 300 | Requests per address per minute, before auth |
| `CLEARCUT_AUTH_FAILURES_PER_MINUTE` | 20 | Failed credentials per address per minute before a block |
| `CLEARCUT_AUTH_BLOCK_SECONDS` | 60 | How long a blocked address waits |
| `CLEARCUT_MAX_CONCURRENT_REQUESTS` | 256 | Requests in flight before shedding |
| `CLEARCUT_REQUEST_TIMEOUT_SECS` | 120 | Longest a request may take |

All are validated at startup in the same style as the existing limits: a bad value stops the
server with a plain message and exit code 2. The existing `CLEARCUT_RATE_LIMIT_PER_MINUTE`
(default 120 per account) is unchanged.

## Failure behaviour

- A missing client address skips the per-IP controls, never the global ones.
- The limiter mutex recovers from poisoning; a panic elsewhere cannot disable limiting.
- A request refused for concurrency or timeout still gets the security headers and a log line.
- The authentication failure count is in memory. Restarting the server clears it, which is
  acceptable: the per-IP request limit and the global cap do not depend on history.

## Testing

- Unit tests for the guard: exact limit, `Retry-After` ceiling, window reset, independent
  addresses, failure block, success not clearing failures, oldest-entry eviction.
- Extractor tests with a fake authenticator: the third request over a limit of two is `429`; the
  request after the failure budget is `429` and the authenticator's call count did not grow.
- Middleware tests: a slow handler gets `408`; a second request over a concurrency cap of one
  gets `503`; every response carries the new headers and a valid `X-Request-Id`.
- Intake tests: each signature accepted, a PDF renamed as PNG refused, an executable refused as
  PDF, XML with a BOM accepted, the replayed body byte-identical.
- Web test: the banner text is present in the served page and the CSP invariants still pass.
- The full workspace suite, then the live deployment.

## Deployment

The HTTPS deployment sets `CLEARCUT_TRUST_PROXY=1` in the VM's `.env`, because Caddy is the only
way in. Without it, every client looks like Caddy and the per-IP limit becomes one global bucket.
The compose file passes the new variables through with defaults.
