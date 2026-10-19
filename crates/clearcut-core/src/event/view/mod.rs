// Written by Kone Tshivhinda & Claude
//! Deterministic views over events: by group, date, document and severity.
//!
//! Every view is plain code over event *types*, never a model's judgement
//! (ADR-0004). Views borrow the events (`&Event`) instead of copying them, so
//! they are cheap and always show the same events the caller holds.
//!
//! All views first apply [`current`], so a corrected event never appears next
//! to the event that replaced it. To audit the full history, use the raw slice.
//!
//! A "by party" view is deferred: events carry no party field yet, so there is
//! nothing reliable to group on. It returns with event derivation (P3-03).

use super::{DocumentId, Event, EventGroup, EventId, Severity};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};

/// The events that have not been superseded by a later correction, in input
/// order. A chain of corrections (v1 <- v2 <- v3) leaves only v3. A
/// `supersedes` link to an event that is not in the slice is ignored.
pub fn current(events: &[Event]) -> Vec<&Event> {
    let superseded: HashSet<&EventId> = events.iter().filter_map(Event::supersedes).collect();
    events
        .iter()
        .filter(|event| !superseded.contains(event.id()))
        .collect()
}

/// Reading order: by date (undated facts last), then taxonomy group, then ID.
/// The ID is the final tie-break so the order is identical on every run.
fn reading_order(a: &Event, b: &Event) -> Ordering {
    let key = |e: &Event| (e.date().is_none(), e.date(), e.group(), e.id().clone());
    key(a).cmp(&key(b))
}

/// The chronological timeline of current events.
pub fn timeline(events: &[Event]) -> Vec<&Event> {
    let mut list = current(events);
    list.sort_by(|a, b| reading_order(a, b));
    list
}

/// Sorts events into buckets by `key`. Because the input is the sorted
/// timeline, every bucket is already in reading order. `BTreeMap` keeps its
/// keys sorted, which makes the output order deterministic. (`K: Ord` means
/// "any key type that can be sorted".)
///
/// The lifetime `'a` says "these references all borrow from the `events`
/// slice". That lets a key (like a document ID) be a reference into an event.
fn bucket<'a, K: Ord>(events: &'a [Event], key: impl Fn(&'a Event) -> K) -> BTreeMap<K, Vec<&'a Event>> {
    let mut buckets: BTreeMap<K, Vec<&'a Event>> = BTreeMap::new();
    for event in timeline(events) {
        buckets.entry(key(event)).or_default().push(event);
    }
    buckets
}

/// Current events by taxonomy group, in taxonomy order. Empty groups are left out.
pub fn by_group(events: &[Event]) -> Vec<(EventGroup, Vec<&Event>)> {
    bucket(events, Event::group).into_iter().collect()
}

/// Current events by source document, documents sorted by ID.
pub fn by_document(events: &[Event]) -> Vec<(&DocumentId, Vec<&Event>)> {
    bucket(events, |e| &e.citation().document).into_iter().collect()
}

/// Current events by severity, most severe first. Empty levels are left out.
pub fn by_severity(events: &[Event]) -> Vec<(Severity, Vec<&Event>)> {
    bucket(events, Event::severity).into_iter().rev().collect()
}

#[cfg(test)]
mod tests;
