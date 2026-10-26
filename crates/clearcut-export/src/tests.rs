// Written by Kone Tshivhinda & Claude
use super::*;
use clearcut_core::event::{Citation, Confidence, DocumentId, EventId, EventParts, EventPayload, Extractor};
use clearcut_core::{Currency, Money, TaxCategory};
use std::io::{Cursor, Read};
use std::num::NonZeroU32;

/// Text that must never be treated as markup, a formula or an instruction.
const HOSTILE: &str = "=HYPERLINK(\"http://evil.example\",\"x\") <script>alert(1)</script> & \"quoted\"";

fn event(id: &str, payload: EventPayload, extractor: Extractor, per_mille: u16) -> Event {
    Event::new(EventParts {
        id: EventId::new(id),
        date: None,
        payload,
        citation: Citation {
            document: DocumentId::new("doc_1"),
            page: NonZeroU32::MIN,
            locator: None,
        },
        extractor,
        confidence: Confidence::new(per_mille).unwrap(),
        supersedes: None,
    })
    .unwrap()
}

fn zar(minor: i64) -> Money {
    Money::new(minor, Currency::Zar)
}

fn events() -> Vec<Event> {
    let model = Extractor::Model {
        name: "claude-haiku-5-5".into(),
        prompt_version: "v1".into(),
    };
    vec![
        event(
            "doc_1:r1:supplier",
            EventPayload::SupplierIdentified {
                name: format!("M\u{fc}ller & S\u{f6}hne {HOSTILE}"),
                vat_number: Some("4123456789".into()),
            },
            model.clone(),
            950,
        ),
        event(
            "doc_1:r1:line-1",
            EventPayload::LineItemBilled {
                description: HOSTILE.into(),
                quantity_milli: 10_000,
                unit_price: zar(150_000),
                line_total: zar(1_500_000),
                tax_category: TaxCategory::Standard,
            },
            model,
            950,
        ),
        event(
            "doc_1:r1:totals-stated",
            EventPayload::TotalsStated {
                net: zar(1_500_000),
                vat: zar(225_000),
                gross: zar(1_725_000),
            },
            Extractor::Human {
                user: "api_key:test".into(),
            },
            990,
        ),
        event(
            "doc_1:totals-mismatch-gross",
            EventPayload::TotalsMismatch {
                field: clearcut_core::event::TotalField::Gross,
                stated: zar(1_725_000),
                recomputed: zar(1_725_001),
            },
            Extractor::System,
            1000,
        ),
    ]
}

fn input(events: &[Event]) -> ExportInput<'_> {
    ExportInput {
        document: DocumentFacts {
            id: "doc_1".into(),
            filename: format!("invoice {HOSTILE}.pdf"),
            status: "done".into(),
            page_count: Some(1),
        },
        events,
    }
}

fn make(format: Format) -> Rendered {
    let events = events();
    render(format, &input(&events)).unwrap()
}

fn unzip(bytes: &[u8], name: &str) -> String {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).expect("a zip file");
    let mut text = String::new();
    archive
        .by_name(name)
        .expect(name)
        .read_to_string(&mut text)
        .unwrap();
    text
}

#[test]
fn every_format_makes_a_file_of_the_right_kind() {
    for format in Format::ALL {
        let file = make(format);
        assert!(!file.bytes.is_empty(), "{format:?}");
        assert_eq!(file.extension, format.extension());
        assert_eq!(Format::parse(format.extension()), Some(format));
    }
    assert!(serde_json::from_slice::<serde_json::Value>(&make(Format::Json).bytes).is_ok());
    assert!(make(Format::Csv).bytes.starts_with(b"event_id,date,group,type"));
    assert!(make(Format::Markdown).bytes.starts_with(b"# invoice"));
    assert!(make(Format::Html).bytes.starts_with(b"<!doctype html>"));
    assert!(make(Format::Xlsx).bytes.starts_with(b"PK"));
    assert!(make(Format::Docx).bytes.starts_with(b"PK"));
    let pdf = make(Format::Pdf).bytes;
    assert!(pdf.starts_with(b"%PDF-") && pdf.windows(5).any(|w| w == b"%%EOF"));
    assert_eq!(Format::parse("exe"), None);
}

