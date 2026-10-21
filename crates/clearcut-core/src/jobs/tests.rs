// Written by Kone Tshivhinda & Claude
use super::*;
use crate::document::DocumentStatus;
use crate::Error;
use std::time::Duration;

const SECOND: Duration = Duration::from_secs(1);

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

// ---------- kinds ----------

#[test]
fn the_job_kinds_and_their_stored_names_are_exactly_the_agreed_list() {
    // ADR-0008. The names are stored in the database, so changing one is a
    // breaking change.
    assert_eq!(JobKind::ALL, [JobKind::ProcessDocument]);
    assert_eq!(JobKind::ProcessDocument.as_str(), "process_document");
}

#[test]
fn a_kind_is_parsed_exactly_or_refused() {
    assert_eq!(JobKind::parse("process_document"), Ok(JobKind::ProcessDocument));
    for bad in [
        "",
        "Process_Document",
        " process_document",
        "process_document ",
        "process",
        "*",
    ] {
        assert_eq!(
            JobKind::parse(bad),
            Err(Error::UnknownJobKind(bad.to_string())),
            "{bad:?}"
        );
    }
}

// ---------- states ----------

#[test]
fn the_states_and_their_stored_names_are_exactly_the_agreed_list() {
    let expected = [
        (JobState::Queued, "queued"),
        (JobState::Running, "running"),
        (JobState::Succeeded, "succeeded"),
        (JobState::NeedsReview, "needs_review"),
        (JobState::Cancelled, "cancelled"),
    ];
    assert_eq!(JobState::ALL.len(), expected.len());
    for (state, name) in expected {
        assert!(JobState::ALL.contains(&state));
        assert_eq!(state.as_str(), name);
        assert_eq!(JobState::parse(name), Ok(state));
        assert_eq!(serde_json::to_string(&state).unwrap(), format!("\"{name}\""));
        assert_eq!(
            serde_json::from_str::<JobState>(&format!("\"{name}\"")).unwrap(),
            state
        );
    }
}

#[test]
fn there_is_no_failed_state() {
    // ADR-0005/0008: a job that cannot be completed ends needs_review.
    assert_eq!(
        JobState::parse("failed"),
        Err(Error::UnknownJobState("failed".into()))
    );
    assert!(serde_json::from_str::<JobState>("\"failed\"").is_err());
}

#[test]
fn a_state_is_parsed_exactly_or_refused() {
    for bad in ["", "Queued", " queued", "queued ", "needs-review", "done"] {
        assert_eq!(
            JobState::parse(bad),
            Err(Error::UnknownJobState(bad.to_string())),
            "{bad:?}"
        );
    }
}

#[test]
fn exactly_three_states_are_terminal() {
    let terminal: Vec<_> = JobState::ALL.into_iter().filter(|s| s.is_terminal()).collect();
    assert_eq!(
        terminal,
        [JobState::Succeeded, JobState::NeedsReview, JobState::Cancelled]
    );
}

#[test]
fn the_legal_transitions_are_exactly_the_agreed_ones() {
    use JobState::*;
    // (from, to): claim, retry/defer, re-claim after a lost lease, finish,
    // cancel before it runs, and the deadline sweep.
    let legal = [
        (Queued, Running),
        (Queued, Cancelled),
        (Queued, NeedsReview),
        (Running, Running),
        (Running, Queued),
        (Running, Succeeded),
        (Running, NeedsReview),
        (Running, Cancelled),
    ];
    for from in JobState::ALL {
        for to in JobState::ALL {
            let expected = legal.contains(&(from, to));
            assert_eq!(from.can_move_to(to), expected, "{from:?} -> {to:?}");
            match from.transition(to) {
                Ok(next) => assert!(expected && next == to, "{from:?} -> {to:?}"),
                Err(error) => {
                    assert!(!expected, "{from:?} -> {to:?}");
                    assert_eq!(error, Error::InvalidJobTransition { from, to });
                }
            }
        }
    }
}

#[test]
fn a_finished_job_never_moves_again() {
    for from in JobState::ALL.into_iter().filter(|s| s.is_terminal()) {
        for to in JobState::ALL {
            assert!(!from.can_move_to(to), "{from:?} -> {to:?}");
        }
    }
}

// ---------- the document moves with the job ----------

#[test]
fn each_job_state_has_the_agreed_effect_on_its_document() {
    assert_eq!(JobState::Queued.document_status(), None);
    assert_eq!(
        JobState::Running.document_status(),
        Some(DocumentStatus::Processing)
    );
    assert_eq!(JobState::Succeeded.document_status(), Some(DocumentStatus::Done));
    assert_eq!(
        JobState::NeedsReview.document_status(),
        Some(DocumentStatus::NeedsReview)
    );
    // A cancelled document can be processed again later.
    assert_eq!(
        JobState::Cancelled.document_status(),
        Some(DocumentStatus::Received)
    );
}

