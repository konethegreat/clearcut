// Written by Kone Tshivhinda & Claude
use super::*;
use crate::reading::ProblemKind;

// ---------- the outcomes a test feeds in ----------

fn assessment(verdict: Verdict, blocking: u32) -> Assessment {
    Assessment {
        verdict,
        blocking,
        notes: Vec::new(),
        reasons: Vec::new(),
        confidence: ConfidenceBasis::TextConfirmed,
    }
}

fn accept() -> Outcome {
    Outcome::Read(assessment(Verdict::Accept, 0))
}

fn escalate(blocking: u32) -> Outcome {
    Outcome::Read(assessment(
        Verdict::Escalate(EscalationReason::LineArithmetic),
        blocking,
    ))
}

fn escalate_for(reason: EscalationReason) -> Outcome {
    Outcome::Read(assessment(Verdict::Escalate(reason), 1))
}

fn review(reason: ReviewReason) -> Outcome {
    Outcome::Read(assessment(Verdict::Review(reason), 1))
}

fn call(step: &Step) -> &CallPlan {
    match step {
        Step::Call(plan) => plan,
        Step::Done(done) => panic!("expected a call, got {done:?}"),
    }
}

fn done(step: &Step) -> &PageOutcome {
    match step {
        Step::Done(done) => done,
        Step::Call(plan) => panic!("expected the end, got a call: {plan:?}"),
    }
}

fn review_of(step: &Step) -> (ReviewReason, Option<u32>) {
    match done(step) {
        PageOutcome::NeedsReview { reason, draft } => (*reason, *draft),
        other => panic!("expected review, got {other:?}"),
    }
}

fn default_start() -> (PageReader, Step) {
    PageReader::start(&RouterPolicy::default_ladder(), 100)
}

/// Feeds each outcome in turn and returns the last step.
fn feed(reader: &mut PageReader, outcomes: Vec<Outcome>) -> Step {
    let mut last = None;
    for outcome in outcomes {
        last = Some(reader.on_outcome(outcome));
    }
    last.expect("at least one outcome")
}

// ---------- the first call ----------

#[test]
fn the_first_call_is_on_the_first_rung_with_its_settings() {
    let (reader, step) = default_start();
    let plan = call(&step);
    assert_eq!(plan.attempt, 1);
    assert_eq!(plan.rung, 0);
    assert_eq!(plan.model, "claude-haiku-5-5");
    assert_eq!(plan.effort, ThinkingEffort::Medium);
    assert_eq!(plan.max_output_tokens, 16_000);
    assert_eq!(
        plan.picture,
        Picture {
            dpi: 150,
            max_edge: 2_000
        }
    );
    assert_eq!(plan.kind, CallKind::First);
    assert_eq!(reader.calls_made(), 0);
}

#[test]
fn the_first_call_skips_a_rung_that_is_off() {
    let policy = RouterPolicy::default_ladder()
        .with_enabled("claude-haiku-5-5", false)
        .unwrap();
    let (_, step) = PageReader::start(&policy, 100);
    let plan = call(&step);
    assert_eq!(plan.model, "claude-sonnet-5-5");
    assert_eq!(plan.rung, 1);
    assert_eq!(plan.kind, CallKind::First);
}

#[test]
fn with_no_rung_to_use_the_page_goes_to_a_person_at_once() {
    let policy = RouterPolicy::default_ladder();
    let all: Vec<&str> = policy.rungs().iter().map(|r| r.model.as_str()).collect();
    let (_, step) = PageReader::start(&policy.without_models(&all), 100);
    assert_eq!(review_of(&step), (ReviewReason::NoRungAvailable, None));
}

#[test]
fn a_document_with_no_calls_left_does_not_start() {
    let (_, step) = PageReader::start(&RouterPolicy::default_ladder(), 0);
    assert_eq!(review_of(&step), (ReviewReason::CallCap, None));
}

// ---------- accepting ----------