#[test]
fn text_from_the_document_cannot_become_markup_or_a_formula() {
    // HTML: every character that matters is escaped, so nothing the document says is markup.
    let html = String::from_utf8(make(Format::Html).bytes).unwrap();
    assert!(!html.contains("<script>") && html.contains("&lt;script&gt;"));
    assert!(html.contains("M\u{fc}ller &amp; S\u{f6}hne"));

    // CSV: a cell that starts with a formula character is neutralised.
    let csv = String::from_utf8(make(Format::Csv).bytes).unwrap();
    assert!(!csv
        .lines()
        .any(|line| line.split(',').any(|cell| cell.starts_with("=HYPERLINK"))));

    // Excel: the text is a plain string in the shared strings, and no cell anywhere is a formula.
    let xlsx = make(Format::Xlsx).bytes;
    assert!(unzip(&xlsx, "xl/sharedStrings.xml").contains("HYPERLINK"));
    for sheet in 1..=5 {
        assert!(
            !unzip(&xlsx, &format!("xl/worksheets/sheet{sheet}.xml")).contains("<f>"),
            "sheet {sheet}"
        );
    }
}

#[test]
fn the_workbook_has_its_five_sheets_and_real_numbers() {
    let xlsx = make(Format::Xlsx).bytes;
    let workbook = unzip(&xlsx, "xl/workbook.xml");
    for name in ["Summary", "Events", "Line items", "Checks", "Sources"] {
        assert!(workbook.contains(&format!("name=\"{name}\"")), "{name}");
    }
    // The line total R15 000.00 is the number 15000 in a cell, not text.
    assert!(unzip(&xlsx, "xl/worksheets/sheet3.xml").contains("<v>15000</v>"));
    // The mismatch the checks found is on the Checks sheet.
    assert!(unzip(&xlsx, "xl/sharedStrings.xml").contains("critical"));
}

#[test]
fn the_word_file_carries_the_report_and_the_notice() {
    let docx = make(Format::Docx).bytes;
    let body = unzip(&docx, "word/document.xml");
    assert!(body.contains("Line items") && body.contains("Everything that was read"));
    // The hostile text is in the file as escaped text, not as an element.
    assert!(!body.contains("<script>"));
    assert!(body.contains("HYPERLINK"));
    let footer = (1..=3).find_map(|n| {
        let mut archive = zip::ZipArchive::new(Cursor::new(&docx[..])).unwrap();
        let mut text = String::new();
        archive
            .by_name(&format!("word/footer{n}.xml"))
            .ok()?
            .read_to_string(&mut text)
            .ok()?;
        Some(text)
    });
    assert!(footer.is_some_and(|text| text.contains("Check figures that matter")));
}

#[test]
fn the_same_document_gives_the_same_figures_in_every_format() {
    // One model, so a figure cannot differ between formats.
    for (format, needle) in [
        (Format::Html, "R15\u{a0}000.00"),
        (Format::Text, "R15\u{a0}000.00"),
    ] {
        let text = String::from_utf8(make(format).bytes).unwrap();
        assert!(text.contains(needle), "{format:?}");
    }
    let json = String::from_utf8(make(Format::Json).bytes).unwrap();
    assert!(
        json.contains("1500000"),
        "the exact minor units are in the canonical file"
    );
}

/// Writes every format to `CLEARCUT_DUMP_DIR` so a person can open them (not run by default).
#[test]
#[ignore = "writes files for a person to look at"]
fn dump_every_format_for_a_look() {
    let Ok(dir) = std::env::var("CLEARCUT_DUMP_DIR") else {
        return;
    };
    for format in Format::ALL {
        let file = make(format);
        std::fs::write(format!("{dir}/sample.{}", file.extension), file.bytes).unwrap();
    }
}
