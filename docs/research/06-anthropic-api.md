# Research 06: the Anthropic API, as it applies to ClearCut

_Kone Tshivhinda & Claude. Read from Anthropic's official documentation on **22 Oct 2026**. Re-read the linked page before relying on any number: limits, prices and model lines change._

Why this exists: ClearCut will use **only** the Anthropic API, and users will bring **their own API key** (ADR-0011). That makes the API's rules (rate limits, caching, errors, retention, prices) design inputs, not details. Prices below are USD as published; ZAR figures use the project's assumption of **R18 per USD**.

Sources (all on `platform.claude.com/docs/en/`): `api/rate-limits`, `build-with-claude/prompt-caching`, `about-claude/models/overview`, `about-claude/pricing`, `api/errors`, `build-with-claude/effort`, `build-with-claude/pdf-support`, `build-with-claude/structured-outputs`, `build-with-claude/batch-processing`, `manage-claude/authentication`, `manage-claude/api-and-data-retention`.

## 1. The models we use

| | Fable 5.1 | Opus 5.5 | Sonnet 5.5 | Haiku 5.5 |
|---|---|---|---|---|
| API id | `claude-fable-5-1` | `claude-opus-5-5` | `claude-sonnet-5-5` | `claude-haiku-5-5` |
| Input / output per MTok | $10 / $50 | $4 / $20 | $2 / $10 | $0.10 / $0.50 (prompts up to 100k tokens), **$0.50 / $2.50 above 100k** |
| In rand per MTok (R18/$) | R180 / R900 | R72 / R360 | R36 / R180 | R1.80 / R9.00 (above 100k: R9 / R45) |
| Cache read (of base input) | 0.025x | 0.05x | 0.05x | 0.1x |
| Context / max output | 1M / 128K | 1M / 128K | 1M / 128K | 1M / 128K |
| Thinking | adaptive, **always on** | adaptive, **always on** | adaptive (`disabled` refused; `between_tools` is the lowest) | adaptive (`disabled` accepted at `high` effort or below) |
| Default effort | high | medium | high | medium |
| Minimum cacheable prompt | 512 tokens | 512 | 512 | 512 |
| Start-tier limit (RPM / ITPM / OTPM) | 1 000 / **500 000** / 100 000 | 1 000 / 2 000 000 / 400 000 | same as Opus | same as Opus |
| Retirement | not before 1 Sep 2027 | 22 Sep 2027 | 28 Sep 2027 | 7 Oct 2027 |

- Only **Haiku 5.5** is priced higher for long prompts; the others charge the same per token up to 1M.
- Every model id is a **pinned snapshot**, so ids can live in configuration without surprise drift.
- The **Models API** returns each model's `max_input_tokens`, `max_tokens` and a `capabilities` object, so ClearCut can discover what a key can use instead of hard-coding it.
- Not supported on these models: **assistant prefill** (400), `thinking: enabled` with a budget (400), and **forced tool use** (`tool_choice` `any` or `tool`; 400). Use **structured outputs** or `auto` with strict tools. This matters: our extraction call cannot force a tool.
- **Fable 5.1 is a "Covered Model"**: it requires **30-day data retention** and is **not available under zero data retention (ZDR)** unless Anthropic authorises it. From an organisation whose retention setting does not meet that, a request returns **400 `invalid_request_error`**. So the top rung of the escalation ladder can be unavailable to a user, and ClearCut must treat it as optional.

## 2. Rate limits (per organisation, shared by everything the user runs)

