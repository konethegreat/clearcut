# Long invoices, Slice A: honest limits and a capability-aware ladder

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make a dense page retry with room instead of dying at 16 000 tokens, make a context overflow its own honest outcome, make the ladder skip models the key cannot use, make a dropped text layer visible to the reviewer, and make intake refuse the two types the worker cannot read.

**Architecture:** The policy gains a truncation ceiling above the answer ceiling; the page state machine retries a cut-off at that ceiling and ends `too_long` only when it has been reached. A context overflow is a new provider error and a new state-machine outcome that escalates but never retries the same input, and a pre-flight input check skips a rung whose window cannot hold the page plus its largest answer. The server asks the free Models API once per tenant (cached for an hour) and narrows the ladder before the read stage. A page whose text is above the sending limit records why it was read from its picture. Intake stops advertising TIFF and XML.

**Tech Stack:** Rust workspace (`clearcut-core`, `clearcut-provider`, `clearcut-anthropic`, `clearcut-keys`, `clearcut-router`, `clearcut-pipeline`, `clearcut-server`, `clearcut-intake`), `cargo test` with a real Postgres for the store-backed crates, TypeScript SDK, OpenAPI contract, Markdown docs.

**Spec:** [docs/superpowers/specs/2026-11-03-long-invoice-completeness-design.md](../specs/2026-11-03-long-invoice-completeness-design.md) (Slice A, §5, §11, §12).

## Global Constraints

- **No database migration.** The page state, page details and carry are JSON; every new field is additive with a serde default.
- **The reading schema stays `reading-v3` and the prompt stays `prompt-v3`.** Slice A changes no prompt text and reaches no new model.
- **Money is integer cents.** No floating point, no new price, no new provider call beyond the calls described here.
- **No document content in a log, audit entry, note, error or test fixture.** Notes and audit details carry counts, fixed names and byte sizes only. Uploaded text and model output are untrusted data, never instructions.
- **The call log's label vocabulary does not change.** A new escalation reason is stored as the existing `escalated` label with `input_too_large` as its detail, which fits `label_detail ~ '^[a-z_]{0,40}$'`.
- **User-facing copy contains no em dashes**, uses light surfaces, and says plainly what the system does.
- **Every source file keeps its first line** `// Written by Kone Tshivhinda & Claude`.
- **Test first.** Each task writes the failing test, runs it, implements, runs it green, then commits. Conventional commits.
- **Never print, commit or test with a real API key.** All tests use fakes and throwaway tenants.
- **Every gate green before merge:** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` (with the test Postgres), the web tests, the worker tests, the contract check and the gold-set check, as `README.md` and CI define them.

---

### Task 1: The truncation ceiling in `RouterPolicy`

**Files:**
- Modify: `crates/clearcut-core/src/escalation/policy.rs`
- Test: `crates/clearcut-core/src/escalation/tests_policy.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces:
  - `pub const DEFAULT_TRUNCATION_CEILING: u32 = 32_000;`
  - `RouterPolicy::truncation_ceiling(&self) -> u32`
  - `RouterPolicy::with_truncation_ceiling(self, ceiling: u32) -> Result<Self>`
  - `RouterPolicy::new(...)` keeps its five parameters and now sets `truncation_ceiling = answer_ceiling`, so every existing caller keeps its old behaviour until it asks for more.
  - Text without the field reads as `max(answer_ceiling, 32_000)`.

- [ ] **Step 1: Write the failing tests**

Add to `tests_policy.rs` (it already imports `serde_json`, `BTreeMap` is not needed here):

```rust
#[test]
fn the_default_ladder_may_retry_a_cut_off_with_thirty_two_thousand_tokens() {
    let policy = RouterPolicy::default_ladder();
    assert_eq!(policy.answer_ceiling(), 16_000);
    assert_eq!(policy.truncation_ceiling(), 32_000);
}

#[test]
fn a_policy_built_with_new_keeps_its_old_ceiling() {
    let policy = build(vec![rung("a")]).unwrap();
    assert_eq!(policy.truncation_ceiling(), 16_000);
}

#[test]
fn a_truncation_ceiling_below_the_answer_ceiling_or_over_the_provider_maximum_is_refused() {
    let base = build(vec![rung("a")]).unwrap();
    assert!(base.clone().with_truncation_ceiling(15_999).is_err());
    assert!(base.clone().with_truncation_ceiling(64_001).is_err());
    assert_eq!(
        base.with_truncation_ceiling(32_000).unwrap().truncation_ceiling(),
        32_000
    );
}

#[test]
fn a_policy_read_from_text_without_a_truncation_ceiling_gets_the_default() {
    let mut value = serde_json::to_value(RouterPolicy::default_ladder()).unwrap();
    value.as_object_mut().unwrap().remove("truncation_ceiling");
    let back: RouterPolicy = serde_json::from_value(value).unwrap();
    assert_eq!(back.truncation_ceiling(), 32_000);

    // A policy whose answer ceiling is already above the default keeps its own ceiling.
    let mut value = serde_json::to_value(
        RouterPolicy::new(vec![rung("a")], 50_000, 5, 3, 10).unwrap(),
    )
    .unwrap();
    value.as_object_mut().unwrap().remove("truncation_ceiling");
    let back: RouterPolicy = serde_json::from_value(value).unwrap();
    assert_eq!(back.truncation_ceiling(), 50_000);
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p clearcut-core escalation::tests_policy::the_default_ladder_may_retry_a_cut_off_with_thirty_two_thousand_tokens`
Expected: compile error, `no method named truncation_ceiling`.

- [ ] **Step 3: Implement in `policy.rs`**

At the top, beside `PROVIDER_MAX_OUTPUT_TOKENS`:

```rust
/// What a cut-off answer is retried with unless a policy says otherwise (ADR-0018 amendment).
pub const DEFAULT_TRUNCATION_CEILING: u32 = 32_000;
```

Add the field to `RouterPolicy` after `answer_ceiling`:

```rust
    /// The most a cut-off retry may ask for. It is at least the answer ceiling and at most the
    /// provider's own maximum. The budget still checks the retry before it is sent.
    truncation_ceiling: u32,
```

Add to `UncheckedPolicy` with a serde default:

```rust
    #[serde(default)]
    truncation_ceiling: Option<u32>,
```

Change `TryFrom<UncheckedPolicy>` to build and then set it:

```rust
    fn try_from(raw: UncheckedPolicy) -> Result<Self> {
        let truncation = raw
            .truncation_ceiling
            .unwrap_or(raw.answer_ceiling.max(DEFAULT_TRUNCATION_CEILING));
        RouterPolicy::new(
            raw.rungs,
            raw.answer_ceiling,
            raw.max_calls_per_page,
            raw.document_calls_per_page,
            raw.document_calls_extra,
        )?
        .with_truncation_ceiling(truncation)
    }
```

Add the check beside `check_ceiling`:

```rust
/// The truncation ceiling must leave room for the answer ceiling and stay inside the provider's.
pub(super) fn check_truncation_ceiling(answer_ceiling: u32, truncation_ceiling: u32) -> Result<()> {
    if truncation_ceiling < answer_ceiling || truncation_ceiling > PROVIDER_MAX_OUTPUT_TOKENS {
        return Err(invalid("the truncation ceiling is outside the answer ceiling to 64000"));
    }
    Ok(())
}
```

In `new`, after `check_ceiling(answer_ceiling)?;`:

```rust
        check_truncation_ceiling(answer_ceiling, answer_ceiling)?;
```

and in the returned value:

```rust
        Ok(RouterPolicy {
            rungs,
            answer_ceiling,
            truncation_ceiling: answer_ceiling,
            max_calls_per_page,
            document_calls_per_page,
            document_calls_extra,
        })
```

In `default_ladder`'s struct literal, after `answer_ceiling: 16_000,`:

```rust
            truncation_ceiling: DEFAULT_TRUNCATION_CEILING,
```

Add the getter and builder after `answer_ceiling()`:

