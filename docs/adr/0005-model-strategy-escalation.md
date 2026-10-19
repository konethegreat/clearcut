# ADR-0005: Model provider, escalation, thinking and memory

Status: Accepted (v1) · Date: 2026-10-19 · Deciders: Kone Tshivhinda, Claude
Builds on [ADR-0004](0004-ingestion-and-narration-pipeline.md).

## Decisions from Kone
- Provider: **Anthropic API** (one API key).
- **Model escalation**: start with smaller models and escalate when they fail.
- **High thinking** and "boosted memory", so that every task runs to completion.
- Exports: the popular formats everyday people use, each well structured.
- Accuracy targets: undecided → proposed in [research 05 §7](../research/05-ingestion-narration-export.md).

## Decision
### 1. One provider behind an interface
Anthropic only for v1, behind the provider trait from ADR-0004, so a self-hosted or in-region model can be added later if POPIA or a customer requires it.

### 2. Escalation ladder (validation-driven)
| Tier | Model | Role |
|---|---|---|
| 1 | Claude Haiku 5.5 | First pass on clean text-layer pages and simple layouts |
| 2 | Claude Sonnet 5.5 | Default retry; messy scans, tables, unknown suppliers |
| 3 | Claude Opus 5.5 | Hard pages, conflicting totals, handwriting |
| 4 | Claude Fable 5.1 | Last resort before human review (priciest) |

A page **escalates** (never the whole document unless needed) when any of these fire:
- output fails JSON-Schema validation or is truncated (`max_tokens`);
- a **deterministic check fails** (lines ≠ total, VAT ≠ 15% of the category subtotal, bad VAT-number format, impossible date);
- two independent passes **disagree** on a critical field;
- the model reports low confidence on a critical field;
- API error / timeout (retry with backoff on the *same* tier first).

If the top tier still fails validation, the page becomes **`needs_review`** for a human. That is a valid terminal state, so no job ever ends silently failed or hangs.

### 3. Thinking: adaptive, applied where it pays
Current models use **adaptive thinking** (`thinking: {type: "adaptive"}` + `output_config.effort`); the old fixed `budget_tokens` mode is deprecated on 4.6 and rejected with a 400 on 4.7+ ([docs](https://platform.claude.com/docs/en/build-with-claude/extended-thinking)). Thinking tokens bill as **output** tokens.
- Tier 1–2: effort `medium`; Tier 3–4: effort `high`.
- Narration prose: low effort (facts come from templates).
- Do **not** set high effort on every call: it multiplies output cost and latency, and does not by itself guarantee a correct number. (Pushback: "completion" comes from the orchestration above, not from thinking depth.)
- Keep the thinking config stable within a cached prefix; changing it invalidates prompt-cache breakpoints.

### 4. "Boosted memory" = resource completeness (Kone's definition, 19 Oct 2026)
**Principle:** the model must be given every resource it needs to finish its task. Nothing it needs is withheld, and a failed task should never be traceable to missing context. "Everything else we deal with as we go."

Per-task **context pack**, assembled by the Rust orchestrator (what the model needs, not simply everything):
- the page **image and text layer**, plus neighbouring pages and document-level context (page count, running totals, earlier extracted fields);
- the **output schema** and field definitions, the **South African VAT/invoice rules**, and worked examples;
- **supplier history** and past human corrections for this tenant (from the memory store);
- **tools it can call**: the deterministic validator and VAT/totals calculator from `clearcut-core` (so it checks its own arithmetic), a document-page fetcher, and a lookup for supplier master data;
- on escalation, the **failure report** from the previous tier (which checks failed and why), so the next model does not repeat the mistake.

Limit ("need", not "everything"): POPIA data minimisation and prompt-injection safety still apply, so unrelated tenants' data and unneeded personal identifiers are never included. The pack is logged per call so any failure can be audited as "was context missing?".

Mechanisms that deliver this:
1. **Prompt caching** for the static prefix (instructions, JSON schema, SA VAT rules, few-shot examples). Cache reads cost a small fraction of input price. Biggest, safest saving.
2. **Chunking over giant context.** Per-page/section calls beat one huge prompt for accuracy and cost; Haiku 5.5 also costs more above 100k-token prompts.
3. **Memory tool** (client-side, files we store) for cross-job learning: supplier layouts, field quirks, reviewer corrections.
   - **Write rule:** memory is written only from **human-confirmed corrections**, never straight from text read out of an uploaded document. Otherwise a malicious invoice could plant instructions that persist (prompt injection with memory).
   - Validate every path (no traversal), cap size, expire stale notes, and treat stored supplier details as personal information under POPIA.
   - Per-tenant memory stores; never shared across tenants.

### 5. Budgets and observability
Per-job and per-tenant token budgets; estimate before running; log model, tier, effort, tokens, cache hits and escalation reason for every call. Use the **Batch API (50% off)** for non-urgent bulk uploads.

### 6. Secrets
`ANTHROPIC_API_KEY` lives in the environment or a secrets manager, never in the repo (`.env` is git-ignored). Separate keys for dev and prod; rotate on exposure.

## Indicative cost (Rand)
List prices (per million tokens, USD) converted at an **assumed R18/USD**; confirm the live rate and current prices before budgeting. Source: [Anthropic pricing](https://platform.claude.com/docs/en/about-claude/pricing).

| Model | Input / M tokens | Output / M tokens |
|---|---|---|
| Haiku 5.5 (prompts ≤100k) | R1.80 | R9.00 |
| Sonnet 5.5 | R36 | R180 |
| Opus 5.5 | R72 | R360 |
| Fable 5.1 | R180 | R900 |

Illustration only: assume **5 000 input + 1 000 output tokens per page**, no thinking, caching or batch discount. Per page: Haiku ≈ **R0.02**, Sonnet ≈ **R0.36**, Opus ≈ **R0.72**, Fable ≈ **R1.80**. A 100-page document is roughly R2, R36, R72 or R180 respectively. Thinking adds output tokens; caching and batch reduce cost (batch halves it). The ladder keeps most pages at Tier 1–2.
Using US-only inference adds ×1.1.

## POPIA and confidentiality position (needs legal review)
- Anthropic's documented inference options are `global` (default) and `us`; **no South African or African region is documented**, and workspace data at rest is US-only ([Anthropic data residency](https://platform.claude.com/docs/en/build-with-claude/data-residency)). Uploaded invoices (which contain personal and business information) will therefore be **transferred outside South Africa** → POPIA s72 applies.
- A third-party summary (not Anthropic) says API data is deleted within 30 days by default, trust-and-safety-flagged content can be kept up to 2 years, and **zero data retention** must be requested via sales and may exclude the newest models ([Standard Compute, 2026-09-02](https://standardcompute.com/providers/anthropic)). **Verify all of this directly with Anthropic.**
- Mitigations in the design: data minimisation and **redaction** of bank and ID numbers before sending where not needed; a clear privacy notice and consent for people uploading documents; an operator/data-processing agreement with the provider; human review for consequential outcomes (s71); optional in-region/self-hosted provider later.
- Whoever operates ClearCut commercially must appoint an Information Officer and complete a POPIA assessment. This is not legal advice.

## Consequences
- Anthropic outage = pipeline stall; jobs queue and resume (state is in Postgres).
- Real-world accuracy and cost must be measured on the gold set before promising anything.
- The escalation policy is data: tunable without code changes.

## Rejected
- *Highest model + highest thinking on everything:* expensive, slow, still not a guarantee.
- *Letting the model write its own long-term memory from documents:* prompt-injection persistence.
