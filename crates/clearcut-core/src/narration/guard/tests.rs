// Written by Kone Tshivhinda & Claude
use super::scan::scan;
use super::*;
use crate::event::{
    Citation, Confidence, DocumentId, DuplicateReason, Event, EventId, EventParts, EventPayload, Extractor,
    TotalField,
};
use crate::narration::{narrate_event, narrate_timeline, Narrative, Sentence};
use crate::{Money, TaxCategory};
use std::num::NonZeroU32;
use time::macros::date as d;
use time::Date;

fn zar(minor: i64) -> Money {
    Money::zar(minor)
}

fn mk(id: &str, when: Option<Date>, payload: EventPayload, supersedes: Option<&str>) -> Event {
    Event::new(EventParts {
        id: EventId::new(id),
        date: when,
        payload,
        citation: Citation {
            document: DocumentId::new("doc_1"),
            page: NonZeroU32::new(1).unwrap(),
            locator: None,
        },
        extractor: Extractor::System,
        confidence: Confidence::new(1000).unwrap(),
        supersedes: supersedes.map(EventId::new),
    })
    .unwrap()
}

fn line_item() -> EventPayload {
    EventPayload::LineItemBilled {
        description: "Consulting".into(),
        quantity_milli: 10_000,
        unit_price: zar(150_000),
        line_total: zar(1_500_000),
        tax_category: TaxCategory::Standard,
    }
}

fn due_date() -> EventPayload {
    EventPayload::DueDateStated {
        due: d!(2026 - 04 - 30),
        terms: Some("30 days".into()),
    }
}

fn vat_mismatch() -> EventPayload {
    EventPayload::TotalsMismatch {
        field: TotalField::Vat,
        stated: zar(225_000),
        recomputed: zar(225_001),
    }
}

fn supplier() -> EventPayload {
    EventPayload::SupplierIdentified {
        name: "Acme (Pty) Ltd".into(),
        vat_number: Some("4123456789".into()),
    }
}

/// One-sentence candidate narrative citing `ids`.
fn candidate(text: &str, ids: &[&str]) -> Narrative {
    let cites = ids.iter().map(|id| EventId::new(*id)).collect();
    Narrative::new(vec![Sentence::new(text, cites).unwrap()])
}

fn kinds(violations: &[Violation]) -> Vec<&ViolationKind> {
    violations.iter().map(|v| &v.kind).collect()
}

fn unverified_number(token: &str) -> ViolationKind {
    ViolationKind::UnverifiedNumber(token.into())
}

fn unverified_date(raw: &str) -> ViolationKind {
    ViolationKind::UnverifiedDate(raw.into())
}

// ---------- scanner: numerals ----------

#[test]
fn scans_amounts_as_plain_numbers() {
    assert_eq!(scan("R15\u{a0}000.00").numerals, ["15000.00"]);
    assert_eq!(scan("R1\u{a0}234\u{a0}567.89").numerals, ["1234567.89"]);
    assert_eq!(scan("R0.05").numerals, ["0.05"]);
}

#[test]
fn scans_every_number_in_a_sentence() {
    let found = scan("Billed 10 × Consulting at R1\u{a0}500.00 each, R15\u{a0}000.00 in total.");
    assert_eq!(found.numerals, ["10", "1500.00", "15000.00"]);
}

#[test]
fn a_full_stop_after_a_number_is_not_a_decimal_point() {
    assert_eq!(scan("recomputed R2\u{a0}250.01.").numerals, ["2250.01"]);
    assert_eq!(scan("Total is 15.").numerals, ["15"]);
}

#[test]
fn percentages_scan_as_the_number() {
    assert_eq!(scan("at 7.5% is").numerals, ["7.5"]);
}

#[test]
fn an_ordinary_space_does_not_join_digit_groups() {
    // Only the no-break space the narrator writes joins groups, so a sloppy
    // "R15 000.00" splits into "15" and "000.00" and will not verify.
    assert_eq!(scan("R15 000.00").numerals, ["15", "000.00"]);
}

#[test]
fn a_minus_sign_belongs_to_the_number() {
    assert_eq!(scan("-R5.00").numerals, ["-5.00"]);
    assert_eq!(scan("-EUR 2\u{a0}500.00").numerals, ["-2500.00"]);
    assert_eq!(scan("\u{2212}R5.00").numerals, ["-5.00"]);
    assert_eq!(scan("R5.00 - R3.00").numerals, ["5.00", "3.00"]);
}