```rust
    pub fn truncation_ceiling(&self) -> u32 {
        self.truncation_ceiling
    }

    /// The same policy with a different ceiling for a cut-off retry. It must leave room for the
    /// answer ceiling and stay inside what the provider takes.
    pub fn with_truncation_ceiling(mut self, ceiling: u32) -> Result<Self> {
        check_truncation_ceiling(self.answer_ceiling, ceiling)?;
        self.truncation_ceiling = ceiling;
        Ok(self)
    }
```

In `with_enabled`, the final call becomes:

```rust
        RouterPolicy::new(
            rungs,
            self.answer_ceiling,
            self.max_calls_per_page,
            self.document_calls_per_page,
            self.document_calls_extra,
        )?
        .with_truncation_ceiling(self.truncation_ceiling)
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p clearcut-core escalation::tests_policy`
Expected: every test passes.

- [ ] **Step 5: Commit**

```bash
git add crates/clearcut-core/src/escalation/policy.rs crates/clearcut-core/src/escalation/tests_policy.rs
git commit -m "feat(core): give a cut-off answer a 32000 token retry ceiling"
```

---

### Task 2: The page state machine uses the truncation ceiling

**Files:**
- Modify: `crates/clearcut-core/src/escalation/reader.rs`
- Test: `crates/clearcut-core/src/escalation/tests_reader.rs`

**Interfaces:**
- Consumes: `RouterPolicy::truncation_ceiling()` from Task 1.
- Produces: a `PageReader` that stores `truncation_ceiling` and resumes a stored carry that predates the field.

- [ ] **Step 1: Write the failing tests**

In `tests_reader.rs`, replace the two existing truncation tests listed below and add the new ones.

Replace `an_answer_cut_off_below_the_ceiling_is_retried_once_with_the_ceiling` with:

```rust
#[test]
fn an_answer_cut_off_is_retried_once_at_the_truncation_ceiling() {
    let policy = RouterPolicy::default_ladder()
        .with_enabled("claude-haiku-5-5", false)
        .unwrap();
    let (mut reader, _) = PageReader::start(&policy, 100);
    let step = reader.on_outcome(Outcome::Truncated);
    let plan = call(&step);
    assert_eq!(plan.kind, CallKind::RetryTruncated);
    assert_eq!(plan.rung, 1);
    assert_eq!(plan.max_output_tokens, 32_000);
    assert_eq!(plan.attempt, 2);
    // Cut off again at the raised ceiling: the page is too dense for one call.
    let step = reader.on_outcome(Outcome::Truncated);
    assert_eq!(review_of(&step), (ReviewReason::TooLong, None));
}
```

Replace `an_answer_cut_off_at_the_ceiling_is_too_long_at_once` with:

```rust
#[test]
fn a_policy_whose_truncation_ceiling_is_its_answer_ceiling_ends_too_long_at_once() {
    let policy = RouterPolicy::new(RouterPolicy::default_ladder().rungs().to_vec(), 16_000, 5, 3, 10)
        .unwrap();
    let (mut reader, _) = PageReader::start(&policy, 100);
    let step = reader.on_outcome(Outcome::Truncated);
    assert_eq!(review_of(&step), (ReviewReason::TooLong, None));
}
```

In `the_retry_after_a_cut_off_uses_the_raised_cap_only_for_that_rung`, change the first assertion to `32_000`.

Add:

```rust
#[test]
fn a_stored_carry_without_a_truncation_ceiling_resumes_at_its_old_ceiling() {
    let (mut reader, _) = PageReader::start(&RouterPolicy::default_ladder(), 100);
    reader.on_outcome(Outcome::Truncated); // the retry at 32 000 is waiting for an answer
    let mut value = serde_json::to_value(&reader).unwrap();
    value.as_object_mut().unwrap().remove("truncation_ceiling");
    let mut back: PageReader = serde_json::from_value(value).unwrap();
    // The old carry had no field, so it keeps the old rule: 32 000 is already past its
    // 16 000 ceiling, and a second cut-off ends the page.
    let step = back.on_outcome(Outcome::Truncated);
    assert_eq!(review_of(&step), (ReviewReason::TooLong, None));
}
```

In `a_page_that_ended_exactly_at_its_limit_with_its_last_attempt_as_the_draft_reads_back`, the
raised-cap value is now valid against the truncation ceiling (32 000). Keep it.

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p clearcut-core escalation::tests_reader::an_answer_cut_off_is_retried_once_at_the_truncation_ceiling`
Expected: fails with `16000` where `32000` was expected.

- [ ] **Step 3: Implement in `reader.rs`**

Add the field to `PageReader` after `ceiling`:

```rust
    /// What a cut-off retry may ask for. At least `ceiling`; see `RouterPolicy`.
    truncation_ceiling: u32,
```

Add it to `UncheckedReader` with a serde default:

```rust
    #[serde(default)]
    truncation_ceiling: Option<u32>,
```

In `TryFrom<UncheckedReader>`, after the answer-ceiling check:

```rust
        let truncation_ceiling = raw.truncation_ceiling.unwrap_or(raw.ceiling);
        check_truncation_ceiling(raw.ceiling, truncation_ceiling)
            .map_err(|_| broken("the truncation ceiling is outside the answer ceiling to 64000"))?;
