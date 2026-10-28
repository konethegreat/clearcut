// Written by Kone Tshivhinda & Claude
use super::format::{amount, date, quantity_milli, rate_bps};
use super::*;
use crate::event::{
    Citation, Confidence, DocumentId, DuplicateReason, Event, EventId, EventParts, EventPayload, Extractor,
    TotalField,
};
use crate::{Currency, Error, Money, TaxCategory};
use std::num::NonZeroU32;
use time::macros::date as d;
use time::Date;

fn zar(minor: i64) -> Money {
    Money::zar(minor)
}

fn mk(id: &str, when: Option<Date>, payload: EventPayload) -> Event {
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
        supersedes: None,
    })
    .unwrap()
}

fn say(payload: EventPayload) -> String {
    narrate_event(&mk("evt_1", None, payload)).text().to_string()
}

// ---------- formatting ----------

#[test]
fn amounts_use_rand_with_no_break_space_between_thousands() {
    assert_eq!(amount(zar(0)), "R0.00");
    assert_eq!(amount(zar(5)), "R0.05");
    assert_eq!(amount(zar(1_050)), "R10.50");
    assert_eq!(amount(zar(99_999)), "R999.99");
    assert_eq!(amount(zar(100_000)), "R1\u{a0}000.00");
    assert_eq!(amount(zar(1_500_000)), "R15\u{a0}000.00");
    assert_eq!(amount(zar(123_456_789)), "R1\u{a0}234\u{a0}567.89");
}

#[test]
fn negative_amounts_get_a_leading_minus() {
    assert_eq!(amount(zar(-500)), "-R5.00");
    assert_eq!(amount(zar(-123_456)), "-R1\u{a0}234.56");
}

#[test]
fn other_currencies_use_their_code() {
    assert_eq!(amount(Money::new(1_000, Currency::Usd)), "USD 10.00");
    assert_eq!(amount(Money::new(-250_000, Currency::Eur)), "-EUR 2\u{a0}500.00");
}

#[test]
fn the_smallest_amount_does_not_overflow() {
    // i64::MIN has no positive counterpart, so formatting must not negate it.
    assert!(amount(zar(i64::MIN)).starts_with("-R"));
}

#[test]
fn quantities_drop_trailing_zeros() {
    assert_eq!(quantity_milli(10_000), "10");
    assert_eq!(quantity_milli(10_500), "10.5");
    assert_eq!(quantity_milli(125), "0.125");
    assert_eq!(quantity_milli(1), "0.001");
    assert_eq!(quantity_milli(-2_000), "-2");
}

#[test]
fn rates_are_shown_as_percentages_without_trailing_zeros() {
    assert_eq!(rate_bps(1_500), "15%");
    assert_eq!(rate_bps(750), "7.5%");
    assert_eq!(rate_bps(725), "7.25%");
    assert_eq!(rate_bps(0), "0%");
}

#[test]
fn dates_are_written_out_in_full() {
    assert_eq!(date(d!(2026 - 03 - 31)), "31 March 2026");
    assert_eq!(date(d!(2026 - 01 - 05)), "5 January 2026");
}

// ---------- one sentence per event kind ----------

#[test]
fn narrates_a_supplier() {
    let with_vat = EventPayload::SupplierIdentified {
        name: "Acme (Pty) Ltd".into(),
        vat_number: Some("4123456789".into()),
    };
    assert_eq!(
        say(with_vat),
        "The supplier is Acme (Pty) Ltd (VAT number 4123456789)."
    );
    let without = EventPayload::SupplierIdentified {
        name: "Acme (Pty) Ltd".into(),
        vat_number: None,
    };
    assert_eq!(say(without), "The supplier is Acme (Pty) Ltd.");
}

#[test]
fn narrates_a_recipient() {
    let with_vat = EventPayload::RecipientIdentified {
        name: "Client Co".into(),
        vat_number: Some("4987654321".into()),
    };
    assert_eq!(
        say(with_vat),
        "The recipient is Client Co (VAT number 4987654321)."
    );
    let without = EventPayload::RecipientIdentified {
        name: "Client Co".into(),
        vat_number: None,
    };
    assert_eq!(say(without), "The recipient is Client Co.");
}