#[test]
fn a_job_never_leaves_its_document_without_a_way_forward() {
    // Every terminal job state moves the document somewhere.
    for state in JobState::ALL.into_iter().filter(|s| s.is_terminal()) {
        assert!(state.document_status().is_some(), "{state:?}");
    }
}

#[test]
fn an_outcome_is_a_finishing_state() {
    assert_eq!(Outcome::Succeeded.state(), JobState::Succeeded);
    assert_eq!(Outcome::NeedsReview.state(), JobState::NeedsReview);
    assert_eq!(Outcome::Cancelled.state(), JobState::Cancelled);
    for outcome in [Outcome::Succeeded, Outcome::NeedsReview, Outcome::Cancelled] {
        assert!(outcome.state().is_terminal());
        assert_eq!(Some(outcome.document_status()), outcome.state().document_status());
        assert_eq!(outcome.as_str(), outcome.state().as_str());
    }
}

// ---------- retry policy ----------

#[test]
fn the_default_policy_is_the_agreed_one() {
    let p = RetryPolicy::default();
    assert_eq!(p.max_attempts(), 5);
    assert_eq!(p.base_delay(), secs(30));
    assert_eq!(p.max_delay(), secs(15 * 60));
    assert_eq!(p.provider_delay(), secs(5 * 60));
    assert_eq!(p.jitter_bp(), 2000);
}

#[test]
fn the_backoff_doubles_from_the_base_up_to_the_cap() {
    let p = RetryPolicy::default();
    let expected = [30, 60, 120, 240, 480, 900, 900, 900];
    for (i, want) in expected.into_iter().enumerate() {
        assert_eq!(p.backoff(i as u32 + 1), secs(want), "attempt {}", i + 1);
    }
}

#[test]
fn the_backoff_cannot_overflow() {
    let p = RetryPolicy::default();
    assert_eq!(p.backoff(0), secs(30), "attempt 0 is treated as the first");
    assert_eq!(p.backoff(64), secs(900));
    assert_eq!(p.backoff(u32::MAX), secs(900));
}

#[test]
fn jitter_spreads_a_delay_by_a_fifth_either_way() {
    let p = RetryPolicy::default();
    let at = |unit| p.retry_delay(1, unit);
    assert_eq!(at(0), secs(24));
    assert_eq!(at(5000), secs(30));
    assert_eq!(at(10_000), secs(36));
    assert_eq!(at(10_001), secs(36), "a sample above 100% is clamped");
    assert_eq!(at(u32::MAX), secs(36));
    assert!(at(2500) < at(5000) && at(5000) < at(7500));
}

#[test]
fn a_jittered_delay_never_exceeds_the_cap() {
    let p = RetryPolicy::default();
    for attempt in 1..=12 {
        for unit in [0, 5000, 10_000] {
            assert!(
                p.retry_delay(attempt, unit) <= p.max_delay(),
                "attempt {attempt} unit {unit}"
            );
        }
    }
    assert_eq!(p.retry_delay(12, 10_000), p.max_delay());
}

#[test]
fn a_transient_failure_retries_and_counts_until_the_attempts_run_out() {
    let p = RetryPolicy::default();
    for attempt in 1..=4 {
        assert_eq!(
            p.decide(FailureKind::Transient, attempt, 5000),
            Decision::Retry {
                after: p.backoff(attempt),
                counts: true
            },
            "attempt {attempt}"
        );
    }
    for attempt in [5, 6, 100, u32::MAX] {
        assert_eq!(
            p.decide(FailureKind::Transient, attempt, 5000),
            Decision::NeedsReview,
            "attempt {attempt}"
        );
    }
}

#[test]
fn a_provider_outage_never_uses_up_attempts_and_never_ends_the_job() {
    let p = RetryPolicy::default();
    for attempt in [1, 4, 5, 6, 1000, u32::MAX] {
        assert_eq!(
            p.decide(FailureKind::ProviderUnavailable, attempt, 5000),
            Decision::Retry {
                after: secs(300),
                counts: false
            },
            "attempt {attempt}"
        );
    }
}

#[test]
fn a_provider_outage_retry_is_jittered_too() {
    let p = RetryPolicy::default();
    let after = |unit| match p.decide(FailureKind::ProviderUnavailable, 1, unit) {
        Decision::Retry { after, .. } => after,
        other => panic!("{other:?}"),
    };
    assert_eq!(after(0), secs(240));
    assert_eq!(after(10_000), secs(360));
}

#[test]
fn a_page_that_failed_validation_at_the_top_tier_goes_straight_to_review() {
    let p = RetryPolicy::default();
    for attempt in [1, 2, 5] {
        assert_eq!(
            p.decide(FailureKind::NeedsReview, attempt, 0),
            Decision::NeedsReview
        );
    }
}

#[test]
fn attempts_are_exhausted_exactly_at_the_maximum() {
    let p = RetryPolicy::default();
    assert!(!p.is_exhausted(4));
    assert!(p.is_exhausted(5));
    assert!(p.is_exhausted(6));
}

