# ADR-0012: The Anthropic adapter

Status: **Proposed** (awaiting review by Kone Tshivhinda) · Date: 2026-10-22 · Deciders: Kone Tshivhinda, Claude
Task P2-13. Builds on [ADR-0011](0011-bring-your-own-anthropic-key.md) (the key, budgets, limits, caching), [ADR-0009](0009-worker-and-provider-boundary.md) (the `ModelProvider` trait), [ADR-0005](0005-model-strategy-escalation.md) (escalation). Facts and sources: [research 06](../research/06-anthropic-api.md). Design only; no adapter code yet.

## What it is
`AnthropicProvider` in `clearcut-anthropic` implements `ModelProvider::complete`. It turns one checked `ModelRequest` into one `POST /v1/messages`, and the answer into a `ModelResponse` or a `ProviderError`. It does **not** choose models (the router, P2-14), limit concurrency (P2-24), or keep budgets (P2-12); it is the translation layer, and it must be impossible to leak a document or a key through it.

```text
ModelRequest ──► checked ──► JSON body ──► POST /v1/messages ──► JSON ──► ModelResponse
 (provider crate)  schema profile  (this ADR §1)   narrow client (ADR-0011)      or ProviderError (§3)
```

## 1. The request we send

```json
{
  "model": "claude-haiku-5-5",
  "max_tokens": 8192,
  "system": [
    { "type": "text",
      "text": "<instructions, South African rules, schema description, examples>",
      "cache_control": { "type": "ephemeral" } }
  ],
  "messages": [
    { "role": "user", "content": [
      { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": "<page image>" } },
      { "type": "text", "text": "<this page's extracted text and the question>" }
    ] }
  ],
  "output_config": {
    "effort": "medium",
    "format": { "type": "json_schema", "schema": { "...": "the profiled schema (§4)" } }
  }
}
```

Headers: `Authorization: Bearer <key>` (sensitive), `anthropic-version: 2023-06-01`, `content-type: application/json`. Always `"stream": false` in v1.

How each `ModelRequest` part maps:

| Our field | Becomes | Notes |
|---|---|---|
| `model` | `model` | any id; the **Models API result** (ADR-0011) says which exist for the key |
| `system` blocks | `system[]` text blocks | a block flagged `cached` gets `cache_control` |
| `content` blocks | one `user` message, same order | an image is a base64 `image` block; text is a `text` block. **We never reorder**; the context-pack builder (P2-16b) must put the page image **before** its text, and anything static (instructions, examples) in `system` **before the cache breakpoint**, so per-page content comes after it |
| cache flag | `cache_control: {type:"ephemeral"}` (plus `"ttl":"1h"` when the request asks for the long cache) | at most 4, already checked by the builder |
| `effort` | `output_config.effort` | `low`, `medium`, `high`; fixed within a rung (changing it breaks the cache) |
| `output_schema` | `output_config.format.schema` after the **profile** (§4) | the same profiled schema every call, or the cache and the compiled grammar are lost |
| `max_output_tokens` | `max_tokens` | includes thinking (§5) |
| `thinking: Adaptive` | the `thinking` field is **omitted** | adaptive is the default on all four models; `display` defaults to `omitted`, so we never receive or keep reasoning text |
| `thinking: Off` | model-dependent, see below | |
| `call` (tenant, job, page, purpose) | **never sent** | logging and budgets only; no `metadata.user_id` either |

Never sent: `tools`, `tool_choice` (forced tool use is unsupported on these models), assistant prefill (refused), `budget_tokens`, `temperature` and other sampling settings, `service_tier`, `speed`, `inference_geo`.

**`Thinking::Off` by model** (from the Models API behaviour documented in research 06): Haiku 5.5 → `{"type":"disabled"}` (only at effort `high` or below, which `Effort` already guarantees); Sonnet 5.5 → `{"type":"between_tools"}`; Opus 5.5 and Fable 5.1 cannot turn thinking off, so the adapter refuses with a clear `Rejected` before sending rather than silently thinking and charging. An unknown model id is treated as "adaptive only".

## 2. The answer we read

```json
{
  "id": "msg_...", "type": "message", "role": "assistant", "model": "claude-haiku-5-5",
  "content": [
    { "type": "thinking", "thinking": "", "signature": "..." },
    { "type": "text", "text": "{\"invoice_number\": \"...\", ...}" }
  ],
  "stop_reason": "end_turn",
  "usage": {
    "input_tokens": 2095, "output_tokens": 503,
    "cache_creation_input_tokens": 2051, "cache_read_input_tokens": 0,
    "cache_creation": { "ephemeral_5m_input_tokens": 2051, "ephemeral_1h_input_tokens": 0 }
  }
}
```