#[test]
fn digits_inside_identifiers_are_numerals() {
    assert_eq!(scan("invoice INV-0042 for doc_9").numerals, ["-0042", "9"]);
    assert_eq!(scan("VAT number 4123456789").numerals, ["4123456789"]);
}

// ---------- scanner: dates ----------

#[test]
fn scans_a_written_date_as_one_unit() {
    let found = scan("Payment is due on 30 April 2026.");
    assert!(found.numerals.is_empty());
    assert_eq!(found.dates.len(), 1);
    assert_eq!(found.dates[0].raw, "30 April 2026");
    assert_eq!(found.dates[0].date, Some(d!(2026 - 04 - 30)));
}

#[test]
fn scans_an_iso_date_as_one_unit() {
    let found = scan("dated 2026-04-30 here");
    assert!(found.numerals.is_empty());
    assert_eq!(found.dates[0].date, Some(d!(2026 - 04 - 30)));
}

#[test]
fn an_impossible_date_is_kept_but_has_no_value() {
    let found = scan("due 31 April 2026 and 2026-02-30");
    assert_eq!(found.dates.len(), 2);
    assert_eq!(found.dates[0].raw, "31 April 2026");
    assert_eq!(found.dates[0].date, None);
    assert_eq!(found.dates[1].raw, "2026-02-30");
    assert_eq!(found.dates[1].date, None);
}

#[test]
fn an_abbreviated_month_is_not_a_date() {
    // Not recognised as a date, so its numbers are checked on their own.
    let found = scan("due 31 Mar 2026");
    assert!(found.dates.is_empty());
    assert_eq!(found.numerals, ["31", "2026"]);
}

// ---------- the check ----------

#[test]
fn the_template_narrative_always_passes() {
    let events = vec![
        mk("e1", Some(d!(2026 - 03 - 31)), supplier(), None),
        mk("e2", Some(d!(2026 - 03 - 31)), line_item(), None),
        mk(
            "e3",
            Some(d!(2026 - 03 - 31)),
            EventPayload::VatCalculated {
                category: TaxCategory::Standard,
                taxable: zar(1_500_000),
                rate_bps: 1_500,
                vat: zar(225_000),
            },
            None,
        ),
        mk(
            "e4",
            None,
            EventPayload::TotalsStated {
                net: zar(1_500_000),
                vat: zar(225_000),
                gross: zar(1_725_000),
            },
            None,
        ),
        mk("e5", None, vat_mismatch(), None),
        mk("e6", None, due_date(), None),
        mk(
            "e7",
            None,
            EventPayload::CreditNoteReferenced {
                original_invoice_number: "INV-0042".into(),
                amount: zar(-50_000),
            },
            None,
        ),
        mk(
            "e8",
            None,
            EventPayload::ComplianceIssueFound {
                field: "vat_number".into(),
                message: "supplier VAT number is missing".into(),
            },
            None,
        ),
        mk(
            "e9",
            None,
            EventPayload::PossibleDuplicate {
                matching_document: DocumentId::new("doc_9"),
                reason: DuplicateReason::ContentHash,
            },
            None,
        ),
    ];
    assert_eq!(
        check(&narrate_timeline(&events), &events),
        Vec::<Violation>::new()
    );
}

#[test]
fn the_narrated_carried_figures_pass_the_number_guard_and_an_invented_one_is_caught() {
    use crate::event::BalanceRole;
    let events = vec![
        mk(
            "e1",
            None,
            EventPayload::BalanceStated {
                role: BalanceRole::CarriedForward,
                amount: zar(100_000),
            },
            None,
        ),
        mk(
            "e2",
            None,
            EventPayload::BalanceStated {
                role: BalanceRole::BroughtForward,
                amount: zar(100_000),
            },
            None,
        ),
    ];
    assert_eq!(
        check(&narrate_timeline(&events), &events),
        Vec::<Violation>::new()
    );
    let invented = candidate("The amount carried forward is R1\u{a0}001.00.", &["e1"]);
    assert_eq!(kinds(&check(&invented, &events)), [&unverified_number("1001.00")]);
}

#[test]
fn smoothed_prose_that_keeps_the_facts_passes() {
    let events = vec![mk("e1", None, line_item(), None)];
    let story = candidate(
        "Over this period, 10 × Consulting was billed, coming to R15\u{a0}000.00.",
        &["e1"],
    );
    assert_eq!(check(&story, &events), Vec::<Violation>::new());
}