#[test]
fn an_accepted_reading_ends_the_page_with_its_rung_confidence_and_notes() {
    let (mut reader, _) = default_start();
    let notes = vec![Note::DueBeforeIssue];
    let outcome = Outcome::Read(Assessment {
        verdict: Verdict::Accept,
        blocking: 0,
        notes: notes.clone(),
        reasons: Vec::new(),
        confidence: ConfidenceBasis::ArithmeticOnly,
    });
    let step = reader.on_outcome(outcome);
    assert_eq!(
        done(&step),
        &PageOutcome::Read {
            rung: 0,
            attempt: 1,
            confidence: ConfidenceBasis::ArithmeticOnly,
            notes
        }
    );
    assert_eq!(reader.calls_made(), 1);
}

#[test]
fn a_reading_accepted_after_escalation_names_the_rung_and_attempt_that_gave_it() {
    let (mut reader, _) = default_start();
    let step = feed(&mut reader, vec![escalate(2), accept()]);
    match done(&step) {
        PageOutcome::Read { rung, attempt, .. } => assert_eq!((*rung, *attempt), (1, 2)),
        other => panic!("{other:?}"),
    }
}

// ---------- escalating ----------

#[test]
fn each_reason_escalates_to_the_next_rung_and_says_why() {
    for reason in [
        EscalationReason::ReadingProblems,
        EscalationReason::LineArithmetic,
        EscalationReason::TotalsMismatch,
        EscalationReason::TextCheckFailed,
    ] {
        let (mut reader, _) = default_start();
        let step = reader.on_outcome(escalate_for(reason));
        let plan = call(&step);
        assert_eq!(plan.attempt, 2);
        assert_eq!(plan.rung, 1);
        assert_eq!(plan.model, "claude-sonnet-5-5");
        assert_eq!(plan.max_output_tokens, 12_000);
        assert_eq!(plan.kind, CallKind::Escalation { from: 0, reason });
    }
}

#[test]
fn a_page_that_cannot_be_converted_escalates_as_a_reading_problem() {
    let (mut reader, _) = default_start();
    let step = reader.on_outcome(Outcome::NotConverted);
    let plan = call(&step);
    assert_eq!(
        plan.kind,
        CallKind::Escalation {
            from: 0,
            reason: EscalationReason::ReadingProblems
        }
    );
}

#[test]
fn the_upper_rungs_draw_a_larger_picture_and_think_harder() {
    let (mut reader, _) = default_start();
    let step = feed(&mut reader, vec![escalate(1), escalate(1)]);
    let plan = call(&step);
    assert_eq!(plan.model, "claude-opus-5-5");
    assert_eq!(plan.effort, ThinkingEffort::High);
    assert_eq!(
        plan.picture,
        Picture {
            dpi: 200,
            max_edge: 2_576
        }
    );
    assert_eq!(plan.max_output_tokens, 8_000);
}

#[test]
fn failing_the_last_rung_sends_the_page_to_a_person_with_the_best_draft() {
    let (mut reader, _) = default_start();
    let step = feed(&mut reader, vec![escalate(3), escalate(1), escalate(2)]);
    // Blocking issues 3, 1, 2: attempt 2 is the best draft.
    assert_eq!(review_of(&step), (ReviewReason::TopRungFailed, Some(2)));
    assert_eq!(reader.calls_made(), 3);
}

#[test]
fn a_tie_goes_to_the_higher_rung() {
    let (mut reader, _) = default_start();
    let step = feed(&mut reader, vec![escalate(2), escalate(2), escalate(5)]);
    assert_eq!(review_of(&step), (ReviewReason::TopRungFailed, Some(2)));
    let (mut reader, _) = default_start();
    let step = feed(&mut reader, vec![escalate(1), escalate(1), escalate(1)]);
    assert_eq!(review_of(&step), (ReviewReason::TopRungFailed, Some(3)));
}

#[test]
fn readings_that_could_not_be_converted_are_no_draft() {
    let (mut reader, _) = default_start();
    let step = feed(
        &mut reader,
        vec![
            Outcome::NotConverted,
            Outcome::NotConverted,
            Outcome::NotConverted,
        ],
    );
    assert_eq!(review_of(&step), (ReviewReason::TopRungFailed, None));
    // But a draft that came later is still kept.
    let (mut reader, _) = default_start();
    let step = feed(
        &mut reader,
        vec![Outcome::NotConverted, escalate(4), Outcome::NotConverted],
    );
    assert_eq!(review_of(&step), (ReviewReason::TopRungFailed, Some(2)));
}

