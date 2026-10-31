// Written by Kone Tshivhinda & Claude
//! Layout tests for the register files. They build the register in the core, so the figures are
//! the ones a real selection would produce.

use super::*;
use clearcut_core::document::DocumentStatus;
use clearcut_core::event::{
    Citation, Confidence, DocumentId, Event, EventId, EventParts, EventPayload, Extractor,
};
use clearcut_core::register::{build, Register, RegisterFacts, RegisterInput, Selection};
use clearcut_core::Money;
use clearcut_core::TaxCategory;
use std::num::NonZeroU32;
use time::macros::date as d;

fn event(doc: &str, id: &str, payload: EventPayload) -> Event {
    Event::new(EventParts {
        id: EventId::new(format!("{doc}:{id}")),
        date: Some(d!(2026 - 09 - 15)),
        payload,
        citation: Citation {
            document: DocumentId::new(doc),
            page: NonZeroU32::MIN,
            locator: None,
        },
        extractor: Extractor::System,
        confidence: Confidence::CERTAIN,
        supersedes: None,
    })
    .unwrap()
}

fn register() -> Register {
    let doc = "d1";
    let facts = RegisterFacts {
        id: doc.into(),
        filename: "invoice.pdf".into(),
        uploaded_at: time::OffsetDateTime::UNIX_EPOCH,
        status: DocumentStatus::Done,
        page_count: Some(1),
        pages: Vec::new(),
        events: vec![
            event(
                doc,
                "supplier",
                EventPayload::SupplierIdentified {
                    name: "<script>alert(1)</script> [Pay](https://evil)".into(),
                    vat_number: Some("4123456789".into()),
                },
            ),
            event(
                doc,
                "number",
                EventPayload::InvoiceNumberStated {
                    number: "INV-1".into(),
                },
            ),
            event(
                doc,
                "line",
                EventPayload::LineItemBilled {
                    description: "Consulting".into(),
                    quantity_milli: 1_000,
                    unit_price: Money::zar(150_000),
                    line_total: Money::zar(150_000),
                    tax_category: TaxCategory::Standard,
                },
            ),
            event(
                doc,
                "vat",
                EventPayload::VatCalculated {
                    category: TaxCategory::Standard,
                    taxable: Money::zar(150_000),
                    rate_bps: 1_500,
                    vat: Money::zar(22_500),
                },
            ),
            event(
                doc,
                "totals",
                EventPayload::TotalsStated {
                    net: Money::zar(150_000),
                    vat: Money::zar(22_500),
                    gross: Money::zar(172_500),
                },
            ),
        ],
    };
    build(RegisterInput {
        selection: Selection::Documents {
            ids: vec![doc.into()],
        },
        only_ready: false,
        undated_in_tenant: 0,
        facts: vec![facts],
    })
    .unwrap()
}

#[test]
fn every_register_format_makes_a_file_of_the_right_kind() {
    let register = register();
    for format in RegisterFormat::ALL {
        let file = render_register(format, &register).unwrap();
        assert!(!file.bytes.is_empty(), "{format:?}");
        assert_eq!(file.extension, format.extension());
        assert_eq!(RegisterFormat::parse(format.extension()), Some(format));
    }
    let json = render_register(RegisterFormat::Json, &register).unwrap();
    assert!(serde_json::from_slice::<serde_json::Value>(&json.bytes).is_ok());
    assert!(render_register(RegisterFormat::Html, &register)
        .unwrap()
        .bytes
        .starts_with(b"<!doctype html>"));
    assert!(render_register(RegisterFormat::Markdown, &register)
        .unwrap()
        .bytes
        .starts_with(b"# Register"));
    assert!(render_register(RegisterFormat::Text, &register)
        .unwrap()
        .bytes
        .starts_with(b"REGISTER"));
    assert_eq!(RegisterFormat::parse("exe"), None);
}

#[test]
fn text_from_the_document_cannot_become_markup() {
    let register = register();
    let html = String::from_utf8(render_register(RegisterFormat::Html, &register).unwrap().bytes).unwrap();
    assert!(!html.contains("<script>"));
    assert!(html.contains("&lt;script&gt;"));
    // Brackets are plain text in HTML, so a Markdown link cannot form: there is no anchor.
    assert!(!html.contains("href=\"https://evil\""));

    let markdown = String::from_utf8(
        render_register(RegisterFormat::Markdown, &register)
            .unwrap()
            .bytes,
    )
    .unwrap();
    assert!(!markdown.contains("<script>"));
    assert!(markdown.contains("\\<script\\>"));
    // The document-derived supplier name carries a Markdown link; every bracket must be escaped.
    assert!(!markdown.contains("[Pay]("));
    assert!(markdown.contains("\\[Pay\\]("));

    // JSON is data, not markup: the payload is preserved verbatim instead of escaped.
    let json = String::from_utf8(render_register(RegisterFormat::Json, &register).unwrap().bytes).unwrap();
    assert!(json.contains("[Pay](https://evil)"));
}

#[test]
fn the_same_figures_are_in_every_format() {
    let register = register();
    for format in RegisterFormat::ALL {
        let text = String::from_utf8(render_register(format, &register).unwrap().bytes).unwrap();
        // The figures below also appear in the Documents and To check sections, so each format
        // needs its own marker that only the stated totals section provides.
        let totals_marker = match format {
            RegisterFormat::Json => "\"stated_totals\"",
            RegisterFormat::Html => "<h2>Stated totals</h2>",
            RegisterFormat::Markdown => "## Stated totals",
            RegisterFormat::Text => "STATED TOTALS",
        };
        assert!(
            text.contains(totals_marker),
            "{format:?} carries its stated totals section"
        );
        assert!(text.contains("INV-1"), "{format:?}");
        assert!(text.contains("R1\u{a0}500.00"), "{format:?}");
        assert!(text.contains("R1\u{a0}725.00"), "{format:?}");
        assert!(text.contains("R225.00"), "{format:?}");
    }
}
