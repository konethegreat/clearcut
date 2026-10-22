// Written by Kone Tshivhinda & Claude
use super::*;
use std::fs;

/// Runs the command line and returns (exit code, standard output, standard error).
fn run_args(args: &[&str]) -> (u8, String, String) {
    let args: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run(&args, &mut out, &mut err);
    (
        code,
        String::from_utf8(out).unwrap(),
        String::from_utf8(err).unwrap(),
    )
}

fn sample_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    crate::sample::write_sample(&dir.path().join("set")).unwrap();
    dir
}

#[test]
fn no_arguments_prints_the_usage_and_fails() {
    let (code, out, err) = run_args(&[]);
    assert_eq!(code, EXIT_USAGE);
    assert!(out.is_empty());
    assert!(
        err.contains("Usage:") && err.contains("check") && err.contains("sample"),
        "{err}"
    );
}

#[test]
fn help_prints_the_usage_and_succeeds() {
    for flag in ["--help", "-h", "help"] {
        let (code, out, err) = run_args(&[flag]);
        assert_eq!(code, EXIT_OK, "{flag}");
        assert!(out.contains("Usage:"), "{flag}");
        assert!(err.is_empty());
    }
}

#[test]
fn an_unknown_command_or_the_wrong_number_of_arguments_is_a_usage_error() {
    for args in [
        &["frobnicate"][..],
        &["check"],
        &["check", "a", "b"],
        &["sample"],
        &["sample", "a", "b"],
    ] {
        let (code, out, err) = run_args(args);
        assert_eq!(code, EXIT_USAGE, "{args:?}");
        assert!(out.is_empty());
        assert!(err.contains("Usage:"), "{err}");
    }
}

#[test]
fn checking_a_good_set_succeeds_and_shows_the_coverage() {
    let dir = sample_dir();
    let set = dir.path().join("set");
    let (code, out, err) = run_args(&["check", set.to_str().unwrap()]);
    assert_eq!(code, EXIT_OK, "{out}{err}");
    assert!(out.contains("15 cases"), "{out}");
    assert!(out.contains("no problems"), "{out}");
    assert!(err.is_empty());
}

#[test]
fn checking_a_bad_set_fails_and_lists_the_problems() {
    let dir = sample_dir();
    let set = dir.path().join("set");
    fs::write(set.join("g-0003").join("document.pdf"), b"tampered").unwrap();
    let (code, out, _) = run_args(&["check", set.to_str().unwrap()]);
    assert_eq!(code, EXIT_PROBLEMS);
    assert!(
        out.contains("g-0003: the document does not match the sha256"),
        "{out}"
    );
}

#[test]
fn a_folder_that_cannot_be_read_is_reported_on_standard_error() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nope");
    let (code, out, err) = run_args(&["check", missing.to_str().unwrap()]);
    assert_eq!(code, EXIT_UNUSABLE);
    assert!(out.is_empty());
    assert!(err.contains("cannot be read"), "{err}");
    // The path the user typed is not echoed back (it could be a client's name).
    assert!(!err.contains("nope"), "{err}");
}

#[test]
fn writing_the_sample_set_works_once_and_refuses_to_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("new");
    let (code, out, err) = run_args(&["sample", target.to_str().unwrap()]);
    assert_eq!(code, EXIT_OK, "{err}");
    assert!(out.contains("15"), "{out}");
    assert!(target.join("g-0001").join("label.json").is_file());

    let (code, _, err) = run_args(&["sample", target.to_str().unwrap()]);
    assert_eq!(code, EXIT_UNUSABLE);
    assert!(err.contains("not empty"), "{err}");
}

// ---- score and sample-results ----

/// A temporary folder holding a sample gold set (`set`) and sample results (`res`).
fn scored_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    crate::sample::write_sample(&dir.path().join("set")).unwrap();
    crate::sample::results::write_sample_results(&dir.path().join("res")).unwrap();
    dir
}

fn score_args<'a>(dir: &'a tempfile::TempDir, extra: &[&'a str]) -> (u8, String, String) {
    let set = dir.path().join("set");
    let res = dir.path().join("res");
    let (set, res) = (
        set.to_str().unwrap().to_string(),
        res.to_str().unwrap().to_string(),
    );
    let mut args: Vec<&str> = vec!["score", &set, &res];
    args.extend_from_slice(extra);
    run_args(&args)
}