#[test]
fn with_the_top_rung_switched_on_there_are_four_attempts_before_a_person() {
    let policy = RouterPolicy::default_ladder()
        .with_enabled("claude-fable-5-1", true)
        .unwrap();
    let (mut reader, step) = PageReader::start(&policy, 100);
    assert_eq!(call(&step).model, "claude-haiku-5-5");
    let step = feed(&mut reader, vec![escalate(1), escalate(1), escalate(1)]);
    let plan = call(&step);
    assert_eq!(plan.model, "claude-fable-5-1");
    assert_eq!(plan.attempt, 4);
    assert_eq!(plan.max_output_tokens, 4_000);
    let step = reader.on_outcome(escalate(1));
    assert_eq!(review_of(&step).0, ReviewReason::TopRungFailed);
}

#[test]
fn a_review_verdict_ends_the_page_without_escalating() {
    for reason in [ReviewReason::AmountsIncludeVat, ReviewReason::TooLong] {
        let (mut reader, _) = default_start();
        let step = reader.on_outcome(review(reason));
        assert_eq!(review_of(&step), (reason, Some(1)));
    }
}

// ---------- an answer that was cut off ----------

#[test]
fn an_answer_cut_off_below_the_ceiling_is_retried_once_with_the_ceiling() {
    let policy = RouterPolicy::default_ladder()
        .with_enabled("claude-haiku-5-5", false)
        .unwrap();
    let (mut reader, _) = PageReader::start(&policy, 100);
    let step = reader.on_outcome(Outcome::Truncated);
    let plan = call(&step);
    assert_eq!(plan.kind, CallKind::RetryTruncated);
    assert_eq!(plan.rung, 1);
    assert_eq!(plan.max_output_tokens, 16_000);
    assert_eq!(plan.attempt, 2);
    // Cut off again at the ceiling: the page is too dense for one call.
    let step = reader.on_outcome(Outcome::Truncated);
    assert_eq!(review_of(&step), (ReviewReason::TooLong, None));
}

#[test]
fn an_answer_cut_off_at_the_ceiling_is_too_long_at_once() {
    // The first rung already has the ceiling.
    let (mut reader, _) = default_start();
    let step = reader.on_outcome(Outcome::Truncated);
    assert_eq!(review_of(&step), (ReviewReason::TooLong, None));
}

#[test]
fn a_cut_off_keeps_the_draft_that_came_before_it() {
    let (mut reader, _) = default_start();
    let step = feed(&mut reader, vec![escalate(2), Outcome::Truncated]);
    // Sonnet (12 000) is retried at the ceiling.
    assert_eq!(call(&step).kind, CallKind::RetryTruncated);
    let step = reader.on_outcome(Outcome::Truncated);
    assert_eq!(review_of(&step), (ReviewReason::TooLong, Some(1)));
}

#[test]
fn the_retry_after_a_cut_off_uses_the_raised_cap_only_for_that_rung() {
    let (mut reader, _) = default_start();
    reader.on_outcome(escalate(1)); // now on Sonnet (12 000)
    let step = reader.on_outcome(Outcome::Truncated);
    assert_eq!(call(&step).max_output_tokens, 16_000);
    // The retry is read but fails the checks; the next rung (Opus, 8 000) has its own cap.
    let step = reader.on_outcome(escalate(1));
    let plan = call(&step);
    assert_eq!(plan.model, "claude-opus-5-5");
    assert_eq!(plan.max_output_tokens, 8_000);
}

// ---------- an answer that is not valid ----------

#[test]
fn an_invalid_answer_is_retried_once_on_the_same_rung_then_escalated() {
    let (mut reader, _) = default_start();
    let step = reader.on_outcome(Outcome::InvalidOutput);
    let plan = call(&step);
    assert_eq!(
        (plan.kind, plan.rung, plan.attempt),
        (CallKind::RetryInvalid, 0, 2)
    );
    let step = reader.on_outcome(Outcome::InvalidOutput);
    let plan = call(&step);
    assert_eq!(plan.rung, 1);
    assert_eq!(
        plan.kind,
        CallKind::Escalation {
            from: 0,
            reason: EscalationReason::SchemaInvalid
        }
    );
}

