// Written by Kone Tshivhinda & Claude
use super::*;
use crate::label::Label;
use crate::sample::write_sample;
use crate::score::{Disposition, Ratio, SplitChoice};
use clearcut_core::event::Extractor;
use std::fs;
use tempfile::TempDir;

/// A synthetic gold set and a results folder holding a perfect result for
/// every case.
struct Run {
    dir: TempDir,
}

impl Run {
    fn new() -> Run {
        let dir = tempfile::tempdir().unwrap();
        write_sample(&dir.path().join("gold")).unwrap();
        fs::create_dir(dir.path().join("results")).unwrap();
        let run = Run { dir };
        for id in run.case_ids() {
            run.write_result(&id, |_| {});
        }
        run
    }

    fn gold(&self) -> std::path::PathBuf {
        self.dir.path().join("gold")
    }

    fn results(&self) -> std::path::PathBuf {
        self.dir.path().join("results")
    }

    fn case_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = fs::read_dir(self.gold())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        ids.sort();
        ids
    }

    /// Writes a perfect result for `id`, after `change`.
    fn write_result(&self, id: &str, change: impl FnOnce(&mut RunResult)) {
        let label = Label::parse(&fs::read(self.gold().join(id).join("label.json")).unwrap()).unwrap();
        let mut extraction = label.truth.to_extraction(&label.case, "KT").unwrap();
        extraction.extractor = Extractor::Model {
            name: "m".into(),
            prompt_version: "p".into(),
        };
        let mut result = RunResult {
            case: id.to_string(),
            disposition: Disposition::Done,
            extraction: Some(extraction),
            model: None,
            usage: None,
        };
        change(&mut result);
        fs::write(
            self.results().join(format!("{id}.json")),
            serde_json::to_vec_pretty(&result).unwrap(),
        )
        .unwrap();
    }

    fn score(&self, split: SplitChoice) -> Result<Report, ScoreError> {
        score_folders(
            &self.gold(),
            &self.results(),
            &Gate {
                split,
                ..Gate::default()
            },
        )
    }
}

#[test]
fn a_perfect_run_over_the_sample_set_passes_on_every_split() {
    let run = Run::new();
    for (split, cases) in [
        (SplitChoice::Test, 5),
        (SplitChoice::Dev, 10),
        (SplitChoice::All, 15),
    ] {
        let report = run.score(split).unwrap();
        assert_eq!(report.cases, cases, "{split}");
        assert!(report.passed(), "{split}: {:?}", report.gate.failures);
        assert!(report.silent_critical_errors.is_empty());
        assert!(report.result_problems.is_empty(), "{:?}", report.result_problems);
    }
}

#[test]
fn validation_agreement_on_the_sample_set_is_perfect_for_a_faithful_run() {
    let report = Run::new().score(SplitChoice::All).unwrap();
    // Five sample cases plant a mistake the validator should report.
    assert_eq!(report.validation.expected, report.validation.matched);
    assert_eq!(report.validation.reported, report.validation.matched);
    assert!(report.validation.expected >= 6);
}

#[test]
fn tags_come_from_the_labels() {
    let report = Run::new().score(SplitChoice::All).unwrap();
    assert_eq!(report.tags["multi-page"].cases, 2);
    assert_eq!(report.tags["credit-note"].cases, 1);
    assert_eq!(report.tags["text-layer"].cases, 15);
}

#[test]
fn a_planted_error_in_a_result_is_found_in_the_right_case() {
    let run = Run::new();
    run.write_result("g-0012", |r| {
        r.extraction
            .as_mut()
            .unwrap()
            .invoice_number
            .as_mut()
            .unwrap()
            .value = "WRONG".into();
    });
    let report = run.score(SplitChoice::Test).unwrap();
    assert!(!report.passed());
    assert_eq!(report.silent_critical_errors.len(), 1);
    assert_eq!(report.silent_critical_errors[0].case, "g-0012");
    // The same error in a dev case does not matter to a test-split score.
    let run = Run::new();
    run.write_result("g-0002", |r| {
        r.extraction
            .as_mut()
            .unwrap()
            .invoice_number
            .as_mut()
            .unwrap()
            .value = "WRONG".into();
    });
    assert!(run.score(SplitChoice::Test).unwrap().passed());
    assert!(!run.score(SplitChoice::Dev).unwrap().passed());
}

#[test]
fn a_case_with_no_result_file_counts_against_completion_without_a_note() {
    let run = Run::new();
    fs::remove_file(run.results().join("g-0011.json")).unwrap();
    let report = run.score(SplitChoice::Test).unwrap();
    assert_eq!(report.completion, Ratio::new(4, 5));
    assert!(report.result_problems.is_empty());
    assert!(!report.passed());
}