#[test]
fn scoring_the_test_split_of_the_sample_passes() {
    let dir = scored_dir();
    let (code, out, err) = score_args(&dir, &[]);
    assert_eq!(code, EXIT_OK, "{out}{err}");
    assert!(out.starts_with("# Gold-set score: test split, 5 cases"), "{out}");
    assert!(out.contains("**Gate: PASSED**"), "{out}");
    assert!(err.is_empty());
}

#[test]
fn scoring_the_dev_split_of_the_sample_fails_on_its_two_silent_errors() {
    let dir = scored_dir();
    let (code, out, _) = score_args(&dir, &["--split", "dev"]);
    assert_eq!(code, EXIT_PROBLEMS);
    assert!(out.contains("**Gate: FAILED**"), "{out}");
    assert!(out.contains("| g-0002 | supplier.vat_number | wrong |"), "{out}");
    assert!(out.contains("| g-0008 | credit_note.amount | missing |"), "{out}");
    assert!(
        out.contains("Silent critical errors: 2 found, at most 0 allowed."),
        "{out}"
    );
}

#[test]
fn scoring_everything_covers_all_fifteen_cases() {
    let dir = scored_dir();
    let (code, out, _) = score_args(&dir, &["--split", "all"]);
    assert_eq!(code, EXIT_PROBLEMS);
    assert!(out.starts_with("# Gold-set score: all split, 15 cases"), "{out}");
}

#[test]
fn a_gate_file_changes_the_bar_and_the_split_flag_overrides_it() {
    let dir = scored_dir();
    let gate = dir.path().join("gate.json");
    fs::write(
        &gate,
        r#"{ "split": "dev", "max_silent_critical_errors": 2, "min_critical_accuracy_bp": 0, "min_noncritical_accuracy_bp": 0 }"#,
    )
    .unwrap();
    let gate = gate.to_str().unwrap();

    let (code, out, _) = score_args(&dir, &["--gate", gate]);
    assert_eq!(code, EXIT_OK, "{out}");
    assert!(out.contains("dev split, 10 cases"), "{out}");

    let (code, out, _) = score_args(&dir, &["--gate", gate, "--split", "test"]);
    assert_eq!(code, EXIT_OK);
    assert!(out.contains("test split, 5 cases"), "{out}");
}