#[test]
fn an_invalid_answer_twice_on_the_top_rung_goes_to_a_person() {
    let policy = RouterPolicy::default_ladder()
        .with_enabled("claude-haiku-5-5", false)
        .unwrap()
        .with_enabled("claude-sonnet-5-5", false)
        .unwrap();
    let (mut reader, _) = PageReader::start(&policy, 100);
    let step = feed(&mut reader, vec![Outcome::InvalidOutput, Outcome::InvalidOutput]);
    assert_eq!(review_of(&step), (ReviewReason::TopRungFailed, None));
}

#[test]
fn a_new_rung_gets_its_own_retries() {
    let (mut reader, _) = default_start();
    reader.on_outcome(Outcome::InvalidOutput); // retry on Haiku
    reader.on_outcome(Outcome::InvalidOutput); // escalate to Sonnet
    let step = reader.on_outcome(Outcome::InvalidOutput);
    assert_eq!(call(&step).kind, CallKind::RetryInvalid);
    assert_eq!(call(&step).rung, 1);
}

// ---------- stopping at once ----------

#[test]
fn a_refusal_a_rejected_request_an_unsafe_page_and_a_refused_budget_end_the_page() {
    for (outcome, reason) in [
        (Outcome::Refused, ReviewReason::Refused),
        (Outcome::Rejected, ReviewReason::RequestRejected),
        (Outcome::CannotBeSentSafely, ReviewReason::CannotBeSentSafely),
        (Outcome::BudgetRefused, ReviewReason::Budget),
    ] {
        let (mut reader, _) = default_start();
        let step = reader.on_outcome(outcome);
        assert_eq!(review_of(&step), (reason, None));
    }
}

#[test]
fn stopping_keeps_the_best_draft_so_far() {
    let (mut reader, _) = default_start();
    reader.on_outcome(escalate(3));
    let step = reader.on_outcome(Outcome::Refused);
    assert_eq!(review_of(&step), (ReviewReason::Refused, Some(1)));
}

#[test]
fn a_call_that_never_happened_is_not_counted() {
    for outcome in [Outcome::BudgetRefused, Outcome::CannotBeSentSafely] {
        let (mut reader, _) = default_start();
        reader.on_outcome(outcome);
        assert_eq!(reader.calls_made(), 0);
    }
    let (mut reader, _) = default_start();
    reader.on_outcome(Outcome::Refused);
    assert_eq!(reader.calls_made(), 1);
}

// ---------- a model the key cannot use ----------

#[test]
fn a_rung_the_key_cannot_use_is_skipped() {
    let (mut reader, _) = default_start();
    let step = reader.on_outcome(Outcome::RungUnavailable);
    let plan = call(&step);
    assert_eq!(plan.model, "claude-sonnet-5-5");
    assert_eq!(
        plan.kind,
        CallKind::Escalation {
            from: 0,
            reason: EscalationReason::RungUnavailable
        }
    );
}

#[test]
fn when_no_rung_can_be_used_there_is_none_available() {
    let (mut reader, _) = default_start();
    let step = feed(
        &mut reader,
        vec![
            Outcome::RungUnavailable,
            Outcome::RungUnavailable,
            Outcome::RungUnavailable,
        ],
    );
    assert_eq!(review_of(&step), (ReviewReason::NoRungAvailable, None));
}

#[test]
fn when_the_last_rung_is_unavailable_after_a_failed_reading_it_is_the_top_that_failed() {
    let (mut reader, _) = default_start();
    let step = feed(
        &mut reader,
        vec![escalate(2), escalate(1), Outcome::RungUnavailable],
    );
    assert_eq!(review_of(&step), (ReviewReason::TopRungFailed, Some(2)));
}

// ---------- the caps ----------

fn capped(per_page: u32) -> RouterPolicy {
    let base = RouterPolicy::default_ladder();
    RouterPolicy::new(base.rungs().to_vec(), 16_000, per_page, 3, 10).unwrap()
}

#[test]
fn the_page_cap_stops_escalation_before_the_ladder_does() {
    let (mut reader, _) = PageReader::start(&capped(2), 100);
    let step = feed(&mut reader, vec![escalate(1), escalate(1)]);
    // Opus remains, but two calls were allowed.
    assert_eq!(review_of(&step), (ReviewReason::CallCap, Some(2)));
}