#[test]
fn narrates_an_invoice_number() {
    let number = EventPayload::InvoiceNumberStated {
        number: "INV-000001".into(),
    };
    assert_eq!(say(number), "The invoice number is INV-000001.");
}

#[test]
fn narrates_a_line_item() {
    let line = EventPayload::LineItemBilled {
        description: "Consulting".into(),
        quantity_milli: 10_000,
        unit_price: zar(150_000),
        line_total: zar(1_500_000),
        tax_category: TaxCategory::Standard,
    };
    assert_eq!(
        say(line),
        "Billed 10 × Consulting at R1\u{a0}500.00 each, R15\u{a0}000.00 in total (standard-rated)."
    );
}

#[test]
fn names_every_tax_category() {
    let line = |category| EventPayload::LineItemBilled {
        description: "Item".into(),
        quantity_milli: 1_000,
        unit_price: zar(100),
        line_total: zar(100),
        tax_category: category,
    };
    assert!(say(line(TaxCategory::ZeroRated)).ends_with("(zero-rated)."));
    assert!(say(line(TaxCategory::Exempt)).ends_with("(VAT-exempt)."));
    assert!(say(line(TaxCategory::OutOfScope)).ends_with("(outside the scope of VAT)."));
}

#[test]
fn narrates_vat() {
    let vat = EventPayload::VatCalculated {
        category: TaxCategory::Standard,
        taxable: zar(1_500_000),
        rate_bps: 1_500,
        vat: zar(225_000),
    };
    assert_eq!(
        say(vat),
        "VAT on standard-rated supplies of R15\u{a0}000.00 at 15% is R2\u{a0}250.00."
    );
}

#[test]
fn narrates_stated_totals() {
    let totals = EventPayload::TotalsStated {
        net: zar(1_500_000),
        vat: zar(225_000),
        gross: zar(1_725_000),
    };
    assert_eq!(
        say(totals),
        "The document states a net total of R15\u{a0}000.00, VAT of R2\u{a0}250.00 and a gross total of R17\u{a0}250.00."
    );
}

#[test]
fn narrates_a_mismatch_without_inventing_a_difference() {
    let mismatch = |field| EventPayload::TotalsMismatch {
        field,
        stated: zar(225_000),
        recomputed: zar(225_001),
    };
    let text = say(mismatch(TotalField::Vat));
    assert_eq!(
        text,
        "The stated VAT of R2\u{a0}250.00 does not match the recomputed R2\u{a0}250.01."
    );
    // The difference (R0.01) is a derived number that is in no event, so it
    // must never appear: the number guard would reject it.
    assert!(!text.contains("R0.01"));
    assert!(say(mismatch(TotalField::Net)).starts_with("The stated net total of"));
    assert!(say(mismatch(TotalField::Gross)).starts_with("The stated gross total of"));
}

#[test]
fn narrates_a_line_total_mismatch_without_inventing_a_difference() {
    let text = say(EventPayload::LineTotalMismatch {
        line: 2,
        stated: zar(110_000),
        recomputed: zar(100_000),
    });
    assert_eq!(
        text,
        "On line 2 the stated total of R1\u{a0}100.00 does not match the recomputed R1\u{a0}000.00."
    );
    assert!(!text.contains("R100.00")); // the R100.00 difference is in no event
}

#[test]
fn narrates_a_due_date() {
    let with_terms = EventPayload::DueDateStated {
        due: d!(2026 - 04 - 30),
        terms: Some("30 days".into()),
    };
    assert_eq!(
        say(with_terms),
        "Payment is due on 30 April 2026 (terms: 30 days)."
    );
    let bare = EventPayload::DueDateStated {
        due: d!(2026 - 04 - 30),
        terms: None,
    };
    assert_eq!(say(bare), "Payment is due on 30 April 2026.");
}

#[test]
fn narrates_a_credit_note() {
    let credit = EventPayload::CreditNoteReferenced {
        original_invoice_number: "INV-0042".into(),
        amount: zar(50_000),
    };
    assert_eq!(
        say(credit),
        "A credit note of R500.00 refers to invoice INV-0042."
    );
}

