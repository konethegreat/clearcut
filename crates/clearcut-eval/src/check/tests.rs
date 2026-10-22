// Written by Kone Tshivhinda & Claude
use super::*;
use crate::hash::sha256_hex;
use crate::keys::Difference;
use crate::label::tests::base;
use crate::label::{LabelProblem, ParseError, TruthError, TruthErrorKind};
use crate::personal::PersonalData;
use serde_json::{json, Value};
use std::fs;
use tempfile::TempDir;

/// A gold-set folder in a temporary directory, built case by case.
struct Set {
    dir: TempDir,
    count: u32,
}

impl Set {
    fn new() -> Set {
        Set {
            dir: tempfile::tempdir().unwrap(),
            count: 0,
        }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn doc_bytes(id: &str) -> Vec<u8> {
        format!("%PDF-1.4 pretend document for {id}").into_bytes()
    }

    /// Adds a correct case, after letting `edit` change its label.
    fn add_with(&mut self, id: &str, edit: impl FnOnce(&mut Value)) -> &mut Set {
        self.count += 1;
        let n = self.count;
        let doc = Set::doc_bytes(id);
        let mut value = base();
        value["case"] = json!(id);
        value["document"]["sha256"] = json!(sha256_hex(&doc));
        value["truth"]["supplier"]["value"]["name"] = json!(format!("Supplier Number {n} (Pty) Ltd"));
        value["truth"]["supplier"]["value"]["vat_number"] = json!(format!("4{n:09}"));
        edit(&mut value);
        let folder = self.path().join(id);
        fs::create_dir_all(&folder).unwrap();
        fs::write(
            folder.join("label.json"),
            serde_json::to_string_pretty(&value).unwrap(),
        )
        .unwrap();
        fs::write(folder.join("document.pdf"), doc).unwrap();
        self
    }

    fn add(&mut self, id: &str) -> &mut Set {
        self.add_with(id, |_| {})
    }

    fn check(&self) -> CheckReport {
        check_set(self.path()).unwrap()
    }
}

fn problems_of(report: &CheckReport, case: &str) -> Vec<Problem> {
    report
        .problems
        .iter()
        .filter(|p| p.case.as_deref() == Some(case))
        .map(|p| p.problem.clone())
        .collect()
}

fn set_problems(report: &CheckReport) -> Vec<Problem> {
    report
        .problems
        .iter()
        .filter(|p| p.case.is_none())
        .map(|p| p.problem.clone())
        .collect()
}

#[test]
fn a_correct_set_has_no_problems() {
    let mut set = Set::new();
    set.add("g-0001")
        .add_with("g-0002", |v| v["split"] = json!("test"));
    let report = set.check();
    assert!(report.is_ok(), "{:?}", report.problems);
    assert_eq!(report.cases, 2);
    assert!(report.problems.is_empty());
}

#[test]
fn a_missing_folder_is_an_error_and_an_empty_one_is_a_problem() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nope");
    assert!(matches!(check_set(&missing), Err(CheckError::Unreadable(_))));
    // A file where the folder should be.
    let file = dir.path().join("file");
    fs::write(&file, "x").unwrap();
    assert!(matches!(check_set(&file), Err(CheckError::Unreadable(_))));

    let empty = tempfile::tempdir().unwrap();
    let report = check_set(empty.path()).unwrap();
    assert!(!report.is_ok());
    assert_eq!(set_problems(&report), [Problem::NoCases]);
    assert_eq!(report.cases, 0);
}

#[test]
fn a_case_without_its_files_is_reported() {
    let mut set = Set::new();
    set.add("g-0001").add("g-0002");
    fs::remove_file(set.path().join("g-0001").join("label.json")).unwrap();
    fs::remove_file(set.path().join("g-0002").join("document.pdf")).unwrap();
    let report = set.check();
    assert_eq!(problems_of(&report, "g-0001"), [Problem::LabelMissing]);
    assert_eq!(problems_of(&report, "g-0002"), [Problem::DocumentMissing]);
}