#[test]
fn a_bad_gate_file_is_refused_without_echoing_it() {
    let dir = scored_dir();
    let gate = dir.path().join("gate.json");
    fs::write(&gate, r#"{ "secretsetting": 1 }"#).unwrap();
    let (code, out, err) = score_args(&dir, &["--gate", gate.to_str().unwrap()]);
    assert_eq!(code, EXIT_UNUSABLE);
    assert!(out.is_empty());
    assert!(
        err.contains("gate file") && !err.contains("secretsetting"),
        "{err}"
    );

    let (code, _, err) = score_args(
        &dir,
        &["--gate", dir.path().join("missing.json").to_str().unwrap()],
    );
    assert_eq!(code, EXIT_UNUSABLE);
    assert!(err.contains("cannot be read"), "{err}");
}

#[test]
fn the_report_can_also_be_written_as_json_but_never_over_an_existing_file() {
    let dir = scored_dir();
    let json = dir.path().join("report.json");
    let (code, out, err) = score_args(&dir, &["--json", json.to_str().unwrap()]);
    assert_eq!(code, EXIT_OK, "{err}");
    assert!(out.contains("Gate: PASSED"));
    let report: serde_json::Value = serde_json::from_slice(&fs::read(&json).unwrap()).unwrap();
    assert_eq!(report["cases"], 5);
    assert_eq!(report["gate"]["passed"], true);

    let (code, _, err) = score_args(&dir, &["--json", json.to_str().unwrap()]);
    assert_eq!(code, EXIT_UNUSABLE);
    assert!(
        err.contains("already exists") && err.contains("never overwritten"),
        "{err}"
    );
}

#[test]
fn a_gate_file_of_exactly_64_kib_is_read_and_one_byte_more_is_refused() {
    let dir = scored_dir();
    let gate = dir.path().join("gate.json");
    let limit = 64 * 1024;
    for (length, accepted) in [(limit, true), (limit + 1, false)] {
        let mut text = String::from("{}");
        text.push_str(&" ".repeat(length - text.len()));
        fs::write(&gate, text).unwrap();
        let (code, _, err) = score_args(&dir, &["--gate", gate.to_str().unwrap()]);
        assert_eq!(code == EXIT_OK, accepted, "{length}: {err}");
        if !accepted {
            assert!(err.contains("too large"), "{err}");
        }
    }
}

#[test]
fn a_gold_set_with_problems_is_not_scored() {
    let dir = scored_dir();
    fs::write(
        dir.path().join("set").join("g-0001").join("document.pdf"),
        b"tampered",
    )
    .unwrap();
    let (code, out, err) = score_args(&dir, &[]);
    assert_eq!(code, EXIT_PROBLEMS);
    assert!(out.is_empty());
    assert!(err.contains("clearcut-gold check"), "{err}");
}

#[test]
fn a_missing_results_folder_is_reported_without_its_path() {
    let dir = scored_dir();
    let set = dir.path().join("set");
    let missing = dir.path().join("secretname");
    let (code, out, err) = run_args(&["score", set.to_str().unwrap(), missing.to_str().unwrap()]);
    assert_eq!(code, EXIT_UNUSABLE);
    assert!(out.is_empty());
    assert!(
        err.contains("cannot be read") && !err.contains("secretname"),
        "{err}"
    );
}

#[test]
fn score_used_wrongly_is_a_usage_error() {
    for args in [
        &["score"][..],
        &["score", "a"],
        &["score", "a", "b", "c"],
        &["score", "a", "b", "--split"],
        &["score", "a", "b", "--split", "train"],
        &["score", "a", "b", "--gate"],
        &["score", "a", "b", "--json"],
        &["score", "a", "b", "--verbose"],
        &["score", "a", "--verbose"],
        &["score", "a", "b", "--split", "dev", "--split", "test"],
        &["score", "a", "b", "--gate", "x", "--gate", "y"],
    ] {
        let (code, out, err) = run_args(args);
        assert_eq!(code, EXIT_USAGE, "{args:?}");
        assert!(out.is_empty());
        assert!(err.contains("Usage:"), "{err}");
    }
}

#[test]
fn the_usage_mentions_every_command() {
    let (_, out, _) = run_args(&["--help"]);
    for word in [
        "check",
        "sample",
        "sample-results",
        "score",
        "--split",
        "--gate",
        "--json",
    ] {
        assert!(out.contains(word), "{word}");
    }
}

#[test]
fn sample_results_are_written_once_and_never_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("res");
    let (code, out, err) = run_args(&["sample-results", target.to_str().unwrap()]);
    assert_eq!(code, EXIT_OK, "{err}");
    assert!(out.contains("15"), "{out}");
    assert!(target.join("g-0001.json").is_file());
    let (code, _, err) = run_args(&["sample-results", target.to_str().unwrap()]);
    assert_eq!(code, EXIT_UNUSABLE);
    assert!(err.contains("not empty"), "{err}");
    let (code, _, err) = run_args(&["sample-results"]);
    assert_eq!(code, EXIT_USAGE);
    assert!(err.contains("Usage:"));
}

// ---- guard ----

fn small_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("gold-guard.json"),
        r#"{ "approved_folders": ["fixtures"] }"#,
    )
    .unwrap();
    fs::write(dir.path().join(".gitignore"), "/gold/\n/gold-*/\n").unwrap();
    fs::write(dir.path().join("README.md"), "hello").unwrap();
    dir
}

#[test]
fn guarding_a_clean_repository_succeeds() {
    let repo = small_repo();
    let (code, out, err) = run_args(&["guard", repo.path().to_str().unwrap()]);
    assert_eq!(code, EXIT_OK, "{out}{err}");
    assert!(
        out.contains("Checked 3 files") && out.contains("no problems"),
        "{out}"
    );
    assert!(err.is_empty());
}

