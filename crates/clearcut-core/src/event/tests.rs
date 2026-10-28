// Written by Kone Tshivhinda & Claude
use super::*;
use crate::{Currency, Error, Money, TaxCategory};
use serde_json::json;
use std::num::NonZeroU32;
use time::macros::date;

fn zar(minor: i64) -> Money {
    Money::zar(minor)
}

fn citation() -> Citation {
    Citation {
        document: DocumentId::new("doc_1"),
        page: NonZeroU32::new(2).unwrap(),
        locator: None,
    }
}

fn mismatch_payload() -> EventPayload {
    EventPayload::TotalsMismatch {
        field: TotalField::Vat,
        stated: zar(150_000),
        recomputed: zar(150_001),
    }
}

fn parts(payload: EventPayload) -> EventParts {
    EventParts {
        id: EventId::new("evt_1"),
        date: Some(date!(2026 - 03 - 31)),
        payload,
        citation: citation(),
        extractor: Extractor::System,
        confidence: Confidence::new(900).unwrap(),
        supersedes: None,
    }
}

fn invalid(reason: &'static str) -> Error {
    Error::InvalidEvent(reason)
}

// ---------- taxonomy ----------

#[test]
fn every_event_type_maps_to_its_group() {
    use EventGroup::*;
    use EventType::*;
    let expected = [
        (InvoiceNumberStated, Document),
        (SupplierIdentified, Parties),
        (RecipientIdentified, Parties),
        (LineItemBilled, Charges),
        (LineTotalMismatch, Charges),
        (VatCalculated, Tax),
        (TotalsStated, Totals),
        (TotalsMismatch, Totals),
        (DueDateStated, Terms),
        (CreditNoteReferenced, Adjustments),
        (ComplianceIssueFound, Compliance),
        (PossibleDuplicate, Risk),
    ];
    for (event_type, group) in expected {
        assert_eq!(event_type.group(), group, "{event_type:?}");
    }
    assert_eq!(EventType::ALL.len(), expected.len());
}

#[test]
fn severity_is_derived_from_the_event_type() {
    use EventType::*;
    use Severity::*;
    let expected = [
        (InvoiceNumberStated, Info),
        (SupplierIdentified, Info),
        (RecipientIdentified, Info),
        (LineItemBilled, Info),
        (LineTotalMismatch, Critical),
        (VatCalculated, Info),
        (TotalsStated, Info),
        (TotalsMismatch, Critical),
        (DueDateStated, Info),
        (CreditNoteReferenced, Info),
        (ComplianceIssueFound, Warning),
        (PossibleDuplicate, Warning),
    ];
    for (event_type, severity) in expected {
        assert_eq!(event_type.severity(), severity, "{event_type:?}");
    }
    assert_eq!(EventType::ALL.len(), expected.len());
    assert!(Info < Warning && Warning < Critical);
}

#[test]
fn an_event_reports_its_severity() {
    let event = Event::new(parts(mismatch_payload())).unwrap();
    assert_eq!(event.severity(), Severity::Critical);
}

#[test]
fn groups_follow_the_fixed_taxonomy_order() {
    use EventGroup::*;
    assert_eq!(
        EventGroup::ALL,
        [
            Document,
            Parties,
            Charges,
            Tax,
            Totals,
            Terms,
            Adjustments,
            Payments,
            Compliance,
            Risk
        ]
    );
}

#[test]
fn payload_reports_its_event_type() {
    assert_eq!(mismatch_payload().event_type(), EventType::TotalsMismatch);
    let line = EventPayload::LineItemBilled {
        description: "Consulting".into(),
        quantity_milli: 10_000,
        unit_price: zar(150_000),
        line_total: zar(1_500_000),
        tax_category: TaxCategory::Standard,
    };
    assert_eq!(line.event_type(), EventType::LineItemBilled);
}

// ---------- construction and invariants ----------

#[test]
fn builds_a_valid_event_and_exposes_its_parts() {
    let event = Event::new(parts(mismatch_payload())).unwrap();
    assert_eq!(event.id().as_str(), "evt_1");
    assert_eq!(event.date(), Some(date!(2026 - 03 - 31)));
    assert_eq!(event.event_type(), EventType::TotalsMismatch);
    assert_eq!(event.group(), EventGroup::Totals);
    assert_eq!(event.confidence().get(), 900);
    assert_eq!(event.citation().page.get(), 2);
    assert_eq!(event.supersedes(), None);
}

