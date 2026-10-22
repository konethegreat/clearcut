// Written by Kone Tshivhinda & Claude
use super::*;
use crate::check::check_set;
use crate::hash::sha256_hex;
use crate::label::{Split, Tier};
use clearcut_core::{Currency, TaxCategory};
use std::fs;
use std::path::PathBuf;

fn case(id: &str) -> SampleCase {
    cases()
        .into_iter()
        .find(|c| c.label.case == id)
        .unwrap_or_else(|| panic!("no sample case {id}"))
}

#[test]
fn there_are_fifteen_cases_numbered_in_order_and_ten_are_dev_and_five_test() {
    let all = cases();
    let ids: Vec<&str> = all.iter().map(|c| c.label.case.as_str()).collect();
    let expected: Vec<String> = (1..=15).map(|n| format!("g-{n:04}")).collect();
    assert_eq!(ids, expected.iter().map(String::as_str).collect::<Vec<_>>());
    assert_eq!(all.iter().filter(|c| c.label.split == Split::Dev).count(), 10);
    assert_eq!(all.iter().filter(|c| c.label.split == Split::Test).count(), 5);
    assert!(all.iter().all(|c| c.label.provenance.tier == Tier::Synthetic));
}

#[test]
fn generating_twice_gives_exactly_the_same_cases() {
    assert_eq!(cases(), cases());
}

#[test]
fn each_label_records_its_documents_real_hash_and_page_count() {
    for c in cases() {
        assert_eq!(
            c.label.document.sha256,
            sha256_hex(&c.document),
            "{}",
            c.label.case
        );
        assert_eq!(c.label.document.pages as usize, c.pages.len(), "{}", c.label.case);
        let pdf = String::from_utf8(c.document.clone()).unwrap();
        assert!(
            pdf.contains(&format!("/Count {}", c.pages.len())),
            "{}",
            c.label.case
        );
    }
}

#[test]
fn what_the_label_says_is_printed_on_the_page_it_cites() {
    for c in cases() {
        let id = &c.label.case;
        let truth = &c.label.truth;
        let on = |page: u32, needle: &str| {
            let lines = &c.pages[page as usize - 1];
            assert!(
                lines.iter().any(|l| l.contains(needle)),
                "{id}: {needle:?} is not on page {page}: {lines:?}"
            );
        };
        if let Some(n) = &truth.invoice_number {
            on(n.page.get(), &n.value);
        }
        if let Some(d) = &truth.invoice_date {
            on(d.page.get(), &d.value.to_string());
        }
        if let Some(s) = &truth.supplier {
            on(s.page.get(), &s.value.name);
            if let Some(a) = &s.value.address {
                on(s.page.get(), a);
            }
            if let Some(v) = &s.value.vat_number {
                on(s.page.get(), v);
            }
        }
        if let Some(r) = &truth.recipient {
            on(r.page.get(), &r.value.name);
        }
        for line in &truth.lines {
            on(line.page.get(), &line.description);
            on(line.page.get(), &line.line_total);
        }
        if let Some(t) = &truth.stated_totals {
            on(t.page.get(), &t.net);
            on(t.page.get(), &t.vat);
            on(t.page.get(), &t.gross);
        }
        if let Some(due) = &truth.due {
            on(due.page.get(), &due.date.to_string());
        }
        if let Some(note) = &truth.credit_note {
            on(note.page.get(), &note.original_invoice_number);
        }
    }
}

#[test]
fn the_sample_set_passes_its_own_check_and_meets_the_shape_targets() {
    let dir = tempfile::tempdir().unwrap();
    write_sample(dir.path()).unwrap();
    let report = check_set(dir.path()).unwrap();
    assert!(report.is_ok(), "{}", report.render());
    let c = &report.coverage;
    assert_eq!(c.cases, 15);
    assert_eq!((c.dev, c.test), (10, 5));
    assert_eq!(c.synthetic, 15);
    assert!(c.suppliers >= 12, "{}", c.suppliers);
    assert!(c.with_expected_findings >= 4, "{}", c.with_expected_findings);
    for tag in [
        "text-layer",
        "multi-page",
        "credit-note",
        "zero-rated",
        "exempt",
        "no-vat",
        "foreign-currency",
        "wrong-totals",
    ] {
        assert!(c.tags.contains_key(tag), "no case is tagged {tag}");
    }
}