#[test]
fn an_altered_amount_is_caught() {
    let events = vec![mk("e1", None, line_item(), None)];
    let story = candidate("Billed 10 × Consulting at R1\u{a0}600.00 each.", &["e1"]);
    let found = check(&story, &events);
    assert_eq!(kinds(&found), [&unverified_number("1600.00")]);
    assert_eq!(found[0].sentence, 0);
}

#[test]
fn an_altered_quantity_is_caught() {
    let events = vec![mk("e1", None, line_item(), None)];
    let story = candidate("Billed 12 × Consulting.", &["e1"]);
    assert_eq!(kinds(&check(&story, &events)), [&unverified_number("12")]);
}

#[test]
fn an_altered_rate_is_caught() {
    let vat = EventPayload::VatCalculated {
        category: TaxCategory::Standard,
        taxable: zar(1_500_000),
        rate_bps: 1_500,
        vat: zar(225_000),
    };
    let events = vec![mk("e1", None, vat, None)];
    let story = candidate("VAT at 16% was charged.", &["e1"]);
    assert_eq!(kinds(&check(&story, &events)), [&unverified_number("16")]);
}

#[test]
fn an_invented_difference_is_caught() {
    let events = vec![mk("e1", None, vat_mismatch(), None)];
    let story = candidate("The stated VAT of R2\u{a0}250.00 is out by R0.01.", &["e1"]);
    assert_eq!(kinds(&check(&story, &events)), [&unverified_number("0.01")]);
}

#[test]
fn a_dropped_minus_sign_is_caught() {
    let credit = EventPayload::CreditNoteReferenced {
        original_invoice_number: "INV-0042".into(),
        amount: zar(-50_000),
    };
    let events = vec![mk("e1", None, credit, None)];
    let story = candidate("A credit note of R500.00 refers to invoice INV-0042.", &["e1"]);
    assert_eq!(kinds(&check(&story, &events)), [&unverified_number("500.00")]);
}

#[test]
fn a_changed_identifier_is_caught() {
    let events = vec![mk("e1", None, supplier(), None)];
    let story = candidate("The supplier is Acme (VAT number 4123456780).", &["e1"]);
    assert_eq!(kinds(&check(&story, &events)), [&unverified_number("4123456780")]);
}

#[test]
fn separators_may_vary_when_the_value_is_the_same() {
    let events = vec![mk("e1", None, line_item(), None)];
    let story = candidate("Billed R15000.00 in total.", &["e1"]);
    assert_eq!(check(&story, &events), Vec::<Violation>::new());
}

#[test]
fn a_figure_with_missing_decimals_is_rejected_as_unverifiable() {
    // Strict on purpose: a false rejection only means falling back to the template.
    let events = vec![mk("e1", None, line_item(), None)];
    let story = candidate("Billed R1500 each.", &["e1"]);
    assert_eq!(kinds(&check(&story, &events)), [&unverified_number("1500")]);
}

#[test]
fn a_loosely_spaced_amount_is_rejected_as_unverifiable() {
    let events = vec![mk("e1", None, line_item(), None)];
    let story = candidate("Billed R15 000.00 in total.", &["e1"]);
    let found = check(&story, &events);
    assert!(kinds(&found).contains(&&unverified_number("000.00")));
}

#[test]
fn a_figure_from_an_event_the_sentence_does_not_cite_is_caught() {
    let events = vec![
        mk("e1", None, supplier(), None),
        mk("e2", None, line_item(), None),
    ];
    let story = candidate("The supplier billed R15\u{a0}000.00.", &["e1"]);
    assert_eq!(kinds(&check(&story, &events)), [&unverified_number("15000.00")]);
}

#[test]
fn a_sentence_may_cite_several_events() {
    let events = vec![
        mk("e1", None, supplier(), None),
        mk("e2", None, line_item(), None),
    ];
    let story = candidate("Acme billed R15\u{a0}000.00.", &["e1", "e2"]);
    assert_eq!(check(&story, &events), Vec::<Violation>::new());
}

// ---------- the check: dates ----------

#[test]
fn a_date_from_the_cited_event_passes() {
    let events = vec![mk("e1", None, due_date(), None)];
    let story = candidate("The invoice falls due on 30 April 2026.", &["e1"]);
    assert_eq!(check(&story, &events), Vec::<Violation>::new());
}

#[test]
fn the_events_own_date_may_be_used() {
    let events = vec![mk("e1", Some(d!(2026 - 03 - 31)), supplier(), None)];
    let story = candidate("On 31 March 2026 the supplier was identified.", &["e1"]);
    assert_eq!(check(&story, &events), Vec::<Violation>::new());
}