#[test]
fn an_event_may_have_no_date() {
    let mut p = parts(mismatch_payload());
    p.date = None;
    assert_eq!(Event::new(p).unwrap().date(), None);
}

#[test]
fn rejects_empty_event_id() {
    let mut p = parts(mismatch_payload());
    p.id = EventId::new("");
    assert_eq!(Event::new(p), Err(invalid("event id must not be empty")));
}

#[test]
fn rejects_empty_document_id() {
    let mut p = parts(mismatch_payload());
    p.citation.document = DocumentId::new("");
    assert_eq!(Event::new(p), Err(invalid("document id must not be empty")));
}

#[test]
fn an_event_cannot_supersede_itself() {
    let mut p = parts(mismatch_payload());
    p.supersedes = Some(EventId::new("evt_1"));
    assert_eq!(Event::new(p), Err(invalid("an event cannot supersede itself")));
}

#[test]
fn keeps_the_supersedes_link() {
    let mut p = parts(mismatch_payload());
    p.supersedes = Some(EventId::new("evt_0"));
    let event = Event::new(p).unwrap();
    assert_eq!(event.supersedes(), Some(&EventId::new("evt_0")));
}

#[test]
fn rejects_a_payload_that_mixes_currencies() {
    let p = parts(EventPayload::TotalsMismatch {
        field: TotalField::Gross,
        stated: zar(100),
        recomputed: Money::new(100, Currency::Usd),
    });
    assert_eq!(
        Event::new(p),
        Err(invalid("all amounts in one event must share a currency"))
    );
}

#[test]
fn rejects_an_inverted_text_span() {
    let mut p = parts(mismatch_payload());
    p.citation.locator = Some(Locator::TextSpan { start: 10, end: 10 });
    assert_eq!(Event::new(p), Err(invalid("text span must end after it starts")));
}

#[test]
fn rejects_a_bounding_box_outside_the_page() {
    let mut p = parts(mismatch_payload());
    p.citation.locator = Some(Locator::BoundingBox {
        x: 900,
        y: 0,
        width: 200,
        height: 10,
    });
    assert_eq!(
        Event::new(p),
        Err(invalid("bounding box must lie within the page"))
    );
}

#[test]
fn rejects_an_empty_bounding_box() {
    let mut p = parts(mismatch_payload());
    p.citation.locator = Some(Locator::BoundingBox {
        x: 0,
        y: 0,
        width: 0,
        height: 10,
    });
    assert_eq!(
        Event::new(p),
        Err(invalid("bounding box must lie within the page"))
    );
}

// ---------- confidence ----------

#[test]
fn confidence_is_an_integer_between_0_and_1000() {
    assert_eq!(Confidence::new(0).unwrap().get(), 0);
    assert_eq!(Confidence::new(1000).unwrap().get(), 1000);
    assert_eq!(
        Confidence::new(1001),
        Err(invalid("confidence must be between 0 and 1000"))
    );
}

// ---------- payload facts ----------

#[test]
fn mismatch_keeps_stated_and_recomputed_amounts_distinct() {
    let amounts = mismatch_payload().money_fields();
    assert_eq!(amounts, vec![zar(150_000), zar(150_001)]);
}

#[test]
fn line_item_exposes_its_amounts_for_the_number_guard() {
    let line = EventPayload::LineItemBilled {
        description: "Consulting".into(),
        quantity_milli: 10_000,
        unit_price: zar(150_000),
        line_total: zar(1_500_000),
        tax_category: TaxCategory::Standard,
    };
    assert_eq!(line.money_fields(), vec![zar(150_000), zar(1_500_000)]);
}

#[test]
fn searchable_text_is_what_the_document_said_and_nothing_else() {
    let supplier = EventPayload::SupplierIdentified {
        name: "Acme (Pty) Ltd".into(),
        vat_number: Some("4123456789".into()),
    };
    assert_eq!(supplier.searchable_text(), ["Acme (Pty) Ltd", "4123456789"]);
    let number = EventPayload::InvoiceNumberStated {
        number: "INV-1".into(),
    };
    assert_eq!(number.searchable_text(), ["INV-1"]);
    let recipient = EventPayload::RecipientIdentified {
        name: "Client Co".into(),
        vat_number: None,
    };
    assert_eq!(recipient.searchable_text(), ["Client Co"]);
    let totals = EventPayload::TotalsStated {
        net: zar(100),
        vat: zar(15),
        gross: zar(115),
    };
    assert!(totals.searchable_text().is_empty());
}