#[test]
fn a_policy_is_checked_when_it_is_built() {
    let ok = RetryPolicy::new(3, secs(10), secs(100), secs(60), 1000);
    assert!(ok.is_ok());
    // The database allows 1 to 20 attempts.
    assert!(RetryPolicy::new(1, secs(10), secs(100), secs(60), 0).is_ok());
    assert!(RetryPolicy::new(20, secs(10), secs(100), secs(60), 10_000).is_ok());
    assert!(RetryPolicy::new(0, secs(10), secs(100), secs(60), 0).is_err());
    assert!(RetryPolicy::new(21, secs(10), secs(100), secs(60), 0).is_err());
    assert!(RetryPolicy::new(3, Duration::ZERO, secs(100), secs(60), 0).is_err());
    assert!(
        RetryPolicy::new(3, secs(10), secs(5), secs(60), 0).is_err(),
        "cap below base"
    );
    assert!(
        RetryPolicy::new(3, secs(10), secs(10), secs(60), 0).is_ok(),
        "cap equal to base"
    );
    assert!(RetryPolicy::new(3, secs(10), secs(100), Duration::ZERO, 0).is_err());
    assert!(RetryPolicy::new(3, secs(10), secs(100), secs(60), 10_001).is_err());
}

// ---------- lease and deadline ----------

#[test]
fn the_default_lease_is_five_minutes_with_a_heartbeat_every_hundred_seconds() {
    let lease = LeasePolicy::default();
    assert_eq!(lease.duration(), secs(300));
    assert_eq!(lease.heartbeat_interval(), secs(100));
}

#[test]
fn the_heartbeat_comes_well_before_the_lease_runs_out() {
    for secs_ in [3, 10, 60, 300, 3600] {
        let lease = LeasePolicy::new(secs(secs_)).unwrap();
        assert!(
            lease.heartbeat_interval() * 2 < lease.duration() + SECOND,
            "{secs_}"
        );
        assert!(lease.heartbeat_interval() >= SECOND);
    }
}

#[test]
fn a_lease_too_short_to_heartbeat_is_refused() {
    assert!(LeasePolicy::new(Duration::ZERO).is_err());
    assert!(LeasePolicy::new(secs(2)).is_err());
    assert!(LeasePolicy::new(secs(3)).is_ok());
}

#[test]
fn the_default_deadline_is_forty_eight_hours() {
    assert_eq!(DEFAULT_DEADLINE, secs(48 * 3600));
}

// ---------- progress ----------

#[test]
fn progress_cannot_be_ahead_of_its_total() {
    assert!(Progress::new(3, Some(10)).is_ok());
    assert!(Progress::new(10, Some(10)).is_ok());
    assert_eq!(
        Progress::new(11, Some(10)),
        Err(Error::InvalidJobProgress("done is more than total"))
    );
    assert!(Progress::new(7, None).is_ok(), "the total may not be known yet");
}

#[test]
fn progress_reports_a_whole_percentage_when_it_can() {
    let pct = |done, total| Progress::new(done, total).unwrap().percent();
    assert_eq!(pct(0, Some(10)), Some(0));
    assert_eq!(pct(3, Some(10)), Some(30));
    assert_eq!(pct(1, Some(3)), Some(33), "rounded down");
    assert_eq!(pct(10, Some(10)), Some(100));
    assert_eq!(pct(0, Some(0)), None, "nothing to measure against");
    assert_eq!(pct(5, None), None);
    assert_eq!(pct(u32::MAX, Some(u32::MAX)), Some(100), "no overflow");
}

// ---------- text that goes into the database ----------

#[test]
fn an_error_message_is_made_safe_for_the_database_column() {
    assert_eq!(
        short_error("  the   worker \n timed\tout  "),
        "the worker timed out"
    );
    assert_eq!(short_error("bell\u{7}and\u{0}nul"), "bellandnul");
    assert_eq!(short_error(""), "");
    assert_eq!(short_error("   \n "), "");
}

#[test]
fn an_error_message_is_cut_to_the_column_limit_on_a_character_boundary() {
    assert_eq!(MAX_ERROR_CHARS, 500);
    let exact = "e".repeat(500);
    assert_eq!(short_error(&exact), exact, "exactly at the limit is kept whole");
    let long = "é".repeat(501);
    let cut = short_error(&long);
    assert_eq!(cut.chars().count(), 500);
    assert!(cut.ends_with('…'));
    assert!(cut.starts_with("éé"));
}

#[test]
fn a_progress_note_is_cut_to_its_own_limit() {
    assert_eq!(MAX_NOTE_CHARS, 200);
    let cut = short_note(&"n".repeat(300));
    assert_eq!(cut.chars().count(), 200);
    assert!(cut.ends_with('…'));
    assert_eq!(short_note("page 4 of 12"), "page 4 of 12");
}
