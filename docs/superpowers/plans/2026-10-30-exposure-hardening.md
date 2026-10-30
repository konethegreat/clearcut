# Exposure hardening implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans. Steps use checkbox syntax.

**Goal:** Refuse abusive traffic before it costs a database lookup, close the upload media-type hole, and label the site as alpha on every screen.

**Architecture:** One `guard` module in `clearcut-server` owns client identity, the per-IP limits, the concurrency cap, the timeout, the security headers, the request id and the access log. A checked prefix reader in `clearcut-intake` validates the first bytes of every upload. The page gets a static banner.

**Tech stack:** axum 0.8, tower middleware, tokio, Rust string tests, vanilla HTML/CSS.

**Spec:** [2026-10-30-exposure-hardening-design.md](../specs/2026-10-30-exposure-hardening-design.md)

## Global constraints

- Branch `feat/exposure-hardening`; conventional commits; one PR; CI green; squash merge.
- No new dependency. `tower-http` keeps only `sensitive-headers`.
- Never log the `Authorization` header, a request body, a query string or document text.
- The per-IP controls are off when the client address is unknown and when `CLEARCUT_TRUST_PROXY` is off.
- Every new limit has a test that proves the limit, not only the happy path.
- Docs in the same PR: THREAT-MODEL, LAUNCH, DEPLOY, WORKLOG.

---

### Task 1: The guard module

**Files:**
- Create: `crates/clearcut-server/src/guard.rs`
- Modify: `crates/clearcut-server/src/lib.rs` (module, re-export)
- Modify: `crates/clearcut-server/src/auth.rs` (AppState field, `with_guard`, extractor checks)

**Interfaces:**
- Produces: `GuardConfig { per_ip_per_minute: NonZeroU32, failures_per_minute: NonZeroU32, block: Duration, max_concurrent: NonZeroUsize, timeout: Duration, trust_proxy: bool }`
- Produces: `Guard::new(GuardConfig)`, `Guard::client_ip(&Parts) -> Option<IpAddr>`, `Guard::before_auth(IpAddr, Instant) -> Result<(), u64>`, `Guard::record_failure(IpAddr, Instant)`, `Guard::record_success(IpAddr, Instant)`
- Produces: `ClientAddr(pub Option<IpAddr>)` request extension.
- Consumes: `AppState::guard()`.

- [ ] Write unit tests first: exact limit, `Retry-After` ceiling, window reset, independent addresses, failure block, success does not clear failures, oldest eviction at the cap, proxy parsing (last XFF entry, invalid entry falls back).
- [ ] Run `cargo test -p clearcut-server --lib guard::` and see them fail.
- [ ] Implement the module and wire it into `AppState`.
- [ ] Add extractor tests with `ConnectInfo`: limit of two gives a third-request `429`; failure budget of two gives `429` on the third and the authenticator was called twice.
- [ ] Run the server library tests.

### Task 2: The guard middleware

**Files:**
- Modify: `crates/clearcut-server/src/guard.rs`
- Modify: `crates/clearcut-server/src/lib.rs` (`finish` applies the middleware)
- Modify: `crates/clearcut-server/src/tests.rs` (test routes: `/test/slow`, guard helpers, header assertions)

**Interfaces:**
- Produces: `pub async fn guard_request(State<AppState>, Request, Next) -> Response`
- Behaviour: derive and insert `ClientAddr`; acquire a semaphore permit or `503` with `Retry-After: 1`; run under `tokio::time::timeout` or `408`; add the six response headers and `X-Request-Id`; log one line.

- [ ] Write tests first: slow handler `408`; cap of one gives one `503` from two concurrent slow requests; every response has `Permissions-Policy`, COOP, CORP, nosniff, `Referrer-Policy` and a valid `X-Request-Id`; a hostile `X-Request-Id` is replaced.
- [ ] Implement the middleware and apply it in `finish`.
- [ ] Run the server library tests.

### Task 3: Upload magic bytes

**Files:**
- Modify: `crates/clearcut-intake/src/lib.rs` (`ContentMismatch`, `checked_body`, `Prefixed`)
- Modify: `crates/clearcut-server/src/documents/mod.rs` (map `ContentMismatch` to `415`)
- Test: `crates/clearcut-intake/src/tests.rs`

**Interfaces:**
- Produces: `IntakeError::ContentMismatch`
- Produces: `async fn checked_body<R: AsyncRead + Unpin>(MediaType, R) -> Result<impl AsyncRead + Unpin, IntakeError>`
- Produces: `fn signature_matches(MediaType, &[u8]) -> bool`

- [ ] Write tests first: each signature accepted; PDF bytes as PNG refused; `MZ` as PDF refused; XML with UTF-8 BOM accepted; body bytes replayed exactly; empty body is `Empty`.
- [ ] Implement, call it before `objects.put`, and map the error to `415`.
- [ ] Run the intake unit tests (the Postgres-backed tests are unchanged).

### Task 4: The alpha banner

**Files:**
- Modify: `crates/clearcut-server/src/web/index.html`
- Modify: `crates/clearcut-server/src/web/app.css`
- Test: `crates/clearcut-server/src/web.rs`

- [ ] Add the failing web test for the banner text and class.
- [ ] Add the strip above the header and its CSS.
- [ ] Run the web tests; the CSP and single-script tests must still pass.

### Task 5: Wire configuration and deployment

**Files:**
- Modify: `crates/clearcut-server/src/main.rs` (parse six variables, `with_guard`, serve with connect info)
- Modify: `docker-compose.yml` (pass the variables, default off)
- Modify: `.env.example`, `docs/DEPLOY.md`

- [ ] Add the parser and startup errors in the existing style.
- [ ] Serve with `into_make_service_with_connect_info::<SocketAddr>()`.
- [ ] Pass `CLEARCUT_TRUST_PROXY` and the limits through compose.
- [ ] Document each variable and the trusted-proxy condition.

### Task 6: Verify and ship

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo test --workspace` (with the test Postgres)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] Update THREAT-MODEL (threats 3, 7, 11 and open gaps), LAUNCH (section 1, 5, 6) and WORKLOG.
- [ ] Open the PR, watch CI, squash merge.
- [ ] Deploy to the VM, set `CLEARCUT_TRUST_PROXY=1` there, restart, and verify `X-Request-Id`, the headers and the banner live.