#[test]
fn guarding_a_repository_with_a_document_in_it_fails_and_names_the_path() {
    let repo = small_repo();
    fs::create_dir(repo.path().join("docs")).unwrap();
    fs::write(repo.path().join("docs").join("invoice.pdf"), "x").unwrap();
    fs::create_dir(repo.path().join("fixtures")).unwrap();
    fs::write(repo.path().join("fixtures").join("fine.pdf"), "x").unwrap();
    let (code, out, _) = run_args(&["guard", repo.path().to_str().unwrap()]);
    assert_eq!(code, EXIT_PROBLEMS);
    assert!(out.contains("Found 1 problem(s)"), "{out}");
    assert!(
        out.contains("docs/invoice.pdf: a document file outside the approved fixture folders"),
        "{out}"
    );
    assert!(!out.contains("fixtures/fine.pdf"), "{out}");
}

#[test]
fn guarding_without_the_config_file_is_reported() {
    let repo = small_repo();
    fs::remove_file(repo.path().join("gold-guard.json")).unwrap();
    let (code, out, err) = run_args(&["guard", repo.path().to_str().unwrap()]);
    assert_eq!(code, EXIT_UNUSABLE);
    assert!(out.is_empty());
    assert!(err.contains("gold-guard.json is missing"), "{err}");
}

#[test]
fn this_repository_passes_the_guard_through_the_command() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..");
    let (code, out, err) = run_args(&["guard", root.to_str().unwrap()]);
    assert_eq!(code, EXIT_OK, "{out}{err}");
}

// ---- new-case ----

fn new_case_args<'a>(gold: &'a str, document: &'a str, extra: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec!["new-case", gold, document];
    args.extend_from_slice(extra);
    args
}

const REQUIRED: [&str; 8] = [
    "--split",
    "dev",
    "--tier",
    "consented",
    "--labelled-by",
    "KT",
    "--pages",
    "2",
];

#[test]
fn new_case_creates_the_case_and_says_what_to_do_next() {
    let dir = tempfile::tempdir().unwrap();
    let document = dir.path().join("invoice.pdf");
    fs::write(&document, b"%PDF-1.4 hello").unwrap();
    let gold = dir.path().join("gold");
    let mut extra = REQUIRED.to_vec();
    extra.extend(["--consent-ref", "C-017", "--retain-until", "2031-03-31"]);
    let (code, out, err) = run_args(&new_case_args(
        gold.to_str().unwrap(),
        document.to_str().unwrap(),
        &extra,
    ));
    assert_eq!(code, EXIT_OK, "{err}");
    assert!(
        out.contains("Created g-0001") && out.contains("clearcut-gold check"),
        "{out}"
    );
    assert!(gold.join("g-0001").join("document.pdf").is_file());
    assert!(gold.join("g-0001").join("label.json").is_file());
    // A second one continues the numbering.
    let (code, out, _) = run_args(&new_case_args(
        gold.to_str().unwrap(),
        document.to_str().unwrap(),
        &extra,
    ));
    assert_eq!(code, EXIT_OK);
    assert!(out.contains("Created g-0002"), "{out}");
}

#[test]
fn new_case_used_wrongly_is_a_usage_error() {
    let g = "gold";
    let d = "doc.pdf";
    let without = |flag: &str| -> Vec<&str> {
        let mut kept = Vec::new();
        let mut pairs = REQUIRED.chunks(2);
        for pair in &mut pairs {
            if pair[0] != flag {
                kept.extend_from_slice(pair);
            }
        }
        kept
    };
    for flag in ["--split", "--tier", "--labelled-by", "--pages"] {
        let (code, out, err) = run_args(&new_case_args(g, d, &without(flag)));
        assert_eq!(code, EXIT_USAGE, "missing {flag}");
        assert!(out.is_empty() && err.contains("Usage:"), "{err}");
    }
    for extra in [
        vec![
            "--split",
            "train",
            "--tier",
            "consented",
            "--labelled-by",
            "KT",
            "--pages",
            "2",
        ],
        vec![
            "--split",
            "dev",
            "--tier",
            "scraped",
            "--labelled-by",
            "KT",
            "--pages",
            "2",
        ],
        vec![
            "--split",
            "dev",
            "--tier",
            "consented",
            "--labelled-by",
            "KT",
            "--pages",
            "two",
        ],
        vec![
            "--split",
            "dev",
            "--tier",
            "consented",
            "--labelled-by",
            "KT",
            "--pages",
            "-1",
        ],
        vec![
            "--split",
            "dev",
            "--tier",
            "consented",
            "--labelled-by",
            "KT",
            "--pages",
            "2",
            "--retain-until",
            "soon",
        ],
        vec![
            "--split",
            "dev",
            "--tier",
            "consented",
            "--labelled-by",
            "KT",
            "--pages",
            "2",
            "--retain-until",
            "2031-02-30",
        ],
        vec![
            "--split",
            "dev",
            "--split",
            "test",
            "--tier",
            "consented",
            "--labelled-by",
            "KT",
            "--pages",
            "2",
        ],
        vec![
            "--split",
            "dev",
            "--tier",
            "consented",
            "--labelled-by",
            "KT",
            "--pages",
            "2",
            "--verbose",
        ],
        vec![
            "--split",
            "dev",
            "--tier",
            "consented",
            "--labelled-by",
            "KT",
            "--pages",
        ],
    ] {
        let (code, _, err) = run_args(&new_case_args(g, d, &extra));
        assert_eq!(code, EXIT_USAGE, "{extra:?}");
        assert!(err.contains("Usage:"), "{err}");
    }
    let (code, _, _) = run_args(&["new-case", g]);
    assert_eq!(code, EXIT_USAGE);
}