| Part | Rule |
|---|---|
| `content` | thinking and redacted-thinking blocks are **ignored and never stored**; exactly one `text` block is expected and parsed as JSON; none, or more than one, or not JSON → `InvalidOutput` (billed) |
| `stop_reason` `end_turn` | success |
| `max_tokens` | `Truncated { usage }` |
| `refusal` | `Refused { usage }`; `stop_details.category` is read for a fixed reason (never the explanation text) |
| `model_context_window_exceeded`, `pause_turn`, `tool_use`, `stop_sequence`, anything new | `InvalidOutput { usage, reason }` with a fixed reason |
| `usage.input_tokens` | `Usage.input_tokens` (the part **after** the last breakpoint) |
| `usage.output_tokens` | `Usage.output_tokens` (includes thinking) |
| `cache_read_input_tokens` | `Usage.cache_read_tokens` |
| `cache_creation_input_tokens` | `Usage.cache_write_tokens`; the 1-hour part (`cache_creation.ephemeral_1h_input_tokens`) goes to a new `Usage.cache_write_1h_tokens` (§6) because it costs 2x instead of 1.25x |
| `model` | `ModelResponse.model` (the model that actually answered) |
| `request-id` header | `ModelResponse.request_id` (sanitised) |

A usage field that is missing counts as 0; a negative or absurd number is an `InvalidOutput`.

## 3. When it goes wrong

Extends the table in ADR-0011 to the exact `ProviderError` the trait returns.

| Response | `ProviderError` | `failure_kind` | Note |
|---|---|---|---|
| 429 with `retry-after` | `Unavailable { retry_after }` | provider unavailable | no attempt used; limiter (P2-24) slows down |
| 429 `enforced_spend_limit_reached` | `KeyProblem(SpendCap)` + `retry_after` = until 00:00 UTC on the 1st | provider unavailable | the owner must act or wait |
| 400 "reached your specified API usage limits" | `KeyProblem(UserSpendLimit)` | provider unavailable | the owner's own limit |
| 401 | `KeyProblem(Rejected)` | provider unavailable | key marked invalid; no retries |
| 402 | `KeyProblem(Billing)` | provider unavailable | |
| 403, 404 naming the model; 400 Covered-Model refusal | `ModelUnavailable` | needs review (the router skips the rung) | |
| 400 other, 413 | `Rejected(reason)` | needs review | **our bug**: reason is the error type and status and the request id, never Anthropic's message |
| 500, 502, 503, 504, 529, connection errors | retried inline twice with backoff and jitter, then `Unavailable { None }` | provider unavailable | |
| timeout | `Timeout` | transient | |
| stop reasons | see §2 | | |

Every error carries only fixed text plus the sanitised request id. Anthropic's `message` is never copied (it might quote our request).

## 4. The schema profile (structured outputs)
`output_config.format` accepts only a subset of JSON Schema (research 06 §6). The adapter does **not** pass the caller's schema through; it derives a **profile** and fails early, in our code, if it cannot:

- **Closed objects:** every object gets `additionalProperties: false`.
- **Unsupported constraints are removed, and said so:** `minimum`, `maximum`, `exclusiveMinimum/Maximum`, `multipleOf`, `minLength`, `maxLength`, `minItems` above 1, `maxItems`, `uniqueItems`, `pattern`, a `format` outside the supported list (`date`, `date-time`, `time`, `duration`, `email`, `hostname`, `uri`, `ipv4`, `ipv6`, `uuid`), and `$schema`. The removed rule is appended to the field's `description` ("not enforced here: minimum 1") so the model is still told, and `clearcut-core` enforces it afterwards.
- **Refused:** recursive schemas, external `$ref`, `allOf` with `$ref`, non-primitive `enum`s.
- **Counted:** optional properties and union-typed properties are counted (counting every use of a shared definition, the conservative reading), and over **24 optional / 16 union** is refused with the counts.
- The profile is deterministic: the same input schema gives byte-identical output, so the cache and the compiled grammar are reused.

**A risk to decide on now.** The JSON Schema `schemars` generates for core's `Extraction` is almost certainly **over those limits** (many optional fields, and every `Sourced<T>` repeats `locator`). So the model should not be asked for an `Extraction` directly. I propose a separate **model-facing reading shape** (a plain struct in Rust, all fields present, "not on this page" as an empty string or empty list, amounts and quantities as decimal text read by the same exact loader as the gold-set labels, dates as ISO text, one page number per value), converted into an `Extraction` in Rust. That is its own task (P2-13b) and the adapter does not depend on it: the adapter only guarantees that whatever schema it is given is either valid for the API or refused with a precise reason. A test will assert the production schema stays inside the limits.