- Three measures per model class: requests per minute (RPM), **input** tokens per minute (ITPM), **output** tokens per minute (OTPM). Token-bucket: capacity refills continuously. Bursts can trip a limit well below the per-minute figure (60 RPM may be enforced as 1 per second).
- **Cached reads do not count toward ITPM** (on these models). Uncached input and cache writes do. Output is counted as produced; `max_tokens` does not count against OTPM.
- Tiers: new organisations may begin in an **Evaluation tier** with limits below the Start tier, rising with history. Start/Build/Scale monthly spend caps: $500 / $1 000 / $200 000. Start-tier limits for Haiku/Sonnet/Opus 5.5: 1 000 RPM, 2M ITPM, 400K OTPM. **Fable is lower**: 500K ITPM, 100K OTPM, and one combined bucket for Fable 5 and 5.1.
- **Acceleration limits**: a sharp rise in traffic can cause 429s even under the limits. Ramp up gradually and keep usage steady.
- Headers on every response: `anthropic-ratelimit-{requests,input-tokens,output-tokens,tokens}-{limit,remaining,reset}` (reset is RFC 3339), and `retry-after` (seconds) on a 429. `anthropic-workspace-id` names the workspace the key resolved to. `request-id` identifies the request (quote it to support).
- **Two 429s look the same and are not.** A rate limit 429 carries `retry-after`: wait that long. A **monthly spend cap** 429 has **no** `retry-after`, carries `error.details.error_code: "enforced_spend_limit_reached"`, and keeps failing until 00:00 UTC on the first of the next month. Retrying (including an SDK's automatic retries) is futile.
- A spend limit the **user set** themselves (organisation or workspace) returns **400 `invalid_request_error`** starting "You have reached your specified API usage limits".
- Batch API has its own limits (Start tier: 1 000 RPM, 200 000 requests in the queue).

## 3. Errors and what to do

| Status | Type | Meaning for ClearCut |
|---|---|---|
| 400 | `invalid_request_error` | our request is wrong, or the user's own spend limit was hit, or a Covered Model is not allowed for the org: read the message; do not retry blindly |
| 401 | `authentication_error` | the key is malformed, revoked or expired: stop, tell the tenant, never retry |
| 402 | `billing_error` | payment problem on the user's account: pause and tell the tenant |
| 403 | `permission_error` | the key cannot use this resource/model |
| 404 | `not_found_error` | unknown model or resource |
| 413 | `request_too_large` | over 32 MB (Messages), 256 MB (Batch), 500 MB (Files): shrink |
| 429 | `rate_limit_error` | see section 2: honour `retry-after`; **spend-cap 429 has none** |
| 500 | `api_error` | retry with exponential backoff |
| 504 | `timeout_error` | stream, or use the Batch API for long work |
| 529 | `overloaded_error` | transient: back off |

- Errors are JSON `{type:"error", error:{type, message}, request_id}`. A **streaming** response can fail after a 200; handle error events.
- Non-streaming requests expected to exceed 10 minutes should stream or go through batches.
- Official SDKs retry transient failures twice by default, honouring `retry-after`. ClearCut's Rust client must do the equivalent itself and keep it consistent with the job queue's retry policy (ADR-0008) so the two do not multiply.

## 4. Prompt caching

- Enable with `cache_control: {type: "ephemeral"}` on a block (explicit, up to **4 breakpoints**) or at the top level (automatic). Order is **tools, then system, then messages**; a change at one level invalidates it and everything after.
- **TTL**: 5 minutes (default) or 1 hour (`ttl: "1h"`). Write cost 1.25x (5 min) or 2x (1 h) the base input price; **reads 0.1x** (0.05x on Opus 5.5 and Sonnet 5.5, 0.025x on Fable 5.1). A read refreshes a 5-minute entry free. The lifetime counts from the start of the request.
- Minimum cacheable prefix on all four of our models: **512 tokens**. Shorter prompts are silently not cached; the response's `usage` shows `cache_creation_input_tokens` and `cache_read_input_tokens` (both 0 means not cached). `total input = cache_read + cache_creation + input_tokens`, where `input_tokens` counts only what comes **after** the last breakpoint.
- A hit needs an **exact** match of everything up to the breakpoint, in the same organisation and workspace. The lookback is 20 blocks. Put the breakpoint on the last block that is identical across requests; **never** on a block with a timestamp or per-request content.
- **What invalidates**: changing tool definitions; adding or removing **images** anywhere (invalidates the messages level); changing `tool_choice`; changing thinking parameters or a non-default **effort** (model-specific); changing `output_config.format` (structured outputs); unstable JSON key order in tool inputs.
- **Concurrency**: an entry is usable only **after the first response starts**. For a burst of parallel calls that share a prefix, send one, wait for it to begin responding, then send the rest.
- Pre-warming is possible (`max_tokens: 0`) but not with streaming, thinking, structured outputs, forced tool choice or batches.
- Per-message effort changes (beta header `mid-conversation-output-config-2026-07-01`) keep the cache; a top-level change does not.
- Caching does not change output content.

## 5. Effort and thinking

- `output_config.effort`: `low`, `medium`, `high`, `xhigh`, `max`. It affects **all** output tokens (thinking, text, tool calls). Lower effort means fewer tokens and a little less capability. Effort is a signal, not a budget.
- Haiku 5.5 guidance: start at `medium`, use `low` for simple high-volume work, `high` for strict instruction following, and compare `xhigh`/`max` with Sonnet 5.5 before paying for them.
- **Thinking tokens count toward `max_tokens` and are billed as output** even if not shown. Leave room for them; set `max_tokens` generously at higher effort.
- The old way (`budget_tokens`) is refused. Use `thinking: {type: "adaptive"}` (or omit it) plus effort.
- Thinking blocks must be passed back **unchanged** in tool loops; editing or dropping them is a 400.

## 6. Structured outputs

- `output_config.format = {type: "json_schema", schema}` makes the response conform. Works on all four models.
- Limits: objects must set `additionalProperties: false`; **no recursion**; no `minimum`, `maximum`, `minLength`, `maxLength`, `multipleOf`; array `minItems` only 0 or 1; `$ref` only internal; at most 24 optional parameters and 16 union-typed parameters per request; a very large grammar returns 400 "too complex". The first request with a new schema pays a compile delay; compiled grammars are cached 24 h.
- `stop_reason: "refusal"` (200, billed) and `max_tokens` (truncated, may not match the schema) both need handling. Enum casing can drift in the first letter: compare case-insensitively or avoid enums that differ by case.
- It adds a system prompt (input tokens) and **changing the schema invalidates the prompt cache**: keep the schema constant.
- Consequence: the core `Extraction` JSON Schema (via `schemars`) needs a **provider-facing profile** that respects these limits, with everything else validated by `clearcut-core::validate` after the fact.

## 7. PDFs and images

- A PDF can be sent whole (base64, URL or Files API). Limits: 32 MB request, **600 pages** (100 when the context window is under 1M), standard PDF only (no password/encryption). Each page is given to the model as **both extracted text and a page image**; text costs about 1 500 to 3 000 tokens per page plus the image cost; no extra PDF fee.
- Dense or heavily graphic pages can exhaust the context before the page limit: split into sections.
- ClearCut's own design (ADR-0009) keeps the Python worker as the only thing that touches untrusted files and sends the model page images and text it has extracted. Sending the PDF directly is a possible simplification for text PDFs, but not for glyph-scrambled ones (see below), and it would give the model the raw file.
- **Not yet read** (read before building the adapter): the vision page (image size and token rules), token counting, streaming, the Files API, and the thinking pages.

## 8. Data retention and privacy (POPIA)

- By default **conversation content (prompts and outputs) is not retained** on the Claude API, except for **Covered Models** (Fable 5.1 among ours), which require 30-day retention. Retained data is never used for training without express permission.
- ZDR is an arrangement with Anthropic; features that must store state (the **Batch API** and the **Files API**) are **not** ZDR-eligible: using them steps outside the arrangement for that data. Flagged content may be kept up to 2 years even under ZDR.
- Effect on the open item "verify Anthropic retention/ZDR terms" (CLAUDE.md): partly answered. Standard API calls to Haiku/Sonnet/Opus 5.5 do not retain content by default; Fable 5.1 does for 30 days; Batch and Files do store. The privacy notice and the s72 reasoning (data leaves South Africa) still need a person's review (P2-19).

## 9. Keys, workspaces and who pays

- Keys look like `sk-ant-api...`, sent as `Authorization: Bearer` (or legacy `x-api-key`), with the `anthropic-version: 2023-06-01` header. Types: personal, service-account, legacy workspace. A key can be scoped to one workspace (otherwise `anthropic-workspace-id` is required).
- A key can be given an **expiration** (3 hours, 1 day, 7 days, 30 days, custom, or never). An expired key returns 401.
- Anthropic advises: store keys in a secrets manager, rotate, disable or delete a leaked one. **Disable is reversible; delete is not.**
- Limits and spend caps are **per organisation** (workspaces can be given lower ones). A user can set their own spend limit in the Console below their tier's cap.
- Consequence for bring-your-own-key: a user's key shares its organisation's limits and budget with everything else they do. ClearCut must be a polite tenant of that budget.

## 10. What a document costs (a planning estimate, not a promise)

Assuming about 3 000 input tokens and 1 000 output tokens per page (text, a page image, instructions, and a JSON answer; thinking adds output tokens), at R18/USD:

| Model | Per page | A 10-page invoice |
|---|---|---|
| Haiku 5.5 | about R0.015 | about R0.15 |
| Sonnet 5.5 | about R0.29 | about R2.90 |
| Opus 5.5 | about R0.58 | about R5.80 |
| Fable 5.1 | about R1.44 | about R14.40 |

The Batch API takes 50% off both input and output (a document is not urgent). Cache reads cut the repeated instruction prefix to 5% to 10% of its price. A full 200-document gold-set run on Haiku 5.5 alone is therefore in the order of tens of rand, which is why the cheap tier should be the default and escalation the exception. Measure real token counts with the first live run before trusting these numbers.