#[test]
fn a_line_total_mismatch_exposes_both_amounts_and_names_its_line() {
    let mismatch = EventPayload::LineTotalMismatch {
        line: 2,
        stated: zar(110_000),
        recomputed: zar(100_000),
    };
    assert_eq!(mismatch.event_type(), EventType::LineTotalMismatch);
    assert_eq!(mismatch.money_fields(), vec![zar(110_000), zar(100_000)]);
    let json = serde_json::to_value(&mismatch).unwrap();
    assert_eq!(json["type"], "line_total_mismatch");
    assert_eq!(json["line"], 2);
}

#[test]
fn payloads_without_money_expose_no_amounts() {
    let supplier = EventPayload::SupplierIdentified {
        name: "Acme (Pty) Ltd".into(),
        vat_number: Some("4123456789".into()),
    };
    assert!(supplier.money_fields().is_empty());
    let recipient = EventPayload::RecipientIdentified {
        name: "Client Co".into(),
        vat_number: None,
    };
    assert!(recipient.money_fields().is_empty());
    let number = EventPayload::InvoiceNumberStated {
        number: "INV-000001".into(),
    };
    assert!(number.money_fields().is_empty());
    assert_eq!(recipient.event_type(), EventType::RecipientIdentified);
    assert_eq!(number.event_type(), EventType::InvoiceNumberStated);
}

// ---------- JSON ----------

fn full_event() -> Event {
    let mut p = parts(mismatch_payload());
    p.citation.locator = Some(Locator::BoundingBox {
        x: 100,
        y: 200,
        width: 300,
        height: 50,
    });
    p.extractor = Extractor::Model {
        name: "claude-haiku-5-5".into(),
        prompt_version: "v3".into(),
    };
    p.confidence = Confidence::new(950).unwrap();
    p.supersedes = Some(EventId::new("evt_0"));
    Event::new(p).unwrap()
}

fn full_event_json() -> serde_json::Value {
    json!({
        "id": "evt_1",
        "date": "2026-03-31",
        "payload": {
            "type": "totals_mismatch",
            "field": "vat",
            "stated": { "minor": 150000, "currency": "ZAR" },
            "recomputed": { "minor": 150001, "currency": "ZAR" }
        },
        "citation": {
            "document": "doc_1",
            "page": 2,
            "locator": { "type": "bounding_box", "x": 100, "y": 200, "width": 300, "height": 50 }
        },
        "extractor": { "type": "model", "name": "claude-haiku-5-5", "prompt_version": "v3" },
        "confidence": 950,
        "supersedes": "evt_0"
    })
}

#[test]
fn json_shape_is_stable() {
    // Golden test: JSON is the canonical export format, so changing this
    // shape is a breaking change and must be deliberate.
    assert_eq!(serde_json::to_value(full_event()).unwrap(), full_event_json());
}

#[test]
fn json_round_trips() {
    let text = serde_json::to_string(&full_event()).unwrap();
    let back: Event = serde_json::from_str(&text).unwrap();
    assert_eq!(back, full_event());
}

#[test]
fn absent_date_and_supersedes_are_omitted_from_json() {
    let mut p = parts(mismatch_payload());
    p.date = None;
    let value = serde_json::to_value(Event::new(p).unwrap()).unwrap();
    assert!(value.get("date").is_none());
    assert!(value.get("supersedes").is_none());
    assert!(value["citation"].get("locator").is_none());
}

#[test]
fn deserialising_applies_the_same_checks_as_new() {
    let mut value = full_event_json();
    value["supersedes"] = json!("evt_1"); // supersedes itself
    assert!(serde_json::from_value::<Event>(value).is_err());
}

#[test]
fn deserialising_rejects_page_zero() {
    let mut value = full_event_json();
    value["citation"]["page"] = json!(0);
    assert!(serde_json::from_value::<Event>(value).is_err());
}

#[test]
fn deserialising_rejects_out_of_range_confidence() {
    let mut value = full_event_json();
    value["confidence"] = json!(1001);
    assert!(serde_json::from_value::<Event>(value).is_err());
}

#[test]
fn deserialising_rejects_an_invalid_date() {
    let mut value = full_event_json();
    value["date"] = json!("2026-02-30");
    assert!(serde_json::from_value::<Event>(value).is_err());
}
