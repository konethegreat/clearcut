# ADR-0011: Bring your own Anthropic key, and how ClearCut uses the API

Status: **Accepted** (approved by Kone Tshivhinda, 2026-10-22, with all six recommended answers below) · Date: 2026-10-22 · Deciders: Kone Tshivhinda, Claude
Builds on [ADR-0005](0005-model-strategy-escalation.md) (escalation) and [ADR-0009](0009-worker-and-provider-boundary.md) (the provider trait). Facts and sources: [research 06](../research/06-anthropic-api.md). Design; the build order is at the end.

## Context
ClearCut will be open source, uses **only the Anthropic API**, and the project has no money for an API key of its own. So **whoever runs ClearCut supplies their own key**, and every call is billed to them. Three things follow:

1. **It is their money.** A bug that loops, an escalation that always climbs to the top rung, or a re-upload of a large PDF costs a real person real rand. Cost control is a safety feature, not an optimisation.
2. **Their key has its own limits.** Rate limits, spend caps and tiers belong to *their* organisation and are shared with everything else they do. A new organisation may be in a low "Evaluation" tier. ClearCut must be a polite tenant and must not assume throughput.
3. **A secret now lives in our process.** An open-source project holding users' API keys must make it very hard to leak one.

The documentation also shows facts that change the design: forced tool use is unsupported on the current models (so structured outputs, not a forced tool, give us JSON); thinking is always on for Opus 5.5 and Fable 5.1, so effort is the cost control; changing effort, images or the schema invalidates the prompt cache; a spend-cap 429 carries no `retry-after` and must not be retried; and **Fable 5.1 requires 30-day retention** and is refused to organisations under zero data retention.

## Proposal in brief
1. **Anthropic only, bring your own key. No operator key, no pooled keys.** The `ModelProvider` trait stays (for the test fake and clean boundaries); only an Anthropic adapter ships.
2. **Two ways to give a key.** (A) *Self-hosted, one owner:* `ANTHROPIC_API_KEY`, or `ANTHROPIC_API_KEY_FILE` pointing at a file. (B) *Multi-tenant service:* each tenant stores their own key, encrypted at rest, through a write-only endpoint. **Build A first; B follows** once the key store has had its own review.
3. **The key is handled like a password:** a secret type that cannot be printed, zeroed on drop, sent only to `api.anthropic.com` over TLS, never in a log, an error, a URL, a job payload, the worker, or a report.
4. **A key is checked without spending tokens:** `GET /v1/models` on setup (401 means refuse; 200 gives the real list of usable models and their capabilities).
5. **Cheapest first, hard budgets.** Haiku 5.5 is the default rung. Every tenant has a **per-document cap and a monthly budget** (in rand, at a configurable USD rate, default R18), checked *before* each call and each escalation. A cost estimate and a dry run are available before a large upload.
6. **Rate-limit aware.** An adaptive per-key limiter starts small and ramps gradually, reads `anthropic-ratelimit-*` headers, honours `retry-after`, and treats a spend-cap 429, a 401, a 402 and a user-set spend limit as "the key's owner must act", not as transient errors.
7. **One cached static prefix per task**, with an explicit breakpoint, a stable shape, and a "first call warms the cache, then the rest" rule.
8. **Structured outputs** (`output_config.format`) for extraction, with a provider-facing schema profile; everything else validated by `clearcut-core`.
9. **Fable 5.1 is off by default** (Covered Model: 30-day retention, lower token limits, highest price). A tenant can switch it on.
10. **Tests never need a key.** Live tests exist but are ignored unless a contributor sets their own key, and CI never has one.

## Details

