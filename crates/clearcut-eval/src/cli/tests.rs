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