#[test]
fn the_interesting_cases_are_really_what_they_claim() {
    // A foreign-currency invoice.
    assert_eq!(case("g-0007").label.truth.currency, Currency::Usd);
    // A credit note refers back to the invoice it adjusts.
    let note = case("g-0008").label.truth.credit_note.unwrap();
    assert_eq!(note.original_invoice_number, "PP-2031");
    // Tax categories other than standard are present.
    let categories: Vec<TaxCategory> = cases()
        .iter()
        .flat_map(|c| c.label.truth.lines.iter().map(|l| l.tax))
        .collect();
    for wanted in [
        TaxCategory::Standard,
        TaxCategory::ZeroRated,
        TaxCategory::Exempt,
        TaxCategory::OutOfScope,
    ] {
        assert!(categories.contains(&wanted), "{wanted:?}");
    }
    // Rounding: 2.5 x 99.99 = 249.975, which rounds half away from zero to 249.98.
    let rounding = case("g-0015").label.truth;
    assert_eq!(rounding.lines[0].quantity, "2.5");
    assert_eq!(rounding.lines[0].unit_price, "99.99");
    assert_eq!(rounding.lines[0].line_total, "249.98");
    // Planted mistakes, and exactly what the validator should say about each.
    assert_eq!(
        case("g-0009").label.expect.findings,
        ["totals_mismatch:gross", "totals_mismatch:vat"]
    );
    assert_eq!(case("g-0010").label.expect.findings, ["line_total_mismatch:2"]);
    assert_eq!(case("g-0013").label.expect.findings, ["due_before_issue"]);
    assert_eq!(
        case("g-0014").label.expect.findings,
        ["compliance:supplier_address"]
    );
    assert_eq!(
        case("g-0006").label.expect.findings,
        ["compliance:supplier_vat_number"]
    );
    assert!(case("g-0001").label.expect.findings.is_empty());
}

#[test]
fn multi_page_cases_spread_their_lines_and_keep_the_totals_on_the_last_page() {
    let c = case("g-0003");
    assert_eq!(c.pages.len(), 3);
    let line_pages: Vec<u32> = c.label.truth.lines.iter().map(|l| l.page.get()).collect();
    assert_eq!(line_pages, [1, 1, 1, 1, 2, 2, 2, 2, 3]);
    assert_eq!(c.label.truth.stated_totals.as_ref().unwrap().page.get(), 3);
    assert_eq!(c.label.truth.supplier.as_ref().unwrap().page.get(), 1);
}

#[test]
fn writing_refuses_a_folder_that_already_has_something_in_it() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("keep.txt"), "x").unwrap();
    assert!(matches!(write_sample(dir.path()), Err(SampleError::NotEmpty)));
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn writing_creates_a_missing_folder_and_accepts_an_empty_one() {
    let dir = tempfile::tempdir().unwrap();
    write_sample(&dir.path().join("a").join("b")).unwrap();
    assert!(dir
        .path()
        .join("a")
        .join("b")
        .join("g-0015")
        .join("document.pdf")
        .is_file());
    let empty = dir.path().join("empty");
    fs::create_dir(&empty).unwrap();
    write_sample(&empty).unwrap();
    assert!(empty.join("g-0001").join("label.json").is_file());
}

#[test]
fn the_label_files_are_stable_text_ending_in_a_newline() {
    let dir = tempfile::tempdir().unwrap();
    write_sample(dir.path()).unwrap();
    let text = fs::read_to_string(dir.path().join("g-0001").join("label.json")).unwrap();
    assert!(text.ends_with("}\n"));
    assert!(text.contains("\n  \"case\": \"g-0001\""));
}

/// The sample set kept in the repository (`crates/clearcut-eval/sample`).
fn committed() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("sample")
}

#[test]
fn the_committed_sample_set_is_exactly_what_the_generator_produces() {
    // If this fails, run `cargo run -p clearcut-eval -- sample crates/clearcut-eval/sample`
    // into an empty folder and commit the result.
    let mut folders: Vec<String> = fs::read_dir(committed())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    folders.sort();
    let expected: Vec<String> = cases().iter().map(|c| c.label.case.clone()).collect();
    assert_eq!(folders, expected);

    for c in cases() {
        let folder = committed().join(&c.label.case);
        let on_disk = Label::parse(&fs::read(folder.join("label.json")).unwrap()).unwrap();
        assert_eq!(on_disk, c.label, "{}", c.label.case);
        assert_eq!(
            fs::read(folder.join("document.pdf")).unwrap(),
            c.document,
            "{}",
            c.label.case
        );
    }
}

#[test]
fn the_committed_sample_set_passes_the_check() {
    let report = check_set(&committed()).unwrap();
    assert!(report.is_ok(), "{}", report.render());
}