#[test]
fn a_changed_month_is_caught_even_though_the_numbers_match() {
    let events = vec![mk("e1", None, due_date(), None)];
    let story = candidate("The invoice falls due on 30 May 2026.", &["e1"]);
    assert_eq!(kinds(&check(&story, &events)), [&unverified_date("30 May 2026")]);
}

#[test]
fn an_impossible_date_is_caught() {
    let events = vec![mk("e1", None, due_date(), None)];
    let story = candidate("Due on 31 April 2026.", &["e1"]);
    assert_eq!(
        kinds(&check(&story, &events)),
        [&unverified_date("31 April 2026")]
    );
}

#[test]
fn an_iso_date_is_accepted_when_it_is_the_same_day() {
    let events = vec![mk("e1", None, due_date(), None)];
    assert_eq!(
        check(&candidate("Due 2026-04-30.", &["e1"]), &events),
        Vec::<Violation>::new()
    );
    let wrong = check(&candidate("Due 2026-04-29.", &["e1"]), &events);
    assert_eq!(kinds(&wrong), [&unverified_date("2026-04-29")]);
}

// ---------- the check: citations ----------

#[test]
fn citing_an_unknown_event_is_caught() {
    let events = vec![mk("e1", None, supplier(), None)];
    let story = candidate("The supplier is Acme.", &["nope"]);
    let found = check(&story, &events);
    assert!(kinds(&found).contains(&&ViolationKind::UnknownEvent(EventId::new("nope"))));
}

#[test]
fn citing_a_superseded_event_is_caught() {
    let events = vec![
        mk("old", None, supplier(), None),
        mk("new", None, supplier(), Some("old")),
    ];
    let story = candidate("The supplier is Acme (Pty) Ltd.", &["old"]);
    let found = check(&story, &events);
    assert!(kinds(&found).contains(&&ViolationKind::SupersededEvent(EventId::new("old"))));
}

#[test]
fn violations_say_which_sentence_they_are_in() {
    let events = vec![mk("e1", None, line_item(), None)];
    let story = Narrative::new(vec![
        Sentence::new("Billed 10 × Consulting.", vec![EventId::new("e1")]).unwrap(),
        Sentence::new("Billed 11 × Consulting.", vec![EventId::new("e1")]).unwrap(),
    ]);
    let found = check(&story, &events);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].sentence, 1);
}

// ---------- the guard ----------

#[test]
fn a_clean_candidate_is_accepted_unchanged() {
    let events = vec![mk("e1", None, line_item(), None)];
    let story = candidate("Consulting: 10 units, R15\u{a0}000.00 in total.", &["e1"]);
    let guarded = guard(story.clone(), &events);
    assert!(guarded.is_accepted());
    assert_eq!(guarded.narrative, story);
    assert!(guarded.violations.is_empty());
}

#[test]
fn a_bad_candidate_falls_back_to_template_text() {
    let events = vec![
        mk("e1", Some(d!(2026 - 03 - 01)), supplier(), None),
        mk("e2", Some(d!(2026 - 03 - 02)), line_item(), None),
    ];
    let story = Narrative::new(vec![
        Sentence::new(
            "Acme billed R99\u{a0}999.00.",
            vec![EventId::new("e2"), EventId::new("e1")],
        )
        .unwrap(),
        Sentence::new("Acme is the supplier.", vec![EventId::new("e1")]).unwrap(),
    ]);
    let guarded = guard(story, &events);
    assert!(!guarded.is_accepted());
    assert_eq!(kinds(&guarded.violations), [&unverified_number("99999.00")]);
    // Template sentences for each cited event, in the order first cited, once each.
    let expected = Narrative::new(vec![narrate_event(&events[1]), narrate_event(&events[0])]);
    assert_eq!(guarded.narrative, expected);
}

#[test]
fn the_fallback_skips_events_that_cannot_be_narrated() {
    let events = vec![
        mk("old", None, supplier(), None),
        mk("new", None, supplier(), Some("old")),
    ];
    let story = candidate("Acme.", &["old", "ghost", "new"]);
    let guarded = guard(story, &events);
    assert!(!guarded.is_accepted());
    assert_eq!(guarded.narrative, Narrative::new(vec![narrate_event(&events[1])]));
}

#[test]
fn an_empty_narrative_is_accepted() {
    let guarded = guard(Narrative::new(Vec::new()), &[]);
    assert!(guarded.is_accepted());
    assert!(guarded.narrative.sentences().is_empty());
}