#[test]
fn the_document_cap_stops_a_page_when_the_documents_calls_are_used() {
    let (mut reader, step) = PageReader::start(&RouterPolicy::default_ladder(), 1);
    assert!(matches!(step, Step::Call(_)));
    let step = reader.on_outcome(escalate(1));
    assert_eq!(review_of(&step), (ReviewReason::CallCap, Some(1)));
}

#[test]
fn retries_count_against_the_caps() {
    let policy = capped(2);
    let (mut reader, _) = PageReader::start(&policy, 100);
    reader.on_outcome(Outcome::InvalidOutput); // the retry is call 2
    let step = reader.on_outcome(Outcome::InvalidOutput); // would escalate, but the cap is reached
    assert_eq!(review_of(&step), (ReviewReason::CallCap, None));
}

#[test]
fn a_cap_does_not_hide_a_better_reason() {
    // An answer cut off twice is too long whatever the caps say.
    let policy = RouterPolicy::default_ladder()
        .with_enabled("claude-haiku-5-5", false)
        .unwrap();
    let (mut reader, _) = PageReader::start(&policy, 2);
    reader.on_outcome(Outcome::Truncated);
    let step = reader.on_outcome(Outcome::Truncated);
    assert_eq!(review_of(&step), (ReviewReason::TooLong, None));
}

// ---------- the end is the end ----------

#[test]
fn once_a_page_is_done_further_outcomes_change_nothing() {
    let (mut reader, _) = default_start();
    let first = reader.on_outcome(accept());
    let calls = reader.calls_made();
    assert_eq!(reader.on_outcome(escalate(1)), first);
    assert_eq!(reader.on_outcome(Outcome::Truncated), first);
    assert_eq!(reader.calls_made(), calls);
    assert!(reader.is_done());
}

#[test]
fn attempts_are_numbered_in_order() {
    let (mut reader, step) = default_start();
    let mut seen = vec![call(&step).attempt];
    for outcome in [Outcome::InvalidOutput, escalate(1), Outcome::Truncated] {
        let step = reader.on_outcome(outcome);
        if let Step::Call(plan) = &step {
            seen.push(plan.attempt);
        }
    }
    assert_eq!(seen, [1, 2, 3, 4]);
}

// ---------- it can be stored and picked up again ----------

#[test]
fn a_reader_survives_being_stored_between_two_calls() {
    let (mut reader, _) = default_start();
    reader.on_outcome(escalate(3));
    reader.on_outcome(Outcome::InvalidOutput);
    let json = serde_json::to_string(&reader).unwrap();
    let mut restored: PageReader = serde_json::from_str(&json).unwrap();
    assert_eq!(restored, reader);
    // Both carry on identically.
    for outcome in [escalate(1), accept()] {
        assert_eq!(restored.on_outcome(outcome.clone()), reader.on_outcome(outcome));
    }
}

#[test]
fn the_stored_form_holds_no_text() {
    let (mut reader, _) = default_start();
    reader.on_outcome(Outcome::Read(Assessment {
        verdict: Verdict::Escalate(EscalationReason::ReadingProblems),
        blocking: 1,
        notes: vec![Note::NonCriticalProblem {
            path: "recipient/name".to_string(),
            kind: ProblemKind::TooLong,
        }],
        reasons: vec![EscalationReason::ReadingProblems],
        confidence: ConfidenceBasis::Unresolved,
    }));
    let json = serde_json::to_string(&reader).unwrap();
    // Model ids, counters and fixed words only.
    assert!(json.len() < 2_000, "{json}");
    assert!(!json.contains("Invoice"));
}

// ---------- it always ends ----------

fn alphabet() -> Vec<Outcome> {
    vec![
        accept(),
        escalate(1),
        escalate_for(EscalationReason::TextCheckFailed),
        review(ReviewReason::TooLong),
        Outcome::NotConverted,
        Outcome::Truncated,
        Outcome::InvalidOutput,
        Outcome::Refused,
        Outcome::Rejected,
        Outcome::CannotBeSentSafely,
        Outcome::BudgetRefused,
        Outcome::RungUnavailable,
    ]
}

