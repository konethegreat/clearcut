# Threat model and security review

_Kone Tshivhinda & Claude, 27 Oct 2026 (P8-05). Written from what is in the code and its tests, not from intentions: each defence names where it is built and tested. It is an **internal review, not an external audit**; the last section says what an outside reviewer should be given._

## 1. What is being protected
| Asset | Why it matters |
|---|---|
| **Invoice content** (the files, what was read, the events, the narration) | Other people's personal information and commercial secrets (POPIA) |
| **Each account's Anthropic key** | It is their money, and a leaked key can be used by anyone |
| **ClearCut API keys** | They open an account's documents and keys |
| **The master key** | With the database it opens every stored Anthropic key |
| **The audit log** | The only account of who did what |
| **Availability** | Documents must end `done` or `needs_review`, and one account must not starve another |

## 2. Who might attack, and how they get in
- **Anyone on the internet**: can reach only `/health`, the static web page and the API (which refuses everything without a key).
- **A signed-in user of one account** trying to reach another's data, or to spend someone else's money.
- **The author of a document** (an uploaded invoice is untrusted input in every sense): a malformed file, hidden text, text that tries to instruct the model or an agent.
- **An AI agent that has been talked into something** by such a document (the MCP server).
- **A dependency or an image** that has been tampered with or has a flaw.
- **The host or a backup** being stolen; **an operator's mistake**.

```text
 browser / agent ──HTTPS──> Caddy ──> server ──> Postgres  (restricted role, row-level security)
 (ClearCut key)                         │  ├──> file volume (content-addressed, per tenant)
                                        │  ├──> worker   (reads untrusted files; no internet, no database)
                                        │  └──> Anthropic (the account's own key; the only outbound call)
```
Each arrow is a trust boundary. The worker is the most exposed component (it parses hostile files), so it is the most confined.

## 3. Threats and what stops them
| # | Threat | What stops it (and where it is tested) | What is left |
|---|---|---|---|
| 1 | **One account reads or changes another's data** | Row-level security on every table, the service connects as a restricted role that cannot bypass it and **refuses to start** as the owner (ADR-0007; the store tests run as that role with a second tenant and try to cross); security-definer functions check the tenant themselves | A flaw in a new query that is not covered by a cross-tenant test: every new table needs one |
| 2 | **A ClearCut key is guessed, stolen or leaks** | 256-bit random secret; only a SHA-256 of it is stored and the application role cannot read it; compared as hashes of hashes in the database; every failure gives an identical `401`; at most a year's life; revocable at once; never logged (header marked sensitive); the web page keeps it in memory only | Nothing slows repeated guessing at the edge (it is not feasible against 256 bits); a stolen laptop with a pasted key |
| 3 | **A malicious file exploits a parser** | Rust never parses a document; the Python worker does, in a container with no network, no route to the database, a read-only file system, no capabilities, a non-root user and each child process scrubbed, limited and cleaned up (ADR-0009, `docker-compose.yml`; contract tests in CI); the media type is sniffed at intake; size and page limits | A zero-day in PDFium, Pillow or Tesseract is **contained to the worker**, not fixed by it: keep the image updated |
| 4 | **Prompt injection in a document** | A document is data, never instructions (ADR-0014). The model has **no tools** and answers in a fixed schema, so an injection can at worst change what it *reads*. Instruction-like text is flagged and such a page cannot end `done` without a person; every amount must occur in the page's own text; the arithmetic must hold to the cent; narration is template-first with a number guard; a figure the model made up goes to review (pipeline tests: orders in the text, unconfirmed figures) | A document can still make a model **read a number wrongly in a consistent way** (a convincingly altered total). Human review of important figures and the notice on every export are the answer; the gold set (not yet built) will measure how often |
| 5 | **Anthropic keys stolen** | Sealed with AES-256-GCM before they reach the database, tenant bound in so they cannot be moved, master key from the environment and never in the database, write-only API, the application role has no privilege on the table, never in a log, error, URL, job or the worker (ADR-0019; sentinel tests) | The database **and** the master key together; a compromised service process (it holds the master key by design). Hence: dedicated keys with a spend limit and an expiry |
| 6 | **Someone's money is drained** (a bug, a loop, an abusive user) | Per-document cap (R5) and monthly budget (R100) checked **before** each call against the worst case of the next one; a cap on calls per page and document; at most one job per account at a time, so budgets cannot race; a refused or missing key waits instead of retrying; the user's own Anthropic spend limit is the one that cannot be bypassed (ADR-0018, 0011; router and pipeline tests) | A tenant can spend its own R100 a month on garbage; the server's own key (`shared` mode) must never be used for other people: use `tenant-only` |
| 7 | **XSS or CSRF in the web page** | Everything from a document goes in with `textContent` (a test fails if `innerHTML` appears); `script-src 'self'`, no inline script, no framing; the key is a bearer header from page memory, never a cookie, so there is nothing for a forged request to ride on; HSTS from Caddy | A bug in the page's own script while a key is in memory |
| 8 | **A malicious export** (a formula in Excel or CSV, markup in HTML) | HTML escapes every character that matters; Excel cells are plain strings, never formulas; CSV neutralises a leading `= + - @`; PDF and Word carry text only (export tests with hostile text) | A viewer with its own flaw |
| 9 | **Denying who did what** | The audit log is append-only (a trigger refuses change); uploads, reviews, deletions, key changes record the key that did them; a deletion leaves a content-free record | A review made by an AI agent is recorded against its key: ClearCut cannot tell an agent's "yes" from a person's |
| 10 | **Data left after a deletion** | One function erases the document's rows and the file (cannot race a live job); the cost log keeps no content | Older **backups** keep it until they expire (say so in the privacy notice); a tiny race between a delete and an identical re-upload (LAUNCH known limits) |
| 11 | **Denial of service** | Upload size limit enforced as the bytes arrive; the worker caps pages and bytes; per-account concurrency; Caddy limits the request body; the job queue retries without losing work | **No per-account storage quota or request rate limit**: a signed-in user could fill the disk. On the list below |
| 12 | **Supply chain** | `Cargo.lock` and `package-lock.json` committed; Dependabot; **`cargo audit`, `npm audit` and `pip-audit` in CI** (new); a licence check keeps GPL/LGPL/AGPL code out of the worker; GitGuardian on every PR; images build in CI and run non-root | GitHub Actions are pinned by version tag, not commit; base images by tag (not digest); see the exception in section 5 |
| 13 | **Server-side request forgery** | The server makes **one** kind of outbound call, to Anthropic's address from the operator's settings; no URL ever comes from a user or a document; the MCP client follows no redirects and uses no proxy | None known |
| 14 | **An operator's mistake** | Refuses to start as the owner role; `tenant-only` refuses to start without a master key; the compose file publishes the page on `127.0.0.1` only (with HTTPS on, only Caddy's ports 80 and 443 are public); passwords required by name | A weak password or an open firewall: `docs/DEPLOY.md` says what to check |
| 15 | **An AI agent is made to misuse its tools** | ADR-0020: uploads only from one folder and only real documents (the path is checked as text first, so a Windows network path cannot leak credentials); fixed export names; document text labelled as data; an explicit yes for confirm and delete; no tool touches the Anthropic key | An agent can be talked into saying yes; use a key with only the scopes it needs |

