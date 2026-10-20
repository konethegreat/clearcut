// Written by Kone Tshivhinda & Claude
//! The number guard (research 05 §5, rule 4): after a narrative is written,
//! confirm that every figure and date in it comes from the events it cites.
//!
//! A narrative is a financial statement, and a language model can invent
//! numbers. This check is deterministic and does not trust the model:
//!
//! - For each sentence, the *allowed* facts are the figures and dates in the
//!   template wording of the events it cites, plus those events' own dates.
//!   (Using the template text means the guard and the narrator can never
//!   disagree about how a figure is written.)
//! - Every figure and date in the sentence must be in that allowed set.
//! - Every cited event must exist and must not have been superseded.
//!
//! Anything else is a [`Violation`]. [`guard`] then throws the whole candidate
//! away and returns plain template sentences for the events it cited.
//!
//! Limits, stated so nobody over-trusts it: it checks digits and dates, not
//! meaning. Numbers written as words ("ten") are not seen, and a sentence can
//! still be misleading while every figure in it is real. The wording of the
//! facts themselves comes from templates for that reason.

mod scan;

use super::{narrate_event, Narrative};
use crate::event::view;
use crate::event::{Event, EventId};
use scan::scan;
use std::collections::{HashMap, HashSet};
use time::Date;

/// What was wrong with a sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViolationKind {
    /// The sentence cites an event that is not in the event list.
    UnknownEvent(EventId),
    /// The sentence cites an event that a later correction replaced.
    SupersededEvent(EventId),
    /// A figure that is in none of the cited events (normalised, sign kept).
    UnverifiedNumber(String),
    /// A date, as written, that is in none of the cited events or is impossible.
    UnverifiedDate(String),
}

/// One problem, and the (0-based) sentence it was found in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub sentence: usize,
    pub kind: ViolationKind,
}

/// The figures and dates a sentence is allowed to state.
#[derive(Default)]
struct Allowed {
    numerals: HashSet<String>,
    dates: HashSet<Date>,
}

impl Allowed {
    fn add(&mut self, event: &Event) {
        let found = scan(narrate_event(event).text());
        self.numerals.extend(found.numerals);
        self.dates.extend(found.dates.into_iter().filter_map(|d| d.date));
        if let Some(date) = event.date() {
            self.dates.insert(date);
        }
    }
}

/// Checks every sentence of `narrative` against `events`. An empty result
/// means the narrative passed.
pub fn check(narrative: &Narrative, events: &[Event]) -> Vec<Violation> {
    let current = current_by_id(events);
    let known: HashSet<&EventId> = events.iter().map(Event::id).collect();
    let mut violations = Vec::new();

    for (sentence, text) in narrative.sentences().iter().enumerate() {
        let mut problem = |kind| violations.push(Violation { sentence, kind });
        let mut allowed = Allowed::default();

        for id in text.cites() {
            match current.get(id) {
                Some(event) => allowed.add(event),
                None if known.contains(id) => problem(ViolationKind::SupersededEvent(id.clone())),
                None => problem(ViolationKind::UnknownEvent(id.clone())),
            }
        }

        let found = scan(text.text());
        for numeral in found.numerals {
            if !allowed.numerals.contains(&numeral) {
                problem(ViolationKind::UnverifiedNumber(numeral));
            }
        }
        for date in found.dates {
            if !date.date.is_some_and(|d| allowed.dates.contains(&d)) {
                problem(ViolationKind::UnverifiedDate(date.raw));
            }
        }
    }
    violations
}

/// A narrative after the guard has looked at it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guarded {
    /// The candidate if it passed, otherwise the template fallback.
    pub narrative: Narrative,
    /// Why the candidate was rejected. Empty when it passed.
    pub violations: Vec<Violation>,
}

impl Guarded {
    pub fn is_accepted(&self) -> bool {
        self.violations.is_empty()
    }
}

/// Returns `candidate` if it passes [`check`]. If anything fails, the whole
/// candidate is rejected and replaced by the template sentence of each event
/// it cited (once each, in the order first cited). Events that cannot be
/// narrated, because they are unknown or superseded, are left out.
pub fn guard(candidate: Narrative, events: &[Event]) -> Guarded {
    let violations = check(&candidate, events);
    if violations.is_empty() {
        return Guarded {
            narrative: candidate,
            violations,
        };
    }

    let current = current_by_id(events);
    let mut seen = HashSet::new();
    let sentences = candidate
        .sentences()
        .iter()
        .flat_map(|sentence| sentence.cites())
        .filter(|id| seen.insert(*id))
        .filter_map(|id| current.get(id))
        .map(|event| narrate_event(event))
        .collect();
    Guarded {
        narrative: Narrative::new(sentences),
        violations,
    }
}

/// The events that have not been superseded, looked up by ID.
fn current_by_id(events: &[Event]) -> HashMap<&EventId, &Event> {
    view::current(events)
        .into_iter()
        .map(|event| (event.id(), event))
        .collect()
}

#[cfg(test)]
mod tests;