#[test]
fn results_that_cannot_be_read_or_understood_are_noted_and_count_as_missing() {
    let run = Run::new();
    // A folder where a file should be; garbage; the wrong shape; another case's result.
    fs::remove_file(run.results().join("g-0011.json")).unwrap();
    fs::create_dir(run.results().join("g-0011.json")).unwrap();
    fs::write(run.results().join("g-0012.json"), "{ nope").unwrap();
    fs::write(run.results().join("g-0013.json"), r#"{ "case": "g-0013" }"#).unwrap();
    fs::copy(
        run.results().join("g-0015.json"),
        run.results().join("g-0014.json"),
    )
    .unwrap();
    let report = run.score(SplitChoice::Test).unwrap();
    assert_eq!(report.completion, Ratio::new(1, 5));
    assert_eq!(report.result_problems.len(), 4);
    let text: Vec<String> = report.result_problems.iter().map(ToString::to_string).collect();
    assert_eq!(text[0], "g-0011: the result file cannot be read");
    assert!(
        text[1].starts_with("g-0012: the label is not valid JSON") || text[1].starts_with("g-0012: "),
        "{text:?}"
    );
    assert!(text[2].starts_with("g-0013: "), "{text:?}");
    assert_eq!(
        text[3],
        "g-0014: the case named inside the result file is a different one"
    );
}

#[test]
fn result_files_that_belong_to_no_case_are_counted() {
    let run = Run::new();
    fs::write(run.results().join("g-9999.json"), "{}").unwrap();
    fs::write(run.results().join("notes.txt"), "x").unwrap();
    fs::create_dir(run.results().join("scratch")).unwrap();
    let report = run.score(SplitChoice::Test).unwrap();
    assert_eq!(
        report.result_problems,
        [ResultProblem::OrphanResults { count: 3 }]
    );
    // Results for cases outside the selected split are not orphans.
    let clean = Run::new().score(SplitChoice::Test).unwrap();
    assert!(clean.result_problems.is_empty());
}

#[test]
fn an_enormous_result_file_is_refused_unread() {
    let run = Run::new();
    fs::write(run.results().join("g-0011.json"), vec![b' '; 9 * 1024 * 1024]).unwrap();
    let report = run.score(SplitChoice::Test).unwrap();
    assert_eq!(
        report.result_problems,
        [ResultProblem::Unreadable {
            case: "g-0011".into()
        }]
    );
    assert_eq!(report.completion, Ratio::new(4, 5));
}

#[test]
fn a_result_file_of_exactly_the_limit_is_read() {
    let run = Run::new();
    let path = run.results().join("g-0011.json");
    let mut text = fs::read_to_string(&path).unwrap();
    text.push_str(&" ".repeat(MAX_RESULT_BYTES as usize - text.len()));
    fs::write(&path, text).unwrap();
    let report = run.score(SplitChoice::Test).unwrap();
    assert!(report.result_problems.is_empty(), "{:?}", report.result_problems);
    assert_eq!(report.completion, Ratio::new(5, 5));
}

#[cfg(unix)]
#[test]
fn a_link_in_place_of_a_result_is_refused() {
    let run = Run::new();
    let target = run.dir.path().join("elsewhere.json");
    fs::rename(run.results().join("g-0011.json"), &target).unwrap();
    std::os::unix::fs::symlink(&target, run.results().join("g-0011.json")).unwrap();
    let report = run.score(SplitChoice::Test).unwrap();
    assert_eq!(
        report.result_problems,
        [ResultProblem::Unreadable {
            case: "g-0011".into()
        }]
    );
}

#[test]
fn a_gold_set_with_problems_is_not_scored() {
    let run = Run::new();
    fs::write(run.gold().join("g-0001").join("document.pdf"), b"tampered").unwrap();
    let error = run.score(SplitChoice::All).unwrap_err();
    assert_eq!(error, ScoreError::GoldSetHasProblems { count: 1 });
    assert!(error.to_string().contains("clearcut-gold check"), "{error}");
}

#[test]
fn a_folder_that_cannot_be_read_is_an_error() {
    let run = Run::new();
    let missing = run.dir.path().join("nope");
    assert!(matches!(
        score_folders(&missing, &run.results(), &Gate::default()),
        Err(ScoreError::Unreadable(_))
    ));
    assert!(matches!(
        score_folders(&run.gold(), &missing, &Gate::default()),
        Err(ScoreError::Unreadable(_))
    ));
}

#[test]
fn the_error_text_does_not_include_the_path() {
    let run = Run::new();
    let missing = run.dir.path().join("secretfolder");
    let error = score_folders(&run.gold(), &missing, &Gate::default()).unwrap_err();
    assert!(!error.to_string().contains("secretfolder"), "{error}");
}

#[test]
fn a_broken_result_for_a_case_outside_the_selected_split_is_not_even_looked_at() {
    let run = Run::new();
    fs::write(run.results().join("g-0002.json"), "{ nope").unwrap(); // a dev case
    let report = run.score(SplitChoice::Test).unwrap();
    assert!(report.result_problems.is_empty(), "{:?}", report.result_problems);
    assert!(run.score(SplitChoice::Dev).unwrap().result_problems.len() == 1);
}