```

Import it:

```rust
use super::policy::{check_ceiling, check_rung, check_truncation_ceiling};
```

In the rebuilt value:

```rust
        Ok(PageReader {
            rungs: raw.rungs,
            ceiling: raw.ceiling,
            truncation_ceiling,
            call_limit: raw.call_limit,
            ...
```

In `start`:

```rust
        let mut reader = PageReader {
            rungs,
            ceiling: policy.answer_ceiling(),
            truncation_ceiling: policy.truncation_ceiling(),
            ...
```

In the truncation branch:

```rust
            Outcome::Truncated => {
                if self.current_cap() < self.truncation_ceiling {
                    self.cap_override = Some(self.truncation_ceiling);
                    self.retry(CallKind::RetryTruncated)
                } else {
                    // At the truncation ceiling: a stronger model cannot be asked for more room
                    // than the policy allows, so a person decides (a split comes in Slice B).
                    self.review(ReviewReason::TooLong)
                }
            }
```

Update the `cap_override` field comment and the `TryFrom` check from `raw.ceiling` to
`raw.truncation_ceiling`:

```rust
        if raw
            .cap_override
            .is_some_and(|cap| cap == 0 || cap > raw.truncation_ceiling)
        {
            return Err(broken("the raised answer cap is outside 1 to the truncation ceiling"));
        }
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p clearcut-core escalation::tests_reader`
Expected: every test passes. If a tamper test named "a cap raised past the ceiling" now uses a
value that is still legal, raise its value past `truncation_ceiling` (for a 16 000 policy the
value 16 001 is still legal once `truncation_ceiling` is 32 000; change it to 32 001).

- [ ] **Step 5: Commit**

```bash
git add crates/clearcut-core/src/escalation/reader.rs crates/clearcut-core/src/escalation/tests_reader.rs
git commit -m "feat(core): retry a cut-off at the policy truncation ceiling"
```

---

### Task 3: A context overflow is its own error, reason and outcome

**Files:**
- Modify: `crates/clearcut-provider/src/lib.rs`
- Modify: `crates/clearcut-anthropic/src/failure/mod.rs`
- Test: `crates/clearcut-anthropic/src/failure/tests.rs`
- Modify: `crates/clearcut-core/src/escalation/mod.rs`
- Modify: `crates/clearcut-core/src/escalation/reader.rs`
- Test: `crates/clearcut-core/src/escalation/tests_policy.rs`
- Test: `crates/clearcut-core/src/escalation/tests_reader.rs`
- Modify: `crates/clearcut-router/src/lib.rs`
- Test: `crates/clearcut-router/src/tests.rs`

**Interfaces:**
- Consumes: `Outcome` and `EscalationReason` from `clearcut-core`.
- Produces:
  - `ProviderError::ContextExceeded`
  - `EscalationReason::InputTooLarge` (`"input_too_large"`)
  - `Outcome::ContextExceeded` and `Outcome::InputTooLarge`
  - `CallLabel::Escalated(EscalationReason::InputTooLarge)` for a context overflow

- [ ] **Step 1: Write the failing tests**

In `clearcut-anthropic/src/failure/tests.rs`, add:

```rust
#[test]
fn a_prompt_too_long_is_a_context_overflow_not_our_bug() {
    for message in [
        "prompt is too long: 250000 tokens > 200000 maximum",
        "input length and `max_tokens` exceed context limit: 210000 + 32000 > 200000",
    ] {
        let error = map(400, None, &body("invalid_request_error", message));
        assert_eq!(error, ProviderError::ContextExceeded, "{message}");
        assert_eq!(error.failure_kind(), FailureKind::NeedsReview);
        assert_eq!(error.usage(), None, "a refused prompt is not billed");
    }
}
```

In `tests_policy.rs`, add `"input_too_large",` after `"schema_invalid",` in
`escalation_reasons_have_exactly_the_agreed_names`.

In `tests_reader.rs`, add:

```rust
#[test]
fn a_context_overflow_escalates_and_never_retries_the_same_input() {
    let (mut reader, _) = default_start();
    let step = reader.on_outcome(Outcome::ContextExceeded);
    let plan = call(&step);
    assert_eq!(
        plan.kind,
        CallKind::Escalation {
            from: 0,
            reason: EscalationReason::InputTooLarge
        }
    );
    assert_eq!(plan.rung, 1);
    assert_eq!(reader.calls_made(), 1, "the refused call was still a call");
}

#[test]
fn a_context_overflow_at_the_top_rung_ends_too_long() {
    let (mut reader, _) = default_start();
    let step = feed(
        &mut reader,
        vec![
            Outcome::ContextExceeded,
            Outcome::ContextExceeded,
            Outcome::ContextExceeded,
        ],
    );
    assert_eq!(review_of(&step), (ReviewReason::TooLong, None));
}

#[test]
fn a_rung_that_cannot_hold_the_input_is_skipped_without_counting_a_call() {
    let (mut reader, _) = default_start();
    let step = reader.on_outcome(Outcome::InputTooLarge);
    assert_eq!(reader.calls_made(), 0, "nothing was sent, so nothing is counted");
    let plan = call(&step);
    assert_eq!(
        plan.kind,
        CallKind::Escalation {
            from: 0,
            reason: EscalationReason::InputTooLarge
        }
    );
}

#[test]
fn a_rung_that_cannot_hold_the_input_at_the_top_ends_too_long() {
    let (mut reader, _) = default_start();
    let step = feed(
        &mut reader,
        vec![
            Outcome::InputTooLarge,
            Outcome::InputTooLarge,
            Outcome::InputTooLarge,
        ],
    );
    assert_eq!(review_of(&step), (ReviewReason::TooLong, None));
}
```

In the `alphabet()` of `tests_reader.rs`, add `Outcome::ContextExceeded,` and
`Outcome::InputTooLarge,` after `Outcome::RungUnavailable,`. Change the two `12`s to
`letters.len()`:

- The comment and count in `every_sequence_of_outcomes_ends_within_the_page_cap`: use
  `let n = letters.len();` (already there) and change the comment to "All n^5 sequences..." and
  the final assertion to `n.pow(5)`.
- In `generated_policies_and_outcomes_never_exceed_a_cap`, change
  `usize::try_from(next() % 12).unwrap()` to `usize::try_from(next() % letters.len() as u64).unwrap()`.

In `crates/clearcut-router/src/tests.rs`, add:

```rust
#[tokio::test]
async fn a_context_overflow_escalates_without_retrying_the_same_input() {
    let provider = FakeProvider::scripted(vec![
        Err(ProviderError::ContextExceeded),
        answer_from(SONNET, &clean_reading()),
    ]);
    let worker = worker(PAGE_TEXT, 1);
    let result = read(&provider, &worker, &trusted_page()).await;

    assert_eq!(read_outcome(&result).0, 1);
    assert_eq!(
        result.calls[0].label,
        CallLabel::Escalated(EscalationReason::InputTooLarge)
    );
    assert_eq!(
        result.calls[1].kind,
        CallKind::Escalation {
            from: 0,
            reason: EscalationReason::InputTooLarge
        }
    );
    assert_eq!(provider.requests().len(), 2);
}

#[tokio::test]
async fn a_context_overflow_at_every_rung_goes_to_a_person_as_too_long() {
    let provider = FakeProvider::scripted(vec![
        Err(ProviderError::ContextExceeded),
        Err(ProviderError::ContextExceeded),
        Err(ProviderError::ContextExceeded),
    ]);
    let worker = worker(PAGE_TEXT, 1);
    let result = read(&provider, &worker, &trusted_page()).await;
    assert_eq!(review_outcome(&result), (ReviewReason::TooLong, None));
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p clearcut-anthropic failure::tests::a_prompt_too_long_is_a_context_overflow_not_our_bug`
Expected: compile error, no variant `ContextExceeded`.

- [ ] **Step 3: Implement**

`clearcut-provider/src/lib.rs`, add the variant after `Truncated`:

```rust
    /// The request was larger than the model's context window, so it was refused unread and
    /// unbilled. A stronger model or a larger window may take it; the same input will not.
    #[error("the input was larger than the model's context window")]
    ContextExceeded,
```

In `failure_kind`, add it to the `NeedsReview` arm:

```rust
            ProviderError::Refused { .. }
            | ProviderError::Rejected(_)
            | ProviderError::ModelUnavailable
            | ProviderError::ContextExceeded => FailureKind::NeedsReview,
```

`clearcut-anthropic/src/failure/mod.rs`, in `map_error`, after the covered-model arm and before
the 401 arm:

```rust
        400 if error.message_has("prompt is too long") || error.message_has("context limit") => {
            ProviderError::ContextExceeded
        }
```

`clearcut-core/src/escalation/mod.rs`, add to `EscalationReason` after `SchemaInvalid`:

```rust
    /// The page plus its largest answer does not fit the model's context window.
    InputTooLarge,
```

Add it to `ALL` and `as_str`:

```rust
    pub const ALL: [EscalationReason; 7] = [
        ...
        EscalationReason::SchemaInvalid,
        EscalationReason::InputTooLarge,
        EscalationReason::RungUnavailable,
    ];
...
            EscalationReason::InputTooLarge => "input_too_large",
```

`clearcut-core/src/escalation/reader.rs`, add to `Outcome`:

```rust
    /// The provider refused the request because it was larger than the model's context window.
    /// It was not read, so the same input is never sent again.
    ContextExceeded,
    /// Nothing was sent: the page plus its answer does not fit the model's window.
    InputTooLarge,
```

In `on_outcome`, the call-count line becomes:

```rust
        if !matches!(
            outcome,
            Outcome::BudgetRefused | Outcome::CannotBeSentSafely | Outcome::InputTooLarge
        ) {
            self.calls_made += 1;
        }
```

Add the two arms:

```rust
            Outcome::ContextExceeded => self.outgrown(EscalationReason::InputTooLarge),
            Outcome::InputTooLarge => self.outgrown(EscalationReason::InputTooLarge),
```

Add the helper beside `escalate`:

```rust
    /// Leaves a rung that cannot hold the page. At the top rung the page is too long for every
    /// model this policy may use, which is a person's decision, not a failure of a reading.
    fn outgrown(&mut self, reason: EscalationReason) -> Step {
        if self.position + 1 >= self.rungs.len() {
            return self.review(ReviewReason::TooLong);
        }
        self.escalate(reason)
    }
```

`clearcut-router/src/lib.rs`, in `on_error`, add before the `Rejected` arm:

```rust
            ProviderError::ContextExceeded => {
                // The provider read none of it and charged nothing. A stronger model gets a
                // larger window; the same input is never offered twice.
                record.label = CallLabel::Escalated(EscalationReason::InputTooLarge);
                Outcome::ContextExceeded
            }
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p clearcut-anthropic failure::tests`, then
`cargo test -p clearcut-core escalation`, then `cargo test -p clearcut-router`
Expected: all pass. The exhaustive reader test now runs `14^5` sequences and must still finish
under 64 turns each.

- [ ] **Step 5: Commit**

```bash
git add crates/clearcut-provider/src/lib.rs crates/clearcut-anthropic/src/failure/mod.rs crates/clearcut-anthropic/src/failure/tests.rs crates/clearcut-core/src/escalation/mod.rs crates/clearcut-core/src/escalation/reader.rs crates/clearcut-core/src/escalation/tests_policy.rs crates/clearcut-core/src/escalation/tests_reader.rs crates/clearcut-router/src/lib.rs crates/clearcut-router/src/tests.rs
git commit -m "feat(router): treat a context overflow as its own outcome"
```

---

### Task 4: Skip a rung whose window cannot hold the call

**Files:**
- Modify: `crates/clearcut-router/src/lib.rs`
- Test: `crates/clearcut-router/src/tests.rs`

**Interfaces:**
- Consumes: `Outcome::InputTooLarge` and `EscalationReason::InputTooLarge` from Task 3.
- Produces:
  - `RouterConfig.model_input_limits: BTreeMap<String, u64>` (empty by default)
  - `RouterConfig::with_input_limits(self, limits: BTreeMap<String, u64>) -> Self`

- [ ] **Step 1: Write the failing test**

In `crates/clearcut-router/src/tests.rs`, add:

```rust
#[tokio::test]
async fn a_model_whose_window_cannot_hold_the_page_is_skipped() {
    let mut cfg = config();
    cfg.model_input_limits
        .insert(HAIKU.to_string(), 1_000);
    let provider = FakeProvider::scripted(vec![answer_from(SONNET, &clean_reading())]);
    let worker = worker(PAGE_TEXT, 1);
    let result = finished(
        router(&provider, &worker, cfg)
            .read_page(&trusted_page(), &mut budget(), 100, None)
            .await,
    );

    assert_eq!(read_outcome(&result).0, 1);
    assert_eq!(
        result.calls[0].label,
        CallLabel::NotSent(ReviewReason::TooLong)
    );
    assert_eq!(result.calls[0].cost, Cost::ZERO);
    assert_eq!(provider.call_count(), 1, "only Sonnet was asked");
}
```

- [ ] **Step 2: Run the test to see it fail**

Run: `cargo test -p clearcut-router a_model_whose_window_cannot_hold_the_page_is_skipped`
Expected: compile error, no field `model_input_limits`.

- [ ] **Step 3: Implement**

In `RouterConfig`, after `tenant_rules`:

```rust
    /// Each model's context window, when the Models API has told us. A model that is absent is
    /// not checked: the call is made and the provider answers.
    pub model_input_limits: BTreeMap<String, u64>,
```

In `RouterConfig::new`:

```rust
            model_input_limits: BTreeMap::new(),
```

Add the import near the other `std` imports:

```rust
use std::collections::BTreeMap;
```

Add beside `new`:

```rust
    /// The same configuration with the models' context windows, from the key's own model list.
    pub fn with_input_limits(mut self, limits: BTreeMap<String, u64>) -> Self {
        self.model_input_limits = limits;
        self
    }
```

In `attempt`, after the `call_estimate` and before the price lookup:

```rust
        // A model whose window cannot hold the page plus its largest answer is not asked: the
        // provider would refuse the call, and a stronger model has more room.
        if let Some(limit) = self.config.model_input_limits.get(&plan.model).copied() {
            let wanted = estimate
                .prefix_tokens
                .saturating_add(estimate.page_input_tokens)
                .saturating_add(u64::from(estimate.max_output_tokens));
            if wanted > limit {
                run.record_unsent(
                    plan,
                    Some(manifest),
                    CallLabel::NotSent(ReviewReason::TooLong),
                );
                return Attempted::just(Outcome::InputTooLarge);
            }
        }
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p clearcut-router a_model_whose_window_cannot_hold_the_page_is_skipped`
Expected: pass.

- [ ] **Step 5: Commit**

```bash
git add crates/clearcut-router/src/lib.rs crates/clearcut-router/src/tests.rs
git commit -m "feat(router): skip a rung that cannot hold the page"
```

---

### Task 5: Narrow the ladder to what the tenant's key can use

**Files:**
- Modify: `crates/clearcut-core/src/escalation/policy.rs`
- Test: `crates/clearcut-core/src/escalation/tests_policy.rs`
- Modify: `crates/clearcut-keys/src/lib.rs`
- Test: `crates/clearcut-keys/src/tests.rs`
- Modify: `crates/clearcut-pipeline/src/lib.rs`
- Test: `crates/clearcut-pipeline/src/tests/limits.rs` (new) and `crates/clearcut-pipeline/src/tests.rs` (`mod limits;`)
- Modify: `crates/clearcut-server/src/runner.rs`
- Test: `crates/clearcut-server/src/runner.rs` (the existing `mod tests`)

**Interfaces:**
- Consumes: `KeyCheck`, `ModelInfo`, `Lifecycle` from `clearcut-anthropic`; `RouterPolicy` from `clearcut-core`; `TenantKeys::resolve`.
- Produces:
  - `RouterPolicy::with_output_limits(&self, limits: &BTreeMap<String, u32>) -> Self`
  - `TenantKeys::check(&self, key: &ApiKey) -> Result<KeyCheck, KeyCheckError>`
  - `clearcut_pipeline::ModelLimits { unavailable, max_output_tokens, max_input_tokens }`
  - `ProviderSource::limits(&self, tenant) -> LimitsFuture<'_>` (default: `None`)
  - `ReadSettings::with_limits(self, limits: &ModelLimits) -> Self`
  - a one-hour cache in the server's `TenantProviders`

- [ ] **Step 1: Write the failing tests**

`tests_policy.rs` (add `use std::collections::BTreeMap;` at the top):

```rust
#[test]
fn a_rung_is_clamped_to_the_largest_answer_its_model_takes() {
    let base = build(vec![rung("a"), rung("b")]).unwrap();
    let limits = BTreeMap::from([("a".to_string(), 4_000)]);
    let clamped = base.with_output_limits(&limits);
    assert_eq!(clamped.rungs()[0].max_output_tokens, 4_000);
    assert_eq!(clamped.rungs()[1].max_output_tokens, 8_000);
    // A zero or an absent model changes nothing.
    let zero = BTreeMap::from([("a".to_string(), 0)]);
    assert_eq!(
        base.clone().with_output_limits(&zero).rungs()[0].max_output_tokens,
        8_000
    );
}
```

`clearcut-keys/src/tests.rs`, add:

```rust
#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn asking_what_a_key_can_do_uses_the_checker_and_costs_nothing(pool: PgPool) {
    let w = world(&pool, true, KeyMode::TenantOnly, false, good).await;
    w.keys.set(&w.admin, SENTINEL).await.unwrap();
    let asked = w.asked.load(Ordering::SeqCst);
    let check = w.keys.check(&ApiKey::parse(SENTINEL).unwrap()).await.unwrap();
    assert_eq!(w.asked.load(Ordering::SeqCst), asked + 1);
    assert_eq!(check.models.len(), 2);
}
```

Create `crates/clearcut-pipeline/src/tests/limits.rs`:

```rust
// Written by Kone Tshivhinda & Claude
//! What the server learns from the Models API narrows the ladder before a page is read.

use crate::{ModelLimits, ReadSettings};
use std::collections::BTreeMap;

#[test]
fn the_settings_skip_a_model_the_key_cannot_use_and_clamp_its_answer() {
    let settings = ReadSettings::standard().with_limits(&ModelLimits {
        unavailable: vec!["claude-sonnet-5-5".to_string()],
        max_output_tokens: BTreeMap::from([("claude-haiku-5-5".to_string(), 10_000)]),
        max_input_tokens: BTreeMap::from([("claude-haiku-5-5".to_string(), 200_000)]),
    });

    let names: Vec<&str> = settings
        .router()
        .policy
        .rungs()
        .iter()
        .map(|rung| rung.model.as_str())
        .collect();
    assert_eq!(
        names,
        ["claude-haiku-5-5", "claude-opus-5-5", "claude-fable-5-1"]
    );
    assert_eq!(settings.router().policy.rungs()[0].max_output_tokens, 10_000);
    assert_eq!(
        settings.router().model_input_limits.get("claude-haiku-5-5"),
        Some(&200_000)
    );
}

#[test]
fn no_limits_leave_the_ladder_as_it_was() {
    let before = ReadSettings::standard();
    let after = before.clone().with_limits(&ModelLimits::default());
    assert_eq!(after.router().policy, before.router().policy);
    assert!(after.router().model_input_limits.is_empty());
}
```

Add `mod limits;` to `crates/clearcut-pipeline/src/tests.rs` beside `mod read;`.

In `crates/clearcut-server/src/runner.rs`'s `mod tests`, add:

```rust
use clearcut_anthropic::models::{KeyCheck, ModelInfo};
use clearcut_anthropic::Lifecycle;

fn model(id: &str, lifecycle: Lifecycle, structured: bool, image: bool) -> ModelInfo {
    ModelInfo {
        id: id.to_string(),
        display_name: id.to_string(),
        lifecycle,
        max_input_tokens: Some(200_000),
        max_tokens: Some(64_000),
        structured_outputs: structured,
        image_input: image,
        pdf_input: true,
        batch: false,
    }
}

#[test]
fn a_key_check_names_the_rungs_it_cannot_use_and_the_limits_it_reports() {
    let check = KeyCheck {
        models: vec![
            model("claude-haiku-5-5", Lifecycle::Active, true, true),
            model("claude-sonnet-5-5", Lifecycle::Active, false, true),
        ],
        request_id: None,
    };
    let limits = super::limits_from(&check);
    assert_eq!(
        limits.unavailable,
        ["claude-sonnet-5-5", "claude-opus-5-5", "claude-fable-5-1"]
    );
    assert_eq!(
        limits.max_output_tokens.get("claude-haiku-5-5"),
        Some(&64_000)
    );
    assert_eq!(
        limits.max_input_tokens.get("claude-haiku-5-5"),
        Some(&200_000)
    );
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p clearcut-core escalation::tests_policy::a_rung_is_clamped_to_the_largest_answer_its_model_takes`
Expected: compile error, no method `with_output_limits`.

- [ ] **Step 3: Implement the pure parts**

`policy.rs`:

```rust
use std::collections::BTreeMap;
```

Add beside `without_models`:

```rust
    /// The same policy with each rung's answer cap clamped to what its model accepts. A model
    /// that is not named, or whose limit is zero, keeps its cap.
    pub fn with_output_limits(&self, limits: &BTreeMap<String, u32>) -> Self {
        let mut narrowed = self.clone();
        for rung in &mut narrowed.rungs {
            if let Some(max) = limits.get(&rung.model).copied().filter(|max| *max > 0) {
                rung.max_output_tokens = rung.max_output_tokens.min(max);
            }
        }
        narrowed
    }
```

`clearcut-keys/src/lib.rs`, beside `status`:

```rust
    /// Asks the checker what `key` can do, for free. It stores nothing.
    pub async fn check(&self, key: &ApiKey) -> Result<KeyCheck, KeyCheckError> {
        self.checker.check(key).await
    }
```

`clearcut-pipeline/src/lib.rs`:

- Add imports:

```rust
use clearcut_core::escalation::RouterPolicy;
use std::collections::BTreeMap;
```

- Add after `ReadSettings`:

```rust
/// What the tenant's key can use, from the Models API (free; ADR-0019). Unknown limits narrow
/// nothing: the ladder is used as it is and the provider says `ModelUnavailable` if it must.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelLimits {
    /// The ladder models this key cannot call, or that cannot take a page picture.
    pub unavailable: Vec<String>,
    /// Each usable model's largest answer.
    pub max_output_tokens: BTreeMap<String, u32>,
    /// Each usable model's context window.
    pub max_input_tokens: BTreeMap<String, u64>,
}
```

- Add to `ReadSettings`:

```rust
impl ReadSettings {
    /// The same settings narrowed to what the tenant's key can use.
    pub fn with_limits(mut self, limits: &ModelLimits) -> Self {
        let names: Vec<&str> = limits.unavailable.iter().map(String::as_str).collect();
        self.router.policy = self.router.policy.without_models(&names);
        self.router.policy = self.router.policy.with_output_limits(&limits.max_output_tokens);
        self.router.model_input_limits = limits.max_input_tokens.clone();
        self
    }
}
```

- Add to `ProviderSource`:

```rust
pub type LimitsFuture<'a> = Pin<Box<dyn Future<Output = Option<ModelLimits>> + Send + 'a>>;
```

and the default method:

```rust
    /// What the tenant's key can use, when the source knows. `None` means "not known": the
    /// ladder is used unabridged.
    fn limits<'a>(&'a self, _tenant: TenantId) -> LimitsFuture<'a> {
        Box::pin(async { None })
    }
```

- In `run`, the `Ok(provider)` arm:

```rust
                Ok(provider) => {
                    let settings = match source.limits(tenant).await {
                        Some(limits) => settings.clone().with_limits(&limits),
                        None => settings.clone(),
                    };
                    Models { provider, settings }
                }
```

- [ ] **Step 4: Implement the server cache and the model list**

`crates/clearcut-server/src/runner.rs`:

Add imports:

```rust
use clearcut_anthropic::models::{KeyCheck, KeyCheckError};
use clearcut_anthropic::Lifecycle;
use clearcut_core::escalation::RouterPolicy;
use clearcut_pipeline::{LimitsFuture, ModelLimits};
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::{Duration, Instant};
```

Add the cache types and field:

```rust
/// How long a key's model list is believed. The check is free, but a document does not need to
/// ask again within an hour.
const LIMITS_TTL: Duration = Duration::from_secs(60 * 60);

struct CachedLimits {
    /// The last four characters, so a newly set key is checked again.
    fingerprint: String,
    checked: Instant,
    limits: ModelLimits,
}

struct TenantProviders {
    keys: TenantKeys,
    config: AnthropicConfig,
    limits: Mutex<HashMap<TenantId, CachedLimits>>,
}
```

Implement the trait method beside `key_rejected`:

```rust
    fn limits<'a>(&'a self, tenant: TenantId) -> LimitsFuture<'a> {
        Box::pin(async move { self.limits_for(tenant).await })
    }
```

and the method:

```rust
impl TenantProviders {
    /// What the tenant's key can use, cached for an hour per key. A check that fails leaves the
    /// full ladder in place (the provider skips an unusable rung itself).
    async fn limits_for(&self, tenant: TenantId) -> Option<ModelLimits> {
        let Ok(Resolved::Key(key, _)) = self.keys.resolve(tenant).await else {
            return None;
        };
        let fingerprint = key.fingerprint();
        if let Ok(cache) = self.limits.lock() {
            if let Some(cached) = cache.get(&tenant) {
                if cached.fingerprint == fingerprint && cached.checked.elapsed() < LIMITS_TTL {
                    return Some(cached.limits.clone());
                }
            }
        }
        let check: KeyCheck = self.keys.check(&key).await.ok()?;
        let limits = limits_from(&check);
        if let Ok(mut cache) = self.limits.lock() {
            cache.insert(
                tenant,
                CachedLimits {
                    fingerprint,
                    checked: Instant::now(),
                    limits: limits.clone(),
                },
            );
        }
        Some(limits)
    }
}
```

Add the pure mapping function:

```rust
/// The ladder models the key cannot use, and the limits of those it can. A model that is absent
/// from the list, retired, or unable to take structured output or a picture is left out.
fn limits_from(check: &KeyCheck) -> ModelLimits {
    let mut limits = ModelLimits::default();
    for rung in RouterPolicy::default_ladder().rungs() {
        let usable = check
            .model(&rung.model)
            .filter(|info| check.can_use(&rung.model) && info.structured_outputs && info.image_input);
        match usable {
            Some(info) => {
                if let Some(max) = info.max_tokens {
                    limits
                        .max_output_tokens
                        .insert(rung.model.clone(), u32::try_from(max).unwrap_or(u32::MAX).max(1));
                }
                if let Some(max) = info.max_input_tokens {
                    limits.max_input_tokens.insert(rung.model.clone(), max);
                }
            }
            None => limits.unavailable.push(rung.model.clone()),
        }
    }
    limits
}
```

In `start`, construct the struct with the cache:

```rust
        .with_provider_source(
            Arc::new(TenantProviders {
                keys,
                config: anthropic,
                limits: Mutex::new(HashMap::new()),
            }),
            ReadSettings::standard(),
        );
```

Remove the now-unused `KeyCheckError` import if the compiler says so.

- [ ] **Step 5: Run the tests to see them pass**

Run: `cargo test -p clearcut-core escalation::tests_policy`, then
`cargo test -p clearcut-pipeline tests::limits`, then
`cargo test -p clearcut-server runner::tests`, then
`cargo test -p clearcut-keys` (this one needs the test Postgres).
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add crates/clearcut-core/src/escalation/policy.rs crates/clearcut-core/src/escalation/tests_policy.rs crates/clearcut-keys/src/lib.rs crates/clearcut-keys/src/tests.rs crates/clearcut-pipeline/src/lib.rs crates/clearcut-pipeline/src/tests.rs crates/clearcut-pipeline/src/tests/limits.rs crates/clearcut-server/src/runner.rs
git commit -m "feat(server): narrow the ladder to what the tenant key can use"
```

---

### Task 6: A dropped text layer is visible on the page and in the audit

**Files:**
- Modify: `crates/clearcut-router/src/lib.rs`
- Test: `crates/clearcut-router/src/tests.rs`
- Modify: `crates/clearcut-pipeline/src/read.rs`
- Test: `crates/clearcut-pipeline/src/tests/read.rs`

**Interfaces:**
- Consumes: `MAX_PAGE_TEXT_BYTES` from `clearcut-pack`.
- Produces:
  - `PageResult.text_dropped_bytes: usize`
  - `page_notes(outcome, text_dropped_bytes) -> Value` (crate-visible)
  - `text_dropped_pages(&[StoredPage]) -> u32` (crate-visible)
  - audit detail field `pages_text_dropped`

- [ ] **Step 1: Write the failing tests**

`crates/clearcut-router/src/tests.rs`, add:

```rust
#[tokio::test]
async fn text_above_the_sending_limit_is_dropped_and_counted() {
    let long = "a".repeat(MAX_PAGE_TEXT_BYTES + 1);
    let provider = FakeProvider::answering(|_| answer_from(HAIKU, &clean_reading()));
    let worker = worker(&long, 1);
    let result = finished(
        router(&provider, &worker, config())
            .read_page(&trusted_page(), &mut budget(), 100, None)
            .await,
    );

    assert_eq!(result.text_dropped_bytes, long.len());
    assert!(
        texts(&provider.requests()[0])
            .iter()
            .all(|block| !block.starts_with("<page_text")),
        "the too-large text was never sent"
    );
    // The byte count is a size, never document text.
    assert_eq!(result.text_dropped_bytes.to_string().len() <= 7, true);
}

#[tokio::test]
async fn text_within_the_limit_is_not_counted_as_dropped() {
    let provider = FakeProvider::scripted(vec![answer(&clean_reading())]);
    let worker = worker(PAGE_TEXT, 1);
    let result = read(&provider, &worker, &trusted_page()).await;
    assert_eq!(result.text_dropped_bytes, 0);
}
```

Add `use clearcut_pack::MAX_PAGE_TEXT_BYTES;` to the test imports.

`crates/clearcut-pipeline/src/tests/read.rs` (at the end), add:

```rust
use crate::read::{page_notes, text_dropped_pages};
use clearcut_core::escalation::{ConfidenceBasis, PageOutcome};

#[test]
fn a_page_read_from_its_picture_says_why_in_its_notes() {
    let outcome = PageOutcome::Read {
        rung: 0,
        attempt: 1,
        confidence: ConfidenceBasis::Unresolved,
        notes: Vec::new(),
    };
    assert_eq!(
        page_notes(&outcome, 200_001),
        serde_json::json!([{ "type": "text_dropped", "bytes": 200_001 }])
    );
    assert_eq!(page_notes(&outcome, 0), serde_json::json!([]));
}

#[test]
fn the_audit_counts_only_pages_whose_text_was_dropped() {
    use clearcut_store::page_readings::{PageDetails, PageReading, PageState};
    let page = |notes: serde_json::Value| PageReading {
        page: 1,
        state: PageState::InProgress {
            carry: serde_json::json!({}),
        },
        details: PageDetails {
            notes,
            ..PageDetails::default()
        },
    };
    let ended = vec![
        page(serde_json::json!([{ "type": "text_dropped", "bytes": 200_001 }])),
        page(serde_json::json!([{ "type": "too_long" }])),
        page(serde_json::json!([])),
    ];
    assert_eq!(text_dropped_pages(&ended), 1);
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p clearcut-router text_above_the_sending_limit_is_dropped_and_counted`
Expected: compile error, no field `text_dropped_bytes`.

- [ ] **Step 3: Implement in the router**

In `Material`, add:

```rust
    /// The page's text was present but larger than could be sent, so the picture was read
    /// instead. A size, never the text.
    text_dropped_bytes: usize,
```

In `fetch_material`, replace the filter block with:

```rust
        // A page with more text than is ever sent is read from its picture, never cut. The drop
        // is remembered and shown, so a reviewer knows why the picture was used.
        let sends_text = matches!(
            (task.route, task.text_source),
            (ReadRoute::TextAndImage, PageTextSource::TextLayer)
                | (ReadRoute::ImageFirst, PageTextSource::TextLayer)
                | (ReadRoute::ImageOnly, PageTextSource::Ocr)
        );
        let text_dropped_bytes = content
            .as_ref()
            .filter(|_| sends_text)
            .map(|page| page.text().len())
            .filter(|len| *len > MAX_PAGE_TEXT_BYTES)
            .unwrap_or(0);
        let text = content
            .as_ref()
            .filter(|page| page.text().len() <= MAX_PAGE_TEXT_BYTES)
            .and_then(|page| PageText::for_route(task.route, task.text_source, page.text().to_string()));
        let content = if text.is_some() { content } else { None };
        Ok(Material {
            text,
            content,
            trust,
            text_dropped_bytes,
        })
```

Add to `PageResult` after `unavailable_models`:

```rust
    /// Bytes of the page's text that were not sent because they were above the sending limit.
    pub text_dropped_bytes: usize,
```

In `finish`, at the top:

```rust
        let text_dropped_bytes = run
            .material
            .as_ref()
            .map_or(0, |material| material.text_dropped_bytes);
```

and in the `PageResult` literal:

```rust
            text_dropped_bytes,
```

- [ ] **Step 4: Implement in the pipeline**

In `crates/clearcut-pipeline/src/read.rs`, add the helpers above `save_finished`:

```rust
/// What goes in a page's notes: the router's own notes, plus why the picture was used when the
/// page's text was too large to send. Counts only, never text.
pub(crate) fn page_notes(outcome: &PageOutcome, text_dropped_bytes: usize) -> Value {
    let mut notes = match outcome {
        PageOutcome::Read { notes, .. } => serde_json::to_value(notes).unwrap_or_else(|_| json!([])),
        PageOutcome::NeedsReview { .. } => json!([]),
    };
    if text_dropped_bytes > 0 {
        if let Some(list) = notes.as_array_mut() {
            list.push(json!({ "type": "text_dropped", "bytes": text_dropped_bytes }));
        }
    }
    notes
}

/// How many pages of a document were read from their picture because their text was too large.
pub(crate) fn text_dropped_pages(ended: &[StoredPage]) -> u32 {
    let count = ended
        .iter()
        .filter(|page| {
            page.details
                .notes
                .as_array()
                .is_some_and(|notes| notes.iter().any(|note| {
                    note.get("type").and_then(Value::as_str) == Some("text_dropped")
                }))
        })
        .count();
    u32::try_from(count).unwrap_or(u32::MAX)
}
```

In `save_finished`, replace the `notes` binding:

```rust
        let notes = page_notes(&result.outcome, result.text_dropped_bytes);
```

and in `PageDetails`:

```rust
            notes,
```

In `finish_reading`, pass the count to the audit:

```rust
                        detail: audit_detail(
                            pages,
                            &all,
                            spend.calls,
                            spend.cost,
                            text_dropped_pages(&ended),
                            completion.as_ref(),
                        ),
```

In `audit_detail`, add the parameter and field:

```rust
fn audit_detail(
    pages: u32,
    tally: &Tally,
    calls: u32,
    cost: Cost,
    text_dropped: u32,
    completion: Option<&Completion>,
) -> Value {
```

and in the JSON:

```rust
        "pages_text_dropped": text_dropped,
```

- [ ] **Step 5: Run the tests to see them pass**

Run: `cargo test -p clearcut-router text_above_the_sending_limit`, then
`cargo test -p clearcut-pipeline tests::read`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add crates/clearcut-router/src/lib.rs crates/clearcut-router/src/tests.rs crates/clearcut-pipeline/src/read.rs crates/clearcut-pipeline/src/tests/read.rs
git commit -m "feat(pipeline): record when a page's text was too large to send"
```

---

### Task 7: Intake accepts what the worker can read

**Files:**
- Modify: `crates/clearcut-intake/src/lib.rs`
- Test: `crates/clearcut-intake/src/tests.rs`
- Modify: `crates/clearcut-server/src/documents/mod.rs`
- Test: `crates/clearcut-server/src/documents/tests.rs`
- Modify: `crates/clearcut-server/src/openapi.rs`
- Modify: `contracts/openapi.json`
- Modify: `packages/sdk/src/client.ts`
- Modify: `packages/sdk/src/types.ts`
- Rebuild: `packages/sdk/dist/client.d.ts`, `packages/sdk/dist/types.d.ts`

**Interfaces:**
- Consumes: nothing.
- Produces: `MediaType` has exactly three variants: `Pdf`, `Png`, `Jpeg`. TIFF and XML are refused by
  `MediaType::parse` and therefore before the body is read.

- [ ] **Step 1: Write the failing tests**

In `clearcut-intake/src/tests.rs`:

- Remove the `("image/tiff", ...)`, `("application/xml", ...)` and `("text/xml; charset=utf-8", ...)`
  rows from `the_accepted_media_types_parse_whatever_their_case_or_parameters`.
- In `everything_else_is_refused_as_a_media_type`, add:
  `"image/tiff"`, `"application/xml"`, `"text/xml"`, `"text/xml; charset=utf-8"`.
- Remove the two TIFF and XML assertions from `each_media_type_has_one_canonical_name` and the
  `MediaType::parse("text/xml")` line.
- Delete the `TIFF`, `XML` and `XML_WITH_SPACE` constants, the TIFF and XML rows in
  `the_declared_type_must_match_the_first_bytes`, and the whole
  `xml_may_begin_with_a_byte_order_mark_and_whitespace` test.
- In `every_accepted_media_type_can_be_uploaded`, keep only the PDF, PNG and JPEG rows and change
  the count to 3.
- In `an_unsupported_media_type_is_refused_before_the_body_is_read`, add `"image/tiff"` and
  `"application/xml"` to the list.

In `clearcut-server/src/documents/tests.rs`, change the expected `accepted` array to:

```rust
                "accepted": ["application/pdf", "image/png", "image/jpeg"],
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p clearcut-intake media_types`
Expected: `image/tiff` and `application/xml` are still accepted.

- [ ] **Step 3: Implement in `clearcut-intake/src/lib.rs`**

- Module docs line 9: "must be an accepted media type (PDF, PNG or JPEG)".
- The module docs sentence "a PDF must start `%PDF-`, an image with its signature, XML with `<`"
  becomes "a PDF must start `%PDF-`, an image with its signature".
- Remove the `Tiff` and `Xml` variants, their `parse` arms, their `as_str` arms, and the
  `signature_matches` arms.
- Delete `xml_starts`, `first_is_tag` and `utf16_first_is_tag`.
- Change the `HEAD_BYTES` doc comment to: "How many leading bytes are looked at to tell the type:
  enough for the longest signature (a PNG's eight bytes)." The value may stay 64.

- [ ] **Step 4: Update the server and the public contract**

`crates/clearcut-server/src/documents/mod.rs`:
- The module table row becomes `| not a PDF, PNG or JPEG | 415, listing what is accepted |`.
- `explain(UnsupportedMediaType)`'s `accepted` array keeps only the three variants.

`crates/clearcut-server/src/openapi.rs` line 168: `"Not a PDF, PNG or JPEG"`.

`contracts/openapi.json` line 963: `"description": "Not a PDF, PNG or JPEG"`.

`packages/sdk/src/client.ts` line 93: the sentence becomes
`` * `image/png` or `image/jpeg`. Taken from a Blob's own type if you give `` and check the
surrounding line reads naturally (adjust the line break, not the meaning).

`packages/sdk/src/types.ts` line 57:

```ts
  media_type: "application/pdf" | "image/png" | "image/jpeg";
```

Rebuild the SDK:

```bash
cd packages/sdk && npx tsc -p tsconfig.json && cd ../..
```

Confirm `packages/sdk/dist/types.d.ts` and `dist/client.d.ts` carry the new union and comment.

- [ ] **Step 5: Run the tests to see them pass**

Run: `cargo test -p clearcut-intake`, then `cargo test -p clearcut-server documents`, then the
repo's contract check (as CI runs it, `node scripts/check-contract.mjs` or the command named in
`README.md`).
Expected: all pass; the contract generator reports no difference.

- [ ] **Step 6: Commit**

```bash
git add crates/clearcut-intake/src/lib.rs crates/clearcut-intake/src/tests.rs crates/clearcut-server/src/documents/mod.rs crates/clearcut-server/src/documents/tests.rs crates/clearcut-server/src/openapi.rs contracts/openapi.json packages/sdk/src/client.ts packages/sdk/src/types.ts packages/sdk/dist/client.d.ts packages/sdk/dist/types.d.ts
git commit -m "fix(intake): accept only the types the worker can read"
```

---

### Task 8: Documentation, the full gate, and the pull request

**Files:**
- Modify: `docs/adr/0018-model-router-and-escalation.md`
- Modify: `docs/adr/0012-anthropic-adapter.md`
- Modify: `docs/ARCHITECTURE.md`
- Modify: `docs/LAUNCH.md`
- Modify: `docs/THREAT-MODEL.md`
- Modify: `docs/WORKLOG.md`
- Review: `docs/OPERATIONS.md` and `README.md` for any accepted-type list

**Interfaces:**
- Consumes: every change above.
- Produces: the written record and a green Slice A pull request.

- [ ] **Step 1: Amend the ADRs**

`docs/adr/0018-model-router-and-escalation.md`, in the Status line, append a sentence:

```text
Amended 4 November 2026 (Slice A of the long-invoice design): a cut-off answer is retried at a
truncation ceiling of 32 000 tokens (the budget still refuses a retry that would breach a cap); a
context overflow is `ProviderError::ContextExceeded` and escalates once per rung without ever
repeating the same input; a rung whose context window cannot hold the page plus its largest answer
is not called; the server narrows the ladder once an hour from the free Models API using the
tenant's own key.
```

Add a short section at the end of the decision body:

```markdown
## Amendment: the ladder knows the key and may ask for more room (4 November 2026)

The ladder is still data, and the deterministic checks are still the judge. Three limits move:

- `RouterPolicy` has `truncation_ceiling`, default 32 000, at least the answer ceiling and at most
  the provider's 64 000. A cut-off below it is retried once at it; a cut-off at it ends `too_long`.
  The budget check runs first, so a Sonnet retry at 32 000 is refused on a document whose R5 cap
  cannot hold it and the page ends `budget`. That is the honest outcome, not a lost reading.
- A context overflow is its own error and outcome. It escalates to the next rung, which has a
  larger window, and never resends the same request. At the top rung it ends `too_long`.
- Before a call, the router compares the estimated input plus the largest answer with the model's
  `max_input_tokens`, when the Models API reported it, and skips a rung that cannot fit. The skip
  is not a call: it is logged `not_sent` with reason `too_long` and does not count against the
  document's call allowance.

The server asks the Models API once per tenant key and caches the answer for an hour. A model that
is absent, retired, or unable to take structured output or a picture is left out of the ladder. If
the check itself fails, the full ladder is used: the provider's `ModelUnavailable` path already
skips a rung and remembers it for the hour.

`PageReader` stores the truncation ceiling in its carry with a serde default, so a job that spans
this deploy resumes at the ceiling it started with.
```

`docs/adr/0012-anthropic-adapter.md`, in the Status line, append:

```text
Amended 4 November 2026: a 400 whose message says the prompt is too long, or that the input and
`max_tokens` exceed the context limit, maps to `ProviderError::ContextExceeded` (needs a person if
it escapes the router) instead of the generic `Rejected`. The body is still read only to classify,
and the error carries no usage because the call was unbilled.
```

- [ ] **Step 2: Update the public docs**

- `docs/ARCHITECTURE.md` line 41: `accepts PDF/PNG/JPEG/TIFF/XML only` becomes
  `accepts PDF/PNG/JPEG only`.
- `docs/LAUNCH.md` line 15: `(PDF, PNG, JPEG, TIFF, XML)` becomes `(PDF, PNG, JPEG)`.
- `docs/THREAT-MODEL.md` line 36: `(PDF, PNG, JPEG, TIFF, XML; intake tests)` becomes
  `(PDF, PNG, JPEG; intake tests)`.
- Search for any other accepted-type list: `grep -rn "TIFF\|application/xml" docs/ README.md`
  (the XML mentioned by the exporter and the embedded-XML roadmap row is not an upload type and
  stays).
- If `docs/OPERATIONS.md` names upload types, update it the same way.

- [ ] **Step 3: Write the worklog entry**

At the top of `docs/WORKLOG.md`, under the title and before the 3 November entry:

```markdown
## 2026-11-04 (part 84): a dense page gets more room, and the ladder knows the key
**Done**
- **A cut-off may ask for 32 000 tokens.** `RouterPolicy` gains `truncation_ceiling` (default 32 000, at least the answer ceiling, at most the provider's 64 000). A page cut off at 16 000 is retried once at 32 000; a second cut-off ends `too_long`. A policy built with `new` keeps its old ceiling, and a stored carry without the field resumes at the ceiling it started with. The budget check still runs first, so a retry that would breach the R5 document cap ends `budget` instead.
- **A context overflow is its own outcome.** The adapter maps a 400 that says the prompt is too long to `ProviderError::ContextExceeded` (unbilled); the router labels it `escalated` with reason `input_too_large` and the state machine moves to the next rung without ever resending the same input. At the top rung it ends `too_long`.
- **A rung that cannot fit is not called.** The router compares the estimated input plus the largest answer with the model's `max_input_tokens` and skips a rung that cannot hold it, labelled `not_sent` and not counted as a call.
- **The ladder knows the tenant's key.** The server asks the free Models API once per tenant key, caches the answer for an hour, drops models the key cannot use (absent, retired, no structured outputs or no picture) and clamps each rung's answer to what the model reports. A failed check leaves the full ladder in place.
- **A text drop is visible.** A page whose text is above the 200 KB sending limit is still read from its picture, and now records `{ "type": "text_dropped", "bytes": n }` in its notes; the `document.read` audit adds `pages_text_dropped`. No text is stored or logged.
- **Intake matches the worker.** TIFF and XML are refused before the body is read; the API lists PDF, PNG and JPEG, and the SDK and OpenAPI say the same.
- **No migration, no prompt change.** The reading schema stays `reading-v3`, the prompt stays `prompt-v3`, and every new field is additive JSON.
- **Tests:** core (policy and reader, including the stored-carry default and the 14-outcome exhaustive run), provider and adapter mapping, router pre-flight and text drop, pipeline settings narrowing and audit counting, the server's key-to-limits mapping, intake media types, and the contract check.
**Not done:** page parts (Slice B), scans without OCR (B.2), the bounded repair round (C) and coverage evidence (D) are not started; a page that is still too dense after the 32 000 retry still ends `too_long` and is still a person's.
```

- [ ] **Step 4: Run the whole gate**

Run, from the repository root, the commands CI runs (see `.github/workflows/` and `README.md`):

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd packages/sdk && npm test && npm run build && cd ../..
node --check crates/clearcut-server/src/web/app.js
python -m pytest worker/tests        # or the command CI names
```

Expected: every command exits 0. If the test database is not up, start it with the command in
`README.md` and run again. Do not mark this task complete with a failing or skipped suite.

- [ ] **Step 5: Commit, push, open the PR, and merge when green**

```bash
git add docs/
git commit -m "docs: record Slice A of the long-invoice work"
git push -u origin feat/long-invoices-slice-a
gh pr create --title "Long invoices Slice A: honest limits and a capability-aware ladder" --body "$(cat <<'EOF'
## Summary
- a cut-off is retried at a 32 000 token ceiling, with the budget refusing what cannot fit
- a context overflow is its own unbilled error and escalates without resending the input
- a rung whose window cannot hold the call is skipped, not attempted
- the server narrows the ladder once an hour from the free Models API using the tenant's key
- a dropped text layer is recorded on the page and counted in the audit
- intake refuses TIFF and XML, matching the worker and the UI

## Test plan
- [ ] all workspace tests, including the core policy and reader suites
- [ ] the contract check and the SDK build
- [ ] the web and worker tests
EOF
)"
```

Watch the PR's checks. When every job is green, squash-merge it (the repository's delivery rule
allows merging your own increment once the gates pass) and confirm `main` contains the merge.

- [ ] **Step 6: Verify the deployed behaviour, if the server is deployed**

After the merge, if the VM is running the server, follow `docs/ORACLE.md`: pull `main`, rebuild,
and check `/health` answers `{"status":"ok",...}`. Do not upload a real document to prove it: the
change is covered by fakes, and an upload spends the owner's money. Record the `/health` result
in the worklog entry or the PR.

---

## Self-review

**Spec coverage (Slice A, §5 and §11):**

| Spec requirement | Task |
|---|---|
| 5.1 ladder knows the key; clamp to model max; failed check keeps the ladder | 5 |
| 5.2 truncation ceiling 32 000 with the range invariant; stored carry default | 1, 2 |
| 5.3 context overflow as its own error; never retry the same input; pre-flight fit | 3, 4 |
| 5.4 text drop noted and counted | 6 |
| 5.5 intake refuses TIFF and XML with clear copy | 7 |
| §11 tests: skipped model, no structured outputs, 32k retry then too_long, budget refusal, context overflow, text drop, TIFF/XML, stored carry | 1–7 |
| §12 docs and ADR record | 8 |

**Type consistency:** `truncation_ceiling` is `u32` in the policy and the reader; `ModelLimits`
uses `BTreeMap<String, u32>` for output and `BTreeMap<String, u64>` for the window, matching
`ModelInfo::max_tokens` (`u64`) and `RouterConfig.model_input_limits` (`BTreeMap<String, u64>`).
`PageResult::text_dropped_bytes` is `usize`, the same unit as `MAX_PAGE_TEXT_BYTES` and
`String::len()`.

**Placeholder scan:** no `TBD`, no "similar to Task N", no code step without its code. The only
commands that depend on the environment are the CI gate commands, and the plan points at the files
that name them rather than inventing a script.