/// Runs `outcomes` (cycling the picks) until the page ends; returns how many calls were issued.
fn run(policy: &RouterPolicy, doc_calls: u32, picks: &[usize], letters: &[Outcome]) -> (u32, bool) {
    let (mut reader, mut step) = PageReader::start(policy, doc_calls);
    let mut issued = u32::from(matches!(step, Step::Call(_)));
    let mut turns = 0;
    while let Step::Call(_) = step {
        let pick = picks[turns % picks.len()];
        step = reader.on_outcome(letters[pick].clone());
        if matches!(step, Step::Call(_)) {
            issued += 1;
        }
        turns += 1;
        assert!(turns <= 64, "the page did not end");
    }
    (issued, reader.is_done())
}

#[test]
fn every_sequence_of_outcomes_ends_within_the_page_cap() {
    // All 12^5 sequences of five outcomes: the default cap is five calls, so each must be over.
    let policy = RouterPolicy::default_ladder();
    let letters = alphabet();
    let n = letters.len();
    let mut picks = [0usize; 5];
    let mut count = 0u32;
    loop {
        let (issued, finished) = run(&policy, 1_000, &picks, &letters);
        assert!(finished, "{picks:?}");
        assert!(issued <= policy.max_calls_per_page(), "{picks:?} issued {issued}");
        count += 1;
        // Next sequence, counting in base n.
        let mut i = 0;
        loop {
            picks[i] += 1;
            if picks[i] < n {
                break;
            }
            picks[i] = 0;
            i += 1;
            if i == picks.len() {
                assert_eq!(count, u32::try_from(n.pow(5)).unwrap());
                return;
            }
        }
    }
}

#[test]
fn generated_policies_and_outcomes_never_exceed_a_cap() {
    // A small deterministic generator, so the test is the same every time.
    let mut seed = 0x9E37_79B9_7F4A_7C15_u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let base = RouterPolicy::default_ladder();
    let letters = alphabet();
    for _ in 0..20_000 {
        let per_page = u32::try_from(next() % 6).unwrap() + 1;
        let doc_calls = u32::try_from(next() % 8).unwrap();
        let mut policy = RouterPolicy::new(base.rungs().to_vec(), 16_000, per_page, 3, 10).unwrap();
        if next() % 2 == 0 {
            policy = policy.with_enabled("claude-fable-5-1", true).unwrap();
        }
        if next() % 3 == 0 {
            policy = policy.without_models(&["claude-sonnet-5-5"]);
        }
        let picks: Vec<usize> = (0..8).map(|_| usize::try_from(next() % 12).unwrap()).collect();
        let (issued, finished) = run(&policy, doc_calls, &picks, &letters);
        assert!(finished);
        assert!(issued <= per_page, "{issued} calls with a cap of {per_page}");
        assert!(issued <= doc_calls, "{issued} calls with {doc_calls} left");
    }
}

// ---------- rungs are named by their place in the policy, not among the ones that are on ----------

fn without_haiku() -> RouterPolicy {
    RouterPolicy::default_ladder()
        .with_enabled("claude-haiku-5-5", false)
        .unwrap()
}