#[test]
fn a_label_that_cannot_be_parsed_is_reported_by_position_only() {
    let mut set = Set::new();
    set.add("g-0001").add("g-0002");
    fs::write(set.path().join("g-0001").join("label.json"), "{ nope").unwrap();
    let mut value = base();
    value["case"] = json!("g-0002");
    value["surprise"] = json!(true);
    fs::write(
        set.path().join("g-0002").join("label.json"),
        serde_json::to_string_pretty(&value).unwrap(),
    )
    .unwrap();
    let report = set.check();
    assert!(matches!(
        problems_of(&report, "g-0001").as_slice(),
        [Problem::Parse(ParseError::NotJson { .. })]
    ));
    assert!(matches!(
        problems_of(&report, "g-0002").as_slice(),
        [Problem::Parse(ParseError::WrongShape { .. })]
    ));
}

#[test]
fn an_enormous_label_is_refused_without_reading_it_all() {
    let mut set = Set::new();
    set.add("g-0001");
    let big = vec![b' '; 2 * 1024 * 1024];
    fs::write(set.path().join("g-0001").join("label.json"), big).unwrap();
    assert_eq!(problems_of(&set.check(), "g-0001"), [Problem::LabelTooLarge]);
}

#[test]
fn problems_found_in_the_label_itself_are_passed_on() {
    let mut set = Set::new();
    set.add_with("g-0001", |v| {
        v["tags"] = json!(["Bad Tag"]);
        v["format"] = json!(2);
    });
    let report = set.check();
    assert_eq!(
        problems_of(&report, "g-0001"),
        [
            Problem::Label(LabelProblem::UnsupportedFormat),
            Problem::Label(LabelProblem::BadTag { index: 0 })
        ]
    );
}

#[test]
fn the_label_must_name_the_folder_it_is_in() {
    let mut set = Set::new();
    set.add_with("g-0001", |v| v["case"] = json!("g-0002"));
    assert_eq!(problems_of(&set.check(), "g-0001"), [Problem::CaseFolderMismatch]);
}

#[test]
fn the_document_must_match_its_recorded_hash() {
    let mut set = Set::new();
    set.add("g-0001").add("g-0002");
    fs::write(set.path().join("g-0001").join("document.pdf"), b"changed").unwrap();
    fs::write(set.path().join("g-0002").join("document.pdf"), b"").unwrap();
    let report = set.check();
    assert_eq!(problems_of(&report, "g-0001"), [Problem::HashMismatch]);
    assert_eq!(problems_of(&report, "g-0002"), [Problem::DocumentEmpty]);
}

#[test]
fn a_badly_named_document_is_never_opened() {
    let mut set = Set::new();
    set.add_with("g-0001", |v| {
        v["document"]["file"] = json!("../g-0002/document.pdf")
    });
    let report = set.check();
    // Only the name is reported; no attempt to follow it.
    assert_eq!(
        problems_of(&report, "g-0001"),
        [Problem::Label(LabelProblem::BadFileName)]
    );
}

#[test]
fn the_document_must_be_a_real_file_in_the_folder() {
    let mut set = Set::new();
    set.add("g-0001");
    let path = set.path().join("g-0001").join("document.pdf");
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert_eq!(problems_of(&set.check(), "g-0001"), [Problem::DocumentNotAFile]);
}

#[cfg(unix)]
#[test]
fn a_link_in_place_of_the_document_is_refused() {
    let mut set = Set::new();
    set.add("g-0001");
    let outside = set.path().join("outside.txt");
    fs::write(&outside, Set::doc_bytes("g-0001")).unwrap();
    let path = set.path().join("g-0001").join("document.pdf");
    fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(&outside, &path).unwrap();
    let report = set.check();
    assert_eq!(problems_of(&report, "g-0001"), [Problem::DocumentNotAFile]);
}

#[test]
fn a_case_folder_holds_exactly_the_label_and_the_document() {
    let mut set = Set::new();
    set.add("g-0001");
    let folder = set.path().join("g-0001");
    fs::write(folder.join("notes.txt"), "client phone number").unwrap();
    fs::create_dir(folder.join("scratch")).unwrap();
    fs::write(folder.join("scratch").join("original.pdf"), "x").unwrap();
    let report = set.check();
    assert_eq!(
        problems_of(&report, "g-0001"),
        [Problem::UnexpectedEntries { count: 2 }]
    );
}

