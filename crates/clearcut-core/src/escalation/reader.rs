// Written by Kone Tshivhinda & Claude
//! The page state machine (ADR-0018 section 1).
//!
//! The async driver asks "what now?", does it, and says what happened:
//!
//! ```text
//! let (mut reader, mut step) = PageReader::start(&policy, calls_left_in_the_document);
//! while let Step::Call(plan) = step {
//!     let outcome = /* make the call in `plan`, convert, validate, assess */;
//!     step = reader.on_outcome(outcome);
//! }
//! ```
//!
//! The machine holds no document text and no money. It counts calls, remembers the rung and
//! the best attempt so far, and every path through it ends: a rung is left or a counter grows
//! with each outcome, and the caps stop the rest. The tests check every sequence of five
//! outcomes and twenty thousand generated policies.
//!
//! Failures of the *transport* (the provider unavailable, a timeout, a key problem) never
//! reach it: the adapter has already retried them and the job queue retries after that, so the
//! driver stops the document there and resumes from the stored machine (it is `Serialize`).

use super::policy::{check_ceiling, check_rung};
use super::{
    Assessment, ConfidenceBasis, EscalationReason, Note, Picture, ReviewReason, RouterPolicy, Rung,
    ThinkingEffort, Verdict,
};
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};

/// Why a call is being made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CallKind {
    /// The first call on the page.
    First,
    /// The previous rung's reading failed (or the rung could not be used); this is the next.
    /// The driver attaches a failure report in ClearCut's own vocabulary (ADR-0014).
    Escalation { from: usize, reason: EscalationReason },
    /// The answer was cut off; the same rung again with the largest answer allowed.
    RetryTruncated,
    /// The answer was not valid against the schema; the same rung once more.
    RetryInvalid,
}

/// One call to make.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallPlan {
    /// 1, 2, 3 ... in the order the calls of this page are made.
    pub attempt: u32,
    /// The rung's position in the policy's `rungs()`.
    pub rung: usize,
    pub model: String,
    pub effort: ThinkingEffort,
    pub max_output_tokens: u32,
    pub picture: Picture,
    pub kind: CallKind,
}

/// What happened to the last call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Outcome {
    /// The model answered, the answer was converted and checked.
    Read(Assessment),
    /// The answer could not be converted at all (no currency, or one that differs from the
    /// rest of the document).
    NotConverted,
    /// Cut off at the answer cap.
    Truncated,
    /// Not valid against the schema, or not JSON.
    InvalidOutput,
    /// The model declined.
    Refused,
    /// The provider rejected our request (a bug of ours).
    Rejected,
    /// Nothing was sent: the page cannot be sent safely under the strict redaction policy.
    CannotBeSentSafely,
    /// Nothing was sent: the call would have broken a budget.
    BudgetRefused,
    /// The key cannot use the model.
    RungUnavailable,
}

/// How a page ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PageOutcome {
    Read {
        /// The rung (a position in the policy's `rungs()`) and the attempt that gave the reading.
        rung: usize,
        attempt: u32,
        confidence: ConfidenceBasis,
        notes: Vec<Note>,
    },
    NeedsReview {
        reason: ReviewReason,
        /// The attempt with the fewest blocking issues, to start a reviewer from; ties go to
        /// the later (higher) rung. `None` if no attempt could be converted.
        draft: Option<u32>,
    },
}

/// What to do next.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Step {
    Call(CallPlan),
    Done(PageOutcome),
}

/// A rung that is on, with its position in the policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Planned {
    index: usize,
    rung: Rung,
}

/// The state of one page. **Read back from text, it is checked** (`try_from`): a stored
/// reader that has been damaged or edited is refused instead of panicking on an index or
/// spending more calls than the page was allowed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "UncheckedReader")]
pub struct PageReader {
    rungs: Vec<Planned>,
    ceiling: u32,
    /// The fewer of the page's cap and the calls the document has left.
    call_limit: u32,
    /// Where in `rungs` the current rung is.
    position: usize,
    /// Calls actually made. A call that was refused before it was sent is not one.
    calls_made: u32,
    /// Plans handed out; the number of the last one is the attempt number of the outcome.
    issued: u32,
    /// Whether the current rung has already had its invalid-answer retry.
    invalid_retried: bool,
    /// The answer cap in use on the current rung, once a cut-off raised it to the ceiling. That
    /// is also what stops a second cut-off retry: the cap is then no longer below the ceiling.
    cap_override: Option<u32>,
    /// The attempt with the fewest blocking issues so far, and how many it had.
    best: Option<(u32, u32)>,
    /// The call handed out and not yet answered. A page that is not finished always has one,
    /// and a page that is finished never does: after a failure of the transport (nothing came
    /// back) a stored page makes the same call again.
    pending: Option<CallPlan>,
    finished: Option<PageOutcome>,
}