## 4. What reviews have actually found
Every item was found by a review or by trying the system, and fixed. They are the best evidence of what kind of mistake gets made here.
1. A budget **under-reserved** the next call (document text counted too cheaply). Fixed; the worst case is now what is reserved.
2. A **timeout or a cut connection was free**: a loop of them could spend past the cap unseen. Now charged at the worst case.
3. The adapter **retried a cut connection inline**, spending twice. Removed.
4. A page with **instruction-like text could end `done`**. A person now looks first.
5. The **worker shared a network with a database that needed no password**. Now alone with the server, and the database needs passwords.
6. A Windows checkout made **migrations CRLF**, so a database refused to start. Pinned to LF.
7. **Erasing a document could race a job** that was queued in the same instant. The database function now refuses atomically.
8. An agent-supplied **Windows network path** would have made the computer connect out, with the person's credentials, while it was being resolved. Checked as text first.
9. **`ctutils`** (a cryptography helper) had non-constant-time comparisons: upgraded (found by the new `cargo audit`).

## 5. Dependency audit (27 Oct 2026)
- **Rust:** `cargo audit` found 4 advisories: `ctutils` (fixed by upgrading) and three on `zip` 8.6 (`RUSTSEC-2026-0367`, `0368`, `0369`). The `zip` ones are about **extracting** archives; it arrives through the Word library and **ClearCut only ever writes ZIP files, never extracts one**, so they cannot be triggered by our code. They are listed with that reason in `.cargo/audit.toml` and the audit fails again on any *new* advisory. **Revisit when the Word library moves to `zip` 9.** The PDF library's unused HTML layout (it pulled in an unmaintained `bincode`) was switched off, which also shrank the dependency tree.
- **npm:** 0 vulnerabilities. **Python worker:** none known.
- The audit now runs on every pull request that touches a lock file and **every week**, so a new advisory is seen without anyone having to look.

## 6. Open gaps, in the order worth closing
1. **A per-account storage quota and a request rate limit** (threat 11).
2. **Logging each model call as it happens**, so a crash mid-page cannot lose a billed call from the spend total.
3. **A second person approving a review**, if a customer needs separation of duties (threat 9).
4. **Pinning GitHub Actions and base images by digest** (threat 12).
5. **Automated backups and a tested restore** (DEPLOY.md describes them by hand).
6. **Rotating the master key and the database passwords** without re-entering every account's key.

## 7. For an outside reviewer
A penetration test or code review would be most useful on: the **tenant-isolation migrations and functions** (`crates/clearcut-store/migrations`, 0003 to 0010); the **worker's confinement** (`services/worker`, `docker-compose.yml`) with hostile PDFs and images; the **key store** (`clearcut-anthropic/src/vault`, ADR-0019); the **web page** (`crates/clearcut-server/src/web`); the **MCP server** (`clearcut-mcp`, ADR-0020); and the **deployment** (`docs/DEPLOY.md`, `deploy/Caddyfile`). Give them this file, `docs/ARCHITECTURE.md` and the ADRs, a running stack and two accounts to try to cross.