#[test]
fn narrates_a_compliance_issue_with_one_full_stop() {
    let issue = |message: &str| EventPayload::ComplianceIssueFound {
        field: "vat_number".into(),
        message: message.into(),
    };
    let expected = "Compliance issue with vat_number: supplier VAT number is missing.";
    assert_eq!(say(issue("supplier VAT number is missing")), expected);
    assert_eq!(say(issue("supplier VAT number is missing.")), expected);
}

#[test]
fn narrates_a_possible_duplicate() {
    let dup = |reason| EventPayload::PossibleDuplicate {
        matching_document: DocumentId::new("doc_9"),
        reason,
    };
    assert_eq!(
        say(dup(DuplicateReason::ContentHash)),
        "This document may duplicate doc_9: identical content."
    );
    assert_eq!(
        say(dup(DuplicateReason::SupplierNumberDateTotal)),
        "This document may duplicate doc_9: same supplier, invoice number, date and total."
    );
}

// ---------- citations ----------

#[test]
fn a_sentence_cites_the_event_it_came_from() {
    let event = mk(
        "evt_77",
        None,
        EventPayload::SupplierIdentified {
            name: "Acme".into(),
            vat_number: None,
        },
    );
    assert_eq!(narrate_event(&event).cites(), [EventId::new("evt_77")]);
}

#[test]
fn a_sentence_without_a_citation_cannot_exist() {
    assert_eq!(
        Sentence::new("Uncited claim.", Vec::new()),
        Err(Error::InvalidNarration("a sentence must cite at least one event"))
    );
    let ok = Sentence::new("Cited.", vec![EventId::new("evt_1")]).unwrap();
    assert_eq!(ok.text(), "Cited.");
}

// ---------- the narrative ----------

fn supplier(name: &str) -> EventPayload {
    EventPayload::SupplierIdentified {
        name: name.into(),
        vat_number: None,
    }
}

#[test]
fn narrates_current_events_in_timeline_order() {
    let events = vec![
        mk("late", Some(d!(2026 - 04 - 01)), supplier("Late Ltd")),
        mk("early", Some(d!(2026 - 03 - 01)), supplier("Early Ltd")),
        mk("undated", None, supplier("Undated Ltd")),
    ];
    let story = narrate_timeline(&events);
    let texts: Vec<&str> = story.sentences().iter().map(Sentence::text).collect();
    assert_eq!(
        texts,
        [
            "The supplier is Early Ltd.",
            "The supplier is Late Ltd.",
            "The supplier is Undated Ltd."
        ]
    );
}

#[test]
fn a_corrected_event_is_not_narrated() {
    let old = mk("old", None, supplier("Wrong Ltd"));
    let new = Event::new(EventParts {
        id: EventId::new("new"),
        date: None,
        payload: supplier("Right Ltd"),
        citation: old.citation().clone(),
        extractor: Extractor::Human { user: "kone".into() },
        confidence: Confidence::new(1000).unwrap(),
        supersedes: Some(EventId::new("old")),
    })
    .unwrap();
    let story = narrate_timeline(&[old, new]);
    assert_eq!(story.sentences().len(), 1);
    assert_eq!(story.sentences()[0].text(), "The supplier is Right Ltd.");
    assert_eq!(story.sentences()[0].cites(), [EventId::new("new")]);
}

#[test]
fn narration_is_deterministic() {
    let events = vec![
        mk("a", Some(d!(2026 - 03 - 01)), supplier("A")),
        mk("b", Some(d!(2026 - 03 - 01)), supplier("B")),
    ];
    assert_eq!(narrate_timeline(&events), narrate_timeline(&events));
}

#[test]
fn nothing_to_narrate_gives_an_empty_narrative() {
    assert!(narrate_timeline(&[]).sentences().is_empty());
}

#[test]
fn a_narrative_serialises_with_text_and_citations() {
    let events = vec![mk("evt_1", None, supplier("Acme"))];
    let value = serde_json::to_value(narrate_timeline(&events)).unwrap();
    assert_eq!(
        value,
        serde_json::json!({
            "sentences": [{ "text": "The supplier is Acme.", "cites": ["evt_1"] }]
        })
    );
}