/// A reader as it is stored, before it is checked.
#[derive(Deserialize)]
struct UncheckedReader {
    rungs: Vec<Planned>,
    ceiling: u32,
    call_limit: u32,
    position: usize,
    calls_made: u32,
    issued: u32,
    invalid_retried: bool,
    cap_override: Option<u32>,
    best: Option<(u32, u32)>,
    pending: Option<CallPlan>,
    finished: Option<PageOutcome>,
}

fn broken(why: &'static str) -> Error {
    Error::InvalidStoredReader(why)
}

impl TryFrom<UncheckedReader> for PageReader {
    type Error = Error;

    fn try_from(raw: UncheckedReader) -> Result<Self> {
        check_ceiling(raw.ceiling).map_err(|_| broken("the answer ceiling is not one the provider takes"))?;
        for planned in &raw.rungs {
            check_rung(&planned.rung, raw.ceiling)
                .map_err(|_| broken("a rung is not one a policy could hold"))?;
        }
        // A page that is still being read must have a current rung to call.
        if raw.finished.is_none() && raw.position >= raw.rungs.len() {
            return Err(broken("the position is outside the rungs"));
        }
        if raw.calls_made > raw.call_limit {
            return Err(broken("more calls were made than the limit"));
        }
        if raw.calls_made > raw.issued {
            return Err(broken("more calls were made than were planned"));
        }
        if raw.cap_override.is_some_and(|cap| cap == 0 || cap > raw.ceiling) {
            return Err(broken("the raised answer cap is outside 1 to the ceiling"));
        }
        if raw
            .best
            .is_some_and(|(attempt, _)| attempt == 0 || attempt > raw.issued)
        {
            return Err(broken("the best attempt was never planned"));
        }
        match (&raw.finished, &raw.pending) {
            (Some(_), Some(_)) => return Err(broken("a finished page is waiting for a call")),
            (None, None) => return Err(broken("an unfinished page is waiting for no call")),
            (None, Some(plan)) => {
                let current = &raw.rungs[raw.position];
                if plan.attempt != raw.issued
                    || plan.rung != current.index
                    || plan.model != current.rung.model
                {
                    return Err(broken("the waiting call is not the current rung's call"));
                }
            }
            (Some(_), None) => {}
        }
        Ok(PageReader {
            rungs: raw.rungs,
            ceiling: raw.ceiling,
            call_limit: raw.call_limit,
            position: raw.position,
            calls_made: raw.calls_made,
            issued: raw.issued,
            invalid_retried: raw.invalid_retried,
            cap_override: raw.cap_override,
            best: raw.best,
            pending: raw.pending,
            finished: raw.finished,
        })
    }
}

impl PageReader {
    /// Starts a page. `document_calls_left` is what the document may still spend in calls
    /// (`RouterPolicy::document_call_cap` less what earlier pages used).
    pub fn start(policy: &RouterPolicy, document_calls_left: u32) -> (PageReader, Step) {
        let rungs: Vec<Planned> = policy
            .rungs()
            .iter()
            .enumerate()
            .filter(|(_, rung)| rung.enabled)
            .map(|(index, rung)| Planned {
                index,
                rung: rung.clone(),
            })
            .collect();
        let mut reader = PageReader {
            rungs,
            ceiling: policy.answer_ceiling(),
            call_limit: policy.max_calls_per_page().min(document_calls_left),
            position: 0,
            calls_made: 0,
            issued: 0,
            invalid_retried: false,
            cap_override: None,
            best: None,
            pending: None,
            finished: None,
        };
        let step = if reader.rungs.is_empty() {
            reader.review(ReviewReason::NoRungAvailable)
        } else if reader.calls_made >= reader.call_limit {
            reader.review(ReviewReason::CallCap)
        } else {
            reader.issue(CallKind::First)
        };
        (reader, step)
    }

    /// How many calls were really made on this page.
    pub fn calls_made(&self) -> u32 {
        self.calls_made
    }

    pub fn is_done(&self) -> bool {
        self.finished.is_some()
    }

    /// The call that has been handed out and not yet answered; `None` once the page is done.
    pub fn pending(&self) -> Option<&CallPlan> {
        self.pending.as_ref()
    }