### Where the key lives (privacy rule: the worker never sees it)
- **Mode A:** read once at start-up from the environment or a file, into a secret type; the process refuses to start with a key present in a command-line argument (visible to other users in a process list).
- **Mode B:** per-tenant, **envelope-encrypted** (AES-256-GCM, a fresh nonce per key, the tenant id as associated data so a ciphertext cannot be moved to another tenant), the master key from the environment or a secret manager and never in the database. Endpoints: set/replace, delete, and a status that returns only the last four characters and whether the last check passed. **The value is never readable again.** Set, rotate and delete are audit-logged (without the value). A new migration and a security review of its own, shown to Kone first (as with every schema change).
- **In memory:** a `SecretString`-style wrapper (zeroed on drop, `Debug` prints `[redacted]`). It is passed to the HTTP client only to build the `x-api-key` / `Authorization` header, marked sensitive (the same way `HttpWorker` already treats its token).
- **The base URL is operator configuration, never tenant input,** so a tenant cannot point their key (or someone else's) at a server they control. Redirects are not followed and no environment proxy is used (the same hygiene as `HttpWorker`).
- **Not sent to the Python worker, ever** (ADR-0009): it only reads files.
- The user is told how to create a **safe key**: a dedicated workspace, a monthly spend limit set in the Console, and an expiry (Anthropic offers 3 hours to 30 days or custom). Disable is reversible; delete is not. A `docs/BYOK.md` will say so in plain words.

### Money safety
- **Prices** live in a table keyed by model id, with the date they were read; the Models API and the pricing page are the source. Haiku 5.5's price **jumps for prompts over 100 000 tokens**, so the estimator must know that.
- **Estimate first:** page count x tokens per page (measured on real runs, research 06 §10 gives a starting guess), per rung. The token-counting endpoint can give exact counts (documentation not yet read; verify it is free before relying on it).
- **Budgets:** `per_document_cap` and `monthly_budget`, both required to have a value (safe defaults, tenant-visible, changeable). A call or an escalation that would breach either does not happen: the job ends `needs_review` with the reason "budget", never silently. Usage per call (input, output, cache write, cache read) is stored with the job, so the monthly total is the sum of facts, not an estimate.
- **Bulk uploads** can use the **Batch API** (50% off, results within 24 hours, usually under 1) as an opt-in. It is *not* zero-data-retention eligible, so it is off unless the tenant turns it on.
- **A loop cannot run away:** the number of model calls per document is capped, independent of budget.

### Rate limits and errors
- **Limiter:** one per (key, model bucket). Haiku/Sonnet/Opus 5.5 are separate rows; Fable has its own smaller one. Start at concurrency 2, add one after a run of successes, halve on a 429 (additive increase, multiplicative decrease). Never jump traffic suddenly (acceleration limits).
- **Headers:** after each response read `anthropic-ratelimit-input-tokens-remaining` and the requests equivalents; if headroom is low, wait for the `-reset` time rather than hit the wall. Cached reads do not count toward input tokens per minute, so caching also raises real throughput.
- **Mapping to the job queue's failure kinds (ADR-0008):**

| Response | Kind | What happens |
|---|---|---|
| 429 with `retry-after` | provider unavailable (no attempt used) | wait that long, lower concurrency |
| 429, `enforced_spend_limit_reached` (no `retry-after`) | provider unavailable, long delay | tenant shown "monthly cap reached; resumes 00:00 UTC on the 1st"; jobs wait up to their 48 h deadline, then `needs_review` |
| 400 "reached your specified API usage limits" | provider unavailable, long delay | tenant shown "your own spend limit was reached" |
| 401 | provider unavailable | key marked invalid; tenant told; no retries |
| 402 | provider unavailable | tenant told to check billing |
| 403 / 404 on a model | rung unavailable | skip that rung, record why |
| 400 on Fable (retention) | rung unavailable | switch Fable off for this tenant, tell them |
| 413 | needs review (document) | shrink or reject, never loop |
| 500, 504, 529 | transient | exponential backoff with jitter |
| `refusal` stop reason (200) | needs review | billed; do not retry the same input |
| `max_tokens` stop reason | transient, once | retry once with a larger limit, then needs review |

- **Retries are not multiplied:** the client retries only what is certainly worth it inline (a `retry-after` up to 60 s, or a 5xx/529, at most twice); everything else goes to the job layer's own policy.
- Every response's `request-id` is stored with the call, so a user can quote it to Anthropic.

### Prompt caching
- **Layout:** tools, then system (instructions, South African rules, the schema description, few-shot examples), with **one explicit breakpoint at the end of the static part**; the document page (image and text) and the question come after it. All four of our models cache from 512 tokens, which the instructions and rules exceed.
- **Stability rules (a test per rule):** no timestamps or ids in the prefix; fixed JSON key order; one effort level per rung and the same schema every call (changing effort, schema or images before the breakpoint invalidates the cache); the images come after the breakpoint.
- **First call warms:** for a document, send the first page, wait for its response to begin, then fan out the rest, because an entry only exists once the first response starts.
- **TTL:** 5 minutes (write 1.25x) for interactive uploads where pages follow within seconds; **1 hour** (write 2x) for batches and bulk uploads whose calls are minutes apart. A read costs 10% (5% on Sonnet 5.5 and Opus 5.5, 2.5% on Fable 5.1) of the input price. A prefix that is never read again is a loss (1.25x), so a single-page document on its own is not cached.
- **Measure it:** each call stores `cache_creation_input_tokens` and `cache_read_input_tokens`, and the evaluation report shows the hit rate and what caching saved.

### Escalation, effort and thinking
- The ladder is unchanged (Haiku 5.5 → Sonnet 5.5 → Opus 5.5 → Fable 5.1 → human), triggered by a failed deterministic check, low confidence, a refusal, or a schema failure (ADR-0005). It is now also **bounded by the budget** and by what the key can use (the Models API list).
- **Thinking:** omit `thinking` (adaptive); never send `budget_tokens`, `disabled` (Opus 5.5 and Fable 5.1 refuse it) or `enabled`. **Effort is the dial**, set explicitly per rung and fixed within it: start at each model's default and let the gold set (P2-09/P2-10) say whether `low` holds quality on Haiku.
- **`max_tokens` includes thinking.** Set it with room (and review it as measured usage comes in); long outputs stream.
- **No prefill, no forced tool choice.** The JSON comes from structured outputs.

### Structured outputs
The core `Extraction` JSON Schema is generated by `schemars`, but the API accepts a subset (objects closed with `additionalProperties: false`, no recursion, no `minimum`/`maximum`/length limits, at most 24 optional and 16 union-typed parameters). We add a **provider profile**: a derived schema that satisfies those limits (money as integer minor units is fine), with every range and consistency rule left to `validate`. A test checks the profile against the documented limits, and the profile is **constant** (a change invalidates the cache and costs a compile delay on first use).

### Privacy with a user's own key
With a user's key, the data agreement is between that user and Anthropic. By default the API does not keep prompts and outputs, except for Covered Models (Fable 5.1: 30 days); the Batch and Files APIs do store data. If ClearCut is **self-hosted by the owner of the invoices**, this is simple. If someone **hosts ClearCut for other people**, they handle those people's invoices and become the responsible party under POPIA: that needs the privacy notice and consent text (P2-19) and an Information Officer. The README must say which situation each deployment is in.

### Open-source hygiene
- No key in the repository, ever: tests use the fake; examples use `sk-ant-...` placeholders. A small test scans tracked files for the key prefix followed by a plausible body, alongside the document guard (GitGuardian already runs).
- Live tests are `#[ignore]` and read `ANTHROPIC_API_KEY` from the contributor's environment; CI never has a key, and a live run says what it cost.
- Contributors are told the cost of a live run (a few cents of rand on Haiku 5.5).

## What gets built, in order (each its own small PR, tests first)
1. **P2-23 Key handling, mode A:** the secret type, loading from environment or file, refusing a key on the command line, validation through the Models API, redaction tests.
2. **P2-13 Anthropic adapter** behind the provider trait: request building (structured outputs, cache breakpoint, effort), usage parsing (including cache fields), error mapping to failure kinds, with a stub server for tests and an ignored live test.
3. **P2-24 Rate-limit-aware client:** the adaptive limiter, header reading, retry-after, and the table above.
4. **P2-12 Cost guardrails:** prices, estimates in rand, per-document cap, monthly budget, call cap.
5. **P2-14/15/16 Escalation, effort, caching** wired through the router.
6. **P2-23b Key store, mode B** (its own ADR-sized review).

## Alternatives considered
- *A shared project key paid by the project.* Rejected: there is no money, and it would be an unbounded liability and a single point of abuse.
- *The user's key in the browser or SDK only.* Rejected: ClearCut runs jobs in the background for hours (retries, escalation, batch), which needs the key server-side.
- *Support other providers.* Rejected for now (decided in ADR-0005): one provider keeps the privacy story, the testing and the cost model honest.
- *Use the official SDK.* Rust has no first-party SDK; the adapter is a thin HTTP client behind the trait (as `HttpWorker` is), which also keeps the base URL and redirects under our control.
- *Send whole PDFs to the model.* Possible for clean text PDFs (up to 600 pages, text plus page images), but it hands the model the raw file, gives less per-page control for escalation, and fails for glyph-scrambled files (P2-20). Revisit as an optimisation.

## Decisions (Kone's answers to the six questions)
1. **Mode A first** (environment or file key); mode B (encrypted per-tenant keys) later, with its own review.
2. **Fable 5.1 is off by default**, switchable per tenant.
3. **Default budgets: R5 per document and R100 per month**, both visible and changeable.
4. **The Batch API is an opt-in** for bulk uploads (cheaper, but stores data on Anthropic's side).
5. **Cost is shown in rand** at a configurable rate, default R18 per USD.
6. **Docs still to read before the adapter** (vision, token counting, streaming, thinking, Files): read as part of building P2-23 and P2-13, and recorded in research 06.