#[test]
fn the_root_holds_only_case_folders() {
    let mut set = Set::new();
    set.add("g-0001");
    fs::write(set.path().join("README.txt"), "x").unwrap();
    fs::write(set.path().join("stray.pdf"), "x").unwrap();
    fs::create_dir(set.path().join("Acme Corp invoices")).unwrap();
    fs::create_dir(set.path().join("g-1")).unwrap();
    let report = set.check();
    assert_eq!(
        set_problems(&report),
        [
            Problem::UnexpectedRootFiles { count: 2 },
            Problem::FolderNotACaseId { index: 1 },
            Problem::FolderNotACaseId { index: 2 },
        ]
    );
    assert_eq!(report.cases, 1);
    // The folder names never appear in the report.
    let text = report.render();
    assert!(!text.contains("Acme") && !text.contains("stray"), "{text}");
}

#[test]
fn an_amount_that_is_not_exact_is_reported_with_its_field() {
    let mut set = Set::new();
    set.add_with("g-0001", |v| {
        v["truth"]["lines"][0]["unit_price"] = json!("1,500.00")
    });
    let report = set.check();
    let problems = problems_of(&report, "g-0001");
    assert_eq!(problems.len(), 1);
    match &problems[0] {
        Problem::Truth(TruthError {
            field,
            kind: TruthErrorKind::Decimal(_),
        }) => assert_eq!(field, "lines[0].unit_price"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_validator_must_say_exactly_what_the_label_expects() {
    let mut set = Set::new();
    // A document whose printed VAT is wrong, and the label does not expect that.
    set.add_with("g-0001", |v| {
        v["truth"]["stated_totals"]["vat"] = json!("2000.00")
    });
    // The same document, with the finding expected: all is well.
    set.add_with("g-0002", |v| {
        v["truth"]["stated_totals"]["vat"] = json!("2000.00");
        v["truth"]["stated_totals"]["gross"] = json!("17250.00");
        v["expect"]["findings"] = json!(["totals_mismatch:vat"]);
    });
    // A correct document that expects a finding which does not happen.
    set.add_with("g-0003", |v| {
        v["expect"]["findings"] = json!(["due_before_issue"])
    });
    let report = set.check();
    assert_eq!(
        problems_of(&report, "g-0001"),
        [Problem::FindingsDiffer(Difference {
            missing: vec![],
            unexpected: vec!["totals_mismatch:vat".to_string()]
        })]
    );
    assert!(
        problems_of(&report, "g-0002").is_empty(),
        "{:?}",
        problems_of(&report, "g-0002")
    );
    assert_eq!(
        problems_of(&report, "g-0003"),
        [Problem::FindingsDiffer(Difference {
            missing: vec!["due_before_issue".to_string()],
            unexpected: vec![]
        })]
    );
}

#[test]
fn personal_data_in_the_label_text_is_reported_by_field_and_kind() {
    let mut set = Set::new();
    set.add_with("g-0001", |v| {
        v["truth"]["supplier"]["value"]["address"] = json!("1 Main Rd, call 0821234567");
        v["truth"]["lines"][0]["description"] = json!("Fees for jane@example.com");
    });
    let report = set.check();
    assert_eq!(
        problems_of(&report, "g-0001"),
        [
            Problem::Personal {
                field: "supplier.address".into(),
                kind: PersonalData::LongNumber
            },
            Problem::Personal {
                field: "lines[0].description".into(),
                kind: PersonalData::EmailAddress
            },
        ]
    );
}

#[test]
fn the_same_document_in_two_cases_is_reported_on_the_later_one() {
    let mut set = Set::new();
    set.add("g-0001").add("g-0002");
    let first = fs::read(set.path().join("g-0001").join("document.pdf")).unwrap();
    fs::write(set.path().join("g-0002").join("document.pdf"), &first).unwrap();
    let mut label: Value =
        serde_json::from_slice(&fs::read(set.path().join("g-0002").join("label.json")).unwrap()).unwrap();
    label["document"]["sha256"] = json!(sha256_hex(&first));
    fs::write(set.path().join("g-0002").join("label.json"), label.to_string()).unwrap();
    let report = set.check();
    assert!(problems_of(&report, "g-0001").is_empty());
    assert_eq!(
        problems_of(&report, "g-0002"),
        [Problem::DuplicateDocument {
            other: "g-0001".into()
        }]
    );
}

#[test]
fn a_supplier_may_not_appear_in_both_dev_and_test() {
    let mut set = Set::new();
    set.add_with("g-0001", |v| {
        v["truth"]["supplier"]["value"]["name"] = json!("Shared Supplier (Pty) Ltd");
    });
    set.add_with("g-0002", |v| {
        v["truth"]["supplier"]["value"]["name"] = json!("shared supplier   (PTY) ltd");
        v["split"] = json!("test");
    });
    let report = set.check();
    assert_eq!(
        problems_of(&report, "g-0002"),
        [Problem::SupplierInBothSplits {
            other: "g-0001".into()
        }]
    );

    // By VAT number even when the names differ.
    let mut set = Set::new();
    set.add_with("g-0001", |v| {
        v["truth"]["supplier"]["value"]["vat_number"] = json!("4000000777")
    });
    set.add_with("g-0002", |v| {
        v["truth"]["supplier"]["value"]["vat_number"] = json!("4000000777");
        v["split"] = json!("test");
    });
    assert_eq!(
        problems_of(&set.check(), "g-0002"),
        [Problem::SupplierInBothSplits {
            other: "g-0001".into()
        }]
    );

    // Within one split a supplier may repeat, and cases without a supplier are never matched.
    let mut set = Set::new();
    set.add_with("g-0001", |v| {
        v["truth"]["supplier"]["value"]["vat_number"] = json!("4000000777")
    });
    set.add_with("g-0002", |v| {
        v["truth"]["supplier"]["value"]["vat_number"] = json!("4000000777")
    });
    set.add_with("g-0003", |v| {
        v["truth"].as_object_mut().unwrap().remove("supplier");
        v["expect"]["findings"] = json!([
            "compliance:supplier_address",
            "compliance:supplier_name",
            "compliance:supplier_vat_number"
        ]);
    });
    set.add_with("g-0004", |v| {
        v["truth"].as_object_mut().unwrap().remove("supplier");
        v["split"] = json!("test");
        v["expect"]["findings"] = json!([
            "compliance:supplier_address",
            "compliance:supplier_name",
            "compliance:supplier_vat_number"
        ]);
    });
    let report = set.check();
    assert!(report.is_ok(), "{:?}", report.problems);
}

#[test]
fn coverage_counts_what_the_set_holds() {
    let mut set = Set::new();
    set.add_with("g-0001", |v| v["tags"] = json!(["scan", "multi-page"]));
    set.add_with("g-0002", |v| {
        v["split"] = json!("test");
        v["tags"] = json!(["scan"]);
        v["provenance"]["tier"] = json!("synthetic");
        v["truth"]["stated_totals"]["vat"] = json!("1.00");
        v["expect"]["findings"] = json!(["totals_mismatch:vat"]);
    });
    set.add_with("g-0003", |v| {
        v["provenance"]["tier"] = json!("own");
        v["tags"] = json!([]);
    });
    let report = set.check();
    assert!(report.is_ok(), "{:?}", report.problems);
    let coverage = &report.coverage;
    assert_eq!(coverage.cases, 3);
    assert_eq!((coverage.dev, coverage.test), (2, 1));
    assert_eq!((coverage.own, coverage.consented, coverage.synthetic), (1, 1, 1));
    assert_eq!(coverage.suppliers, 3);
    assert_eq!(coverage.with_expected_findings, 1);
    assert_eq!(coverage.tags.get("scan"), Some(&2));
    assert_eq!(coverage.tags.get("multi-page"), Some(&1));
    assert_eq!(coverage.pages, 6);
}

#[test]
fn the_rendered_report_shows_coverage_against_the_targets_and_the_problems() {
    let mut set = Set::new();
    set.add("g-0001")
        .add_with("g-0002", |v| v["case"] = json!("g-0009"));
    let report = set.check();
    let text = report.render();
    assert!(text.contains("2 cases"), "{text}");
    assert!(text.contains("dev 2"), "{text}");
    assert!(text.contains("60"), "{text}"); // supplier target
    assert!(text.contains("g-0002: "), "{text}");
    assert!(text.contains("problem"), "{text}");

    let mut clean = Set::new();
    clean.add("g-0001");
    assert!(clean.check().render().contains("no problems"));
}

#[test]
fn nothing_from_the_labels_ever_reaches_the_report() {
    let mut set = Set::new();
    set.add_with("g-0001", |v| {
        v["truth"]["supplier"]["value"]["name"] = json!("Secretname Traders");
        v["truth"]["supplier"]["value"]["address"] = json!("99 Secretstreet 0821234567");
        v["truth"]["lines"][0]["description"] = json!("Secretdescription a@secret.example");
        v["truth"]["lines"][0]["unit_price"] = json!("secretprice");
        v["truth"]["invoice_number"]["value"] = json!("SECRET-1");
        v["truth"]["stated_totals"]["vat"] = json!("123456.78");
        v["provenance"]["labelled_by"] = json!("Secretlabeller");
        v["tags"] = json!(["secret tag"]);
    });
    let report = set.check();
    assert!(!report.problems.is_empty());
    let text = format!("{}\n{:?}", report.render(), report.problems);
    for secret in ["Secret", "secret", "SECRET", "123456", "0821234567"] {
        assert!(!text.contains(secret), "{secret} leaked:\n{text}");
    }
}

#[test]
fn cases_are_checked_in_name_order_so_reports_are_stable() {
    let mut set = Set::new();
    set.add("g-0010").add("g-0002").add("g-0001");
    for id in ["g-0010", "g-0002", "g-0001"] {
        fs::remove_file(set.path().join(id).join("label.json")).unwrap();
    }
    let report = set.check();
    let order: Vec<&str> = report.problems.iter().filter_map(|p| p.case.as_deref()).collect();
    assert_eq!(order, ["g-0001", "g-0002", "g-0010"]);
}

#[test]
fn a_label_of_exactly_one_mebibyte_is_read_and_one_byte_more_is_refused() {
    let mut set = Set::new();
    set.add("g-0001").add("g-0002");
    let limit = 1024 * 1024;
    for (id, length) in [("g-0001", limit), ("g-0002", limit + 1)] {
        let path = set.path().join(id).join("label.json");
        let mut text = fs::read_to_string(&path).unwrap();
        // Pad with spaces after the closing brace: still valid JSON.
        text.push_str(&" ".repeat(length - text.len()));
        assert_eq!(text.len(), length);
        fs::write(&path, text).unwrap();
    }
    let report = set.check();
    assert!(
        problems_of(&report, "g-0001").is_empty(),
        "{:?}",
        problems_of(&report, "g-0001")
    );
    assert_eq!(problems_of(&report, "g-0002"), [Problem::LabelTooLarge]);
}

#[test]
fn a_single_unexpected_file_is_enough_to_be_reported() {
    let mut set = Set::new();
    set.add("g-0001");
    fs::write(set.path().join("g-0001").join("notes.txt"), "x").unwrap();
    assert_eq!(
        problems_of(&set.check(), "g-0001"),
        [Problem::UnexpectedEntries { count: 1 }]
    );
}

#[cfg(unix)]
#[test]
fn on_case_sensitive_systems_a_differently_cased_label_is_an_unexpected_file() {
    let mut set = Set::new();
    set.add("g-0001");
    fs::write(set.path().join("g-0001").join("LABEL.JSON"), "{}").unwrap();
    fs::write(set.path().join("g-0001").join("DOCUMENT.PDF"), "x").unwrap();
    assert_eq!(
        problems_of(&set.check(), "g-0001"),
        [Problem::UnexpectedEntries { count: 2 }]
    );
}

#[test]
fn problems_found_across_cases_are_still_listed_in_case_order() {
    let mut set = Set::new();
    set.add("g-0001").add("g-0002").add("g-0003");
    // g-0002 is a copy of g-0001 (found when cases are compared), and g-0003
    // has no label (found while reading it, which happens first).
    let first = fs::read(set.path().join("g-0001").join("document.pdf")).unwrap();
    fs::write(set.path().join("g-0002").join("document.pdf"), &first).unwrap();
    let label_path = set.path().join("g-0002").join("label.json");
    let mut label: Value = serde_json::from_slice(&fs::read(&label_path).unwrap()).unwrap();
    label["document"]["sha256"] = json!(sha256_hex(&first));
    fs::write(&label_path, label.to_string()).unwrap();
    fs::remove_file(set.path().join("g-0003").join("label.json")).unwrap();
    let report = set.check();
    let order: Vec<&str> = report.problems.iter().filter_map(|p| p.case.as_deref()).collect();
    assert_eq!(order, ["g-0002", "g-0003"]);
}