    /// How the page ended, once it has.
    pub fn outcome(&self) -> Option<&PageOutcome> {
        self.finished.as_ref()
    }

    /// The attempt with the fewest blocking issues so far (ties to the later one).
    pub fn best_attempt(&self) -> Option<u32> {
        self.best.map(|(attempt, _)| attempt)
    }

    /// Takes what happened to the last call and says what to do next. Once the page is done,
    /// it keeps saying so.
    pub fn on_outcome(&mut self, outcome: Outcome) -> Step {
        if let Some(done) = &self.finished {
            return Step::Done(done.clone());
        }
        self.pending = None;
        if !matches!(outcome, Outcome::BudgetRefused | Outcome::CannotBeSentSafely) {
            self.calls_made += 1;
        }
        match outcome {
            Outcome::Read(assessment) => self.on_read(assessment),
            Outcome::NotConverted => self.escalate(EscalationReason::ReadingProblems),
            Outcome::Truncated => {
                if self.current_cap() < self.ceiling {
                    self.cap_override = Some(self.ceiling);
                    self.retry(CallKind::RetryTruncated)
                } else {
                    // At the ceiling already: a stronger model would be cut off the same way.
                    self.review(ReviewReason::TooLong)
                }
            }
            Outcome::InvalidOutput => {
                if self.invalid_retried {
                    self.escalate(EscalationReason::SchemaInvalid)
                } else {
                    self.invalid_retried = true;
                    self.retry(CallKind::RetryInvalid)
                }
            }
            Outcome::Refused => self.review(ReviewReason::Refused),
            Outcome::Rejected => self.review(ReviewReason::RequestRejected),
            Outcome::CannotBeSentSafely => self.review(ReviewReason::CannotBeSentSafely),
            Outcome::BudgetRefused => self.review(ReviewReason::Budget),
            Outcome::RungUnavailable => self.escalate(EscalationReason::RungUnavailable),
        }
    }

    fn on_read(&mut self, assessment: Assessment) -> Step {
        // The later attempt wins a tie: it came from a higher rung.
        if self
            .best
            .is_none_or(|(_, blocking)| assessment.blocking <= blocking)
        {
            self.best = Some((self.issued, assessment.blocking));
        }
        match assessment.verdict {
            Verdict::Accept => {
                let done = PageOutcome::Read {
                    rung: self.rungs[self.position].index,
                    attempt: self.issued,
                    confidence: assessment.confidence,
                    notes: assessment.notes,
                };
                self.finished = Some(done.clone());
                Step::Done(done)
            }
            Verdict::Escalate(reason) => self.escalate(reason),
            Verdict::Review(reason) => self.review(reason),
        }
    }

    fn current(&self) -> &Planned {
        &self.rungs[self.position]
    }

    fn current_cap(&self) -> u32 {
        self.cap_override.unwrap_or(self.current().rung.max_output_tokens)
    }

    /// Goes to the next rung, or ends the page if there is none or the caps are used.
    fn escalate(&mut self, reason: EscalationReason) -> Step {
        if self.position + 1 >= self.rungs.len() {
            let nothing_tried = reason == EscalationReason::RungUnavailable && self.best.is_none();
            return self.review(if nothing_tried {
                ReviewReason::NoRungAvailable
            } else {
                ReviewReason::TopRungFailed
            });
        }
        if self.calls_made >= self.call_limit {
            return self.review(ReviewReason::CallCap);
        }
        let from = self.current().index;
        self.position += 1;
        self.invalid_retried = false;
        self.cap_override = None;
        self.issue(CallKind::Escalation { from, reason })
    }

    /// The same rung again.
    fn retry(&mut self, kind: CallKind) -> Step {
        if self.calls_made >= self.call_limit {
            return self.review(ReviewReason::CallCap);
        }
        self.issue(kind)
    }

    fn issue(&mut self, kind: CallKind) -> Step {
        self.issued += 1;
        let planned = self.current();
        let plan = CallPlan {
            attempt: self.issued,
            rung: planned.index,
            model: planned.rung.model.clone(),
            effort: planned.rung.effort,
            max_output_tokens: self.current_cap(),
            picture: planned.rung.picture,
            kind,
        };
        self.pending = Some(plan.clone());
        Step::Call(plan)
    }

    fn review(&mut self, reason: ReviewReason) -> Step {
        let done = PageOutcome::NeedsReview {
            reason,
            draft: self.best.map(|(attempt, _)| attempt),
        };
        self.finished = Some(done.clone());
        Step::Done(done)
    }
}