#[test]
fn an_accepted_reading_names_its_rung_by_the_policy_even_when_an_earlier_rung_is_off() {
    let (mut reader, _) = PageReader::start(&without_haiku(), 100);
    match done(&reader.on_outcome(accept())) {
        PageOutcome::Read { rung, attempt, .. } => assert_eq!((*rung, *attempt), (1, 1)),
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_escalation_names_the_rung_it_came_from_by_the_policy() {
    let (mut reader, _) = PageReader::start(&without_haiku(), 100);
    let step = reader.on_outcome(escalate(1));
    let plan = call(&step);
    assert_eq!(plan.rung, 2);
    assert_eq!(
        plan.kind,
        CallKind::Escalation {
            from: 1,
            reason: EscalationReason::LineArithmetic
        }
    );
}

// ---------- a reader read back is checked, so a stored one cannot panic or overspend ----------

fn tampered_reader(
    reader: &PageReader,
    change: impl FnOnce(&mut serde_json::Value),
) -> Result<PageReader, serde_json::Error> {
    let mut value = serde_json::to_value(reader).unwrap();
    change(&mut value);
    serde_json::from_value(value)
}

#[test]
fn a_finished_page_with_no_rungs_can_still_be_stored_and_read_back() {
    let policy = RouterPolicy::default_ladder();
    let all: Vec<&str> = policy.rungs().iter().map(|r| r.model.as_str()).collect();
    let (reader, step) = PageReader::start(&policy.without_models(&all), 100);
    assert_eq!(review_of(&step).0, ReviewReason::NoRungAvailable);
    let back: PageReader = serde_json::from_str(&serde_json::to_string(&reader).unwrap()).unwrap();
    assert_eq!(back, reader);
    assert!(back.is_done());
}

#[test]
fn a_stored_reader_that_breaks_its_own_rules_is_refused() {
    use serde_json::json;
    let (mut reader, _) = default_start();
    reader.on_outcome(escalate(1));
    assert!(tampered_reader(&reader, |_| {}).is_ok());

    let cases: Vec<(&str, Tamper)> = vec![
        (
            "a position past the rungs",
            Box::new(|v| v["position"] = json!(9)),
        ),
        (
            "no rungs on an unfinished page",
            Box::new(|v| v["rungs"] = json!([])),
        ),
        (
            "more calls than the limit",
            Box::new(|v| {
                v["calls_made"] = json!(6);
                v["issued"] = json!(6);
            }),
        ),
        (
            "more calls made than plans issued",
            Box::new(|v| v["calls_made"] = json!(5)),
        ),
        (
            "a cap raised past the ceiling",
            Box::new(|v| v["cap_override"] = json!(16_001)),
        ),
        ("a raised cap of zero", Box::new(|v| v["cap_override"] = json!(0))),
        ("a best attempt of zero", Box::new(|v| v["best"] = json!([0, 1]))),
        (
            "a ceiling past the provider's",
            Box::new(|v| v["ceiling"] = json!(64_001)),
        ),
        (
            "a best attempt that was never issued",
            Box::new(|v| v["best"] = json!([9, 1])),
        ),
        (
            "a model id with a space",
            Box::new(|v| v["rungs"][0]["rung"]["model"] = json!("has space")),
        ),
        (
            "an answer cap over the ceiling",
            Box::new(|v| v["rungs"][0]["rung"]["max_output_tokens"] = json!(16_001)),
        ),
        (
            "a picture the worker would refuse",
            Box::new(|v| v["rungs"][0]["rung"]["picture"]["max_edge"] = json!(9_999)),
        ),
    ];
    for (case, change) in cases {
        assert!(tampered_reader(&reader, change).is_err(), "{case}");
    }
}

#[test]
fn a_reader_read_back_after_every_step_behaves_like_the_original() {
    // Store and restore between every two outcomes of a long script.
    let script = [
        escalate(3),
        Outcome::InvalidOutput,
        Outcome::Truncated,
        escalate(1),
        accept(),
    ];
    let (mut live, _) = default_start();
    let (mut restored, _) = default_start();
    for outcome in script {
        let json = serde_json::to_string(&restored).unwrap();
        restored = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.on_outcome(outcome.clone()), live.on_outcome(outcome));
    }
    assert_eq!(restored, live);
}

/// A change to make to a stored value, to see whether it is still accepted.
type Tamper = Box<dyn FnOnce(&mut serde_json::Value)>;

#[test]
fn a_page_that_ended_exactly_at_its_limit_with_its_last_attempt_as_the_draft_reads_back() {
    // Boundary values that are valid: calls made equal to the limit, the best attempt equal to
    // the last one issued.
    let base = RouterPolicy::default_ladder();
    let policy = RouterPolicy::new(base.rungs().to_vec(), 16_000, 2, 3, 10).unwrap();
    let (mut reader, _) = PageReader::start(&policy, 100);
    let step = feed(&mut reader, vec![escalate(1), escalate(1)]);
    assert_eq!(review_of(&step), (ReviewReason::CallCap, Some(2)));
    assert_eq!(reader.calls_made(), 2);
    let back: PageReader = serde_json::from_str(&serde_json::to_string(&reader).unwrap()).unwrap();
    assert_eq!(back, reader);
    // A raised cap at exactly the ceiling is valid too.
    let (mut raised, _) = PageReader::start(&without_haiku(), 100);
    raised.on_outcome(Outcome::Truncated);
    let back: PageReader = serde_json::from_str(&serde_json::to_string(&raised).unwrap()).unwrap();
    assert_eq!(back, raised);
}