#[test]
fn new_case_reports_unusable_input_without_echoing_names() {
    let dir = tempfile::tempdir().unwrap();
    let gold = dir.path().join("gold");
    let empty = dir.path().join("SecretClient.pdf");
    fs::write(&empty, b"").unwrap();
    let (code, out, err) = run_args(&new_case_args(
        gold.to_str().unwrap(),
        empty.to_str().unwrap(),
        &REQUIRED,
    ));
    assert_eq!(code, EXIT_UNUSABLE);
    assert!(
        out.is_empty() && err.contains("empty") && !err.contains("Secret"),
        "{err}"
    );

    let missing = dir.path().join("SecretMissing.pdf");
    let (code, _, err) = run_args(&new_case_args(
        gold.to_str().unwrap(),
        missing.to_str().unwrap(),
        &REQUIRED,
    ));
    assert_eq!(code, EXIT_UNUSABLE);
    assert!(err.contains("cannot be used") && !err.contains("Secret"), "{err}");
}

#[test]
fn new_case_with_a_bad_value_is_refused_as_bad_usage_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let document = dir.path().join("a.pdf");
    fs::write(&document, b"x").unwrap();
    let gold = dir.path().join("gold");
    let extra = [
        "--split",
        "dev",
        "--tier",
        "own",
        "--labelled-by",
        "KT",
        "--pages",
        "0",
    ];
    let (code, _, err) = run_args(&new_case_args(
        gold.to_str().unwrap(),
        document.to_str().unwrap(),
        &extra,
    ));
    assert_eq!(code, EXIT_USAGE);
    assert!(err.contains("--pages"), "{err}");
}

#[test]
fn the_usage_lists_guard_and_new_case() {
    let (_, out, _) = run_args(&["--help"]);
    for word in [
        "guard",
        "new-case",
        "--labelled-by",
        "--retain-until",
        "--consent-ref",
        "--tier",
    ] {
        assert!(out.contains(word), "{word}");
    }
}

#[test]
fn new_case_records_the_tier_and_split_it_was_given() {
    let dir = tempfile::tempdir().unwrap();
    let document = dir.path().join("a.pdf");
    fs::write(&document, b"x").unwrap();
    let gold = dir.path().join("gold");
    for (split, tier, expected_split, expected_tier) in [
        ("dev", "own", "dev", "own"),
        ("test", "consented", "test", "consented"),
        ("dev", "synthetic", "dev", "synthetic"),
        ("test", "own", "test", "own"),
    ] {
        let extra = [
            "--split",
            split,
            "--tier",
            tier,
            "--labelled-by",
            "KT",
            "--pages",
            "1",
        ];
        let (code, out, err) = run_args(&new_case_args(
            gold.to_str().unwrap(),
            document.to_str().unwrap(),
            &extra,
        ));
        assert_eq!(code, EXIT_OK, "{out}{err}");
        let id = out
            .split_whitespace()
            .nth(1)
            .unwrap()
            .trim_end_matches('.')
            .to_string();
        let label: serde_json::Value =
            serde_json::from_slice(&fs::read(gold.join(&id).join("label.json")).unwrap()).unwrap();
        assert_eq!(label["split"], expected_split, "{id}");
        assert_eq!(label["provenance"]["tier"], expected_tier, "{id}");
    }
}