## 5. Time, size and retries
- **Non-streaming in v1.** A non-streaming request is fine for the output sizes per-page extraction needs; anything that could run over 10 minutes should stream. So `max_output_tokens` above **16 000** is refused (`Rejected`, "needs streaming") until streaming is added. Thinking tokens count toward `max_tokens`, so the router must leave room.
- **Per-request timeout** grows with the output cap: 30 s plus `max_tokens / 40` seconds (16 000 tokens: about 7 minutes). The connect timeout stays 5 s.
- **Answer size** is read up to 2 MiB.
- **Inline retries:** only a 429 whose `retry-after` is 60 s or less, and 5xx/529/connection errors, at most twice, with exponential backoff and jitter; a total wait cap of 2 minutes. Everything else returns to the job layer, which has its own policy (ADR-0008), so retries do not multiply.
- Tests use a paused clock, so no test waits.

## 6. Small, additive changes to `clearcut-provider`
1. `CacheTtl { FiveMinutes, OneHour }` and `RequestBuilder::cache_ttl` (default five minutes); one TTL per request.
2. `Usage.cache_write_1h_tokens` (a part of `cache_write_tokens`), so the cost can be exact. `Add`, `Sum` and `total` are updated (the 1-hour part is not counted twice).
3. `ProviderError::KeyProblem(KeyProblem)` (rejected, no permission, billing, monthly cap, user's spend limit; maps to provider unavailable, never retried quickly) and `ProviderError::ModelUnavailable`. The existing `Auth` stays for the fake.
4. `Thinking::Off`'s doc says what it means ("no up-front thinking; refused where the model cannot").
Each has a test, and `FakeProvider` keeps working.

## 7. Privacy and logging
- **Nothing is logged:** no request or response body, no header, no text. The only things recorded are the model, the usage, the status and the request id.
- Debug output of every adapter type shows sizes only (like `ModelRequest`).
- Reasoning text is never requested (`display` stays `omitted`) and any that arrives is dropped.
- Optionally, in evaluation runs only, `diagnostics.previous_message_id` is sent to get a `cache_miss_reason` back, which tells us exactly why a cache missed; it is off in production.

## 8. How it is tested (no key, no money)
A stub server (as in the key check) answers as told. Golden tests assert the exact request JSON for each row of §1 and the exact error for each row of §3; response tests cover every stop reason and usage shape; a paused-clock test covers retries and the timeout formula; schema-profile tests cover each stripped keyword, each refusal, determinism and the counts; redaction tests check no secret or document text appears in any `Debug` or error; and the same trait-conformance tests run against `FakeProvider` and `AnthropicProvider`. One **live test** (ignored, never in CI) makes a single tiny real call (about R0.003 on Haiku 5.5) and prints the usage and cost; only you run it, with your key.

## 9. What it is not
No concurrency or rate-limit logic (P2-24), no budgets or estimates (P2-12), no model choice or escalation (P2-14), no prompt text or context pack (P2-16b), no Batch API (opt-in, later), no Files API (never).

## Alternatives considered
- *Forced tool use for JSON.* Unsupported on these models.
- *Pass the schema through and let the API complain.* A 400 after a round trip, no counts, and a silently different schema if we edited it ad hoc. The profile makes the difference explicit and testable.
- *Ask the model for an `Extraction` directly.* Likely over the schema limits (§4); a purpose-built reading shape is also easier for the model.
- *Streaming from the start.* More code (event parsing, mid-stream errors) for no gain at per-page output sizes; added when something needs more than 16 000 tokens.
- *The official SDK.* No first-party Rust SDK; the thin client keeps the key, redirects and proxies under our control.

## Questions for Kone
1. **Approve the additive changes to `clearcut-provider`** (§6): cache TTL per request, the 1-hour split in `Usage`, `KeyProblem` and `ModelUnavailable`?
2. **Non-streaming with a 16 000-token output cap in v1**, streaming later if needed?
3. **Strip unsupported schema keywords and say so in the description** (my recommendation), rather than refuse any schema that has one?
4. **A separate model-facing reading shape** instead of asking the model for core's `Extraction` (P2-13b), converted in Rust with the exact decimal loader?
5. **`Thinking::Off` on Opus 5.5 and Fable 5.1 is an error,** not a silent "think anyway"?
6. **One live test** that makes a real call costing about R0.003, run only by you with your key?
