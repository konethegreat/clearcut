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

/// The formats whose bytes are text. The workbook, the bundle and the PDF are checked as bytes.
const TEXT_FORMATS: [RegisterFormat; 4] = [
    RegisterFormat::Json,
    RegisterFormat::Html,
    RegisterFormat::Markdown,
    RegisterFormat::Text,
];

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
        assert_eq!(RegisterFormat::parse(format.name()), Some(format));
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
    assert_eq!(RegisterFormat::parse("csv"), Some(RegisterFormat::CsvBundle));
    assert_eq!(RegisterFormat::parse("zip"), None);
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
    for format in TEXT_FORMATS {
        let text = String::from_utf8(render_register(format, &register).unwrap().bytes).unwrap();
        // The figures below also appear in the Documents and To check sections, so each format
        // needs its own marker that only the stated totals section provides.
        let totals_marker = match format {
            RegisterFormat::Json => "\"stated_totals\"",
            RegisterFormat::Html => "<h2>Stated totals</h2>",
            RegisterFormat::Markdown => "## Stated totals",
            RegisterFormat::Text => "STATED TOTALS",
            // Only the text formats reach this loop; the binary files are checked as bytes.
            RegisterFormat::Xlsx | RegisterFormat::CsvBundle | RegisterFormat::Pdf => {
                unreachable!("the workbook, the bundle and the PDF are checked as bytes")
            }
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

/// Every file inside a workbook or a bundle, by name.
fn entries(bytes: Vec<u8>) -> Vec<(String, String)> {
    use std::io::Read;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("a zip file");
    let names: Vec<String> = archive
        .file_names()
        .map(|name| name.expect("a file name").into_owned())
        .collect();
    let mut out = Vec::new();
    for name in names {
        let mut text = String::new();
        archive
            .by_name(&name)
            .expect(&name)
            .read_to_string(&mut text)
            .unwrap();
        out.push((name, text));
    }
    out
}

fn file_named(files: &[(String, String)], name: &str) -> String {
    files
        .iter()
        .find(|(entry, _)| entry == name)
        .map(|(_, text)| text.clone())
        .unwrap_or_else(|| panic!("{name} is missing"))
}

#[test]
fn the_workbook_has_a_sheet_for_each_table_and_real_numbers() {
    let file = render_register(RegisterFormat::Xlsx, &register()).unwrap();
    assert_eq!(
        file.content_type,
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
    );
    assert!(file.bytes.starts_with(b"PK"));
    let files = entries(file.bytes);
    let sheets = files
        .iter()
        .filter(|(name, _)| name.starts_with("xl/worksheets/sheet"))
        .count();
    assert_eq!(sheets, 6);
    let workbook = file_named(&files, "xl/workbook.xml");
    for sheet in [
        "Register",
        "Lines",
        "VAT by category",
        "Stated totals",
        "To check",
        "Manifest",
    ] {
        assert!(workbook.contains(sheet), "{sheet} is missing from the workbook");
    }
    let all: String = files.into_iter().map(|(_, text)| text).collect();
    assert!(all.contains("INV-1"));
    assert!(all.contains("Consulting"));
    // Every row is traceable back to the document it came from.
    assert!(all.contains("Document id"));
    assert!(all.contains("invoice.pdf"));
    // R1 500.00 is stored as the number 1500, from exact minor units, not as text.
    assert!(all.contains("<v>1500</v>"), "the amount should be a number");
}

#[test]
fn the_bundle_holds_a_file_for_every_table_and_the_manifest() {
    let file = render_register(RegisterFormat::CsvBundle, &register()).unwrap();
    assert_eq!(file.extension, "zip");
    assert_eq!(file.content_type, "application/zip");
    let files = entries(file.bytes);
    for name in [
        "register.csv",
        "lines.csv",
        "vat-by-category.csv",
        "stated-totals.csv",
        "reconciliation.csv",
        "to-check.csv",
        "manifest.json",
    ] {
        assert!(files.iter().any(|(entry, _)| entry == name), "{name} is missing");
    }
    let documents = file_named(&files, "register.csv");
    assert!(documents.starts_with("document_id,filename,"));
    assert!(documents.contains("invoice.pdf"));
    let manifest = file_named(&files, "manifest.json");
    assert!(serde_json::from_str::<serde_json::Value>(&manifest).is_ok());
}

#[test]
fn text_in_the_bundle_cannot_become_a_formula() {
    let mut register = register();
    register.rows[0].supplier = Some("=HYPERLINK(\"http://example.com\")".into());
    let file = render_register(RegisterFormat::CsvBundle, &register).unwrap();
    let documents = file_named(&entries(file.bytes), "register.csv");
    assert!(documents.contains("'=HYPERLINK"));
    assert!(!documents.contains(",=HYPERLINK"));
}

#[test]
fn text_in_the_workbook_cannot_become_a_formula() {
    let mut register = register();
    register.rows[0].supplier = Some("=HYPERLINK(\"http://example.com\")".into());
    let file = render_register(RegisterFormat::Xlsx, &register).unwrap();
    let all: String = entries(file.bytes).into_iter().map(|(_, text)| text).collect();
    assert!(all.contains("=HYPERLINK(\"http://example.com\")"));
    assert!(!all.contains("<f>"));
}

#[test]
fn the_register_pdf_is_a_pdf() {
    let file = render_register(RegisterFormat::Pdf, &register()).unwrap();
    assert_eq!(file.content_type, "application/pdf");
    assert!(file.bytes.starts_with(b"%PDF-"));
    assert!(file.bytes.windows(5).any(|w| w == b"%%EOF"));
}
