// Written by Kone Tshivhinda & Claude
use super::*;
use crate::label::tests::base;
use crate::label::Label;
use clearcut_core::event::Confidence;
use clearcut_core::Money;
use serde_json::json;

// ---- builders ----

/// The truth of the example label (19 fields: 11 critical, 8 non-critical).
fn truth_of(id: &str) -> Extraction {
    let mut value = base();
    value["case"] = json!(id);
    let label = Label::parse(value.to_string().as_bytes()).unwrap();
    label.truth.to_extraction(&label.case, "KT").unwrap()
}

fn done(id: &str, extraction: Extraction) -> RunResult {
    RunResult {
        case: id.to_string(),
        disposition: Disposition::Done,
        extraction: Some(extraction),
        model: None,
        usage: None,
    }
}

/// A case whose result is a perfect copy of the truth, after `change`.
fn case_with(id: &str, split: Split, change: impl FnOnce(&mut RunResult)) -> CaseInput {
    let truth = truth_of(id);
    let mut result = done(id, truth.clone());
    change(&mut result);
    CaseInput {
        id: id.to_string(),
        split,
        tags: Vec::new(),
        expected_findings: Vec::new(),
        truth,
        result: Some(result),
    }
}

fn perfect(id: &str, split: Split) -> CaseInput {
    case_with(id, split, |_| {})
}

fn extraction(result: &mut RunResult) -> &mut Extraction {
    result.extraction.as_mut().expect("an extraction")
}

fn run(cases: &[CaseInput]) -> Report {
    score(cases, &Gate::default(), Vec::new())
}

// ---- a perfect run ----

#[test]
fn a_perfect_run_passes_with_everything_correct() {
    let report = run(&[perfect("g-0001", Split::Test)]);
    assert!(report.gate.passed, "{:?}", report.gate.failures);
    assert!(report.silent_critical_errors.is_empty());
    assert_eq!(report.cases, 1);
    assert_eq!(report.completion, Ratio::new(1, 1));
    assert_eq!(
        report.critical,
        Tally {
            correct: 11,
            ..Tally::default()
        }
    );
    assert_eq!(
        report.non_critical,
        Tally {
            correct: 8,
            ..Tally::default()
        }
    );
    assert_eq!(report.review_rate, Ratio::new(0, 1));
    // Every cited value but the currency is on the right page.
    assert_eq!(report.citation, Ratio::new(18, 18));
    assert_eq!(
        report.validation,
        ValidationAgreement {
            expected: 0,
            reported: 0,
            matched: 0
        }
    );
}

// ---- the silent-error rule ----

#[test]
fn a_wrong_critical_value_nobody_flagged_is_a_silent_critical_error() {
    let case = case_with("g-0001", Split::Test, |r| {
        extraction(r).invoice_number.as_mut().unwrap().value = "INV-9".into()
    });
    let report = run(&[case]);
    assert_eq!(
        report.silent_critical_errors,
        [SilentError {
            case: "g-0001".into(),
            field: Field::InvoiceNumber,
            error: ErrorKind::Wrong
        }]
    );
    assert_eq!(report.critical.wrong_silent, 1);
    assert_eq!(report.critical.correct, 10);
    assert!(!report.gate.passed);
    assert!(report
        .gate
        .failures
        .contains(&GateFailure::SilentCriticalErrors { found: 1, max: 0 }));
}

#[test]
fn ending_in_review_flags_every_field_of_the_case() {
    let case = case_with("g-0001", Split::Test, |r| {
        extraction(r).invoice_number.as_mut().unwrap().value = "INV-9".into();
        r.disposition = Disposition::NeedsReview;
    });
    let report = run(&[case]);
    assert!(report.silent_critical_errors.is_empty());
    assert_eq!(report.critical.wrong_flagged, 1);
    assert_eq!(report.review_rate, Ratio::new(1, 1));
    // The wrong value still counts against accuracy: 10 of 11 is under 97%.
    assert!(!report.gate.passed);
    assert!(!report
        .gate
        .failures
        .iter()
        .any(|f| matches!(f, GateFailure::SilentCriticalErrors { .. })));
    assert!(report
        .gate
        .failures
        .iter()
        .any(|f| matches!(f, GateFailure::CriticalAccuracy { .. })));
}

#[test]
fn low_confidence_flags_a_field_but_only_strictly_below_the_threshold() {
    for (confidence, flagged) in [(799, true), (800, false), (801, false), (0, true), (1000, false)] {
        let case = case_with("g-0001", Split::Test, |r| {
            let number = extraction(r).invoice_number.as_mut().unwrap();
            number.value = "INV-9".into();
            number.confidence = Confidence::new(confidence).unwrap();
        });
        let report = run(&[case]);
        assert_eq!(
            report.critical.wrong_flagged,
            u64::from(flagged),
            "confidence {confidence}"
        );
        assert_eq!(
            report.critical.wrong_silent,
            u64::from(!flagged),
            "confidence {confidence}"
        );
    }
}

#[test]
fn the_review_threshold_is_the_gates_to_set() {
    let case = case_with("g-0001", Split::Test, |r| {
        let number = extraction(r).invoice_number.as_mut().unwrap();
        number.value = "INV-9".into();
        number.confidence = Confidence::new(850).unwrap();
    });
    let strict = Gate {
        review_confidence: 900,
        ..Gate::default()
    };
    assert_eq!(
        score(std::slice::from_ref(&case), &strict, Vec::new())
            .critical
            .wrong_flagged,
        1
    );
    let lax = Gate {
        review_confidence: 800,
        ..Gate::default()
    };
    assert_eq!(score(&[case], &lax, Vec::new()).critical.wrong_silent, 1);
}

#[test]
fn confidence_in_one_field_does_not_flag_another() {
    let case = case_with("g-0001", Split::Test, |r| {
        extraction(r).invoice_number.as_mut().unwrap().confidence = Confidence::new(10).unwrap();
        extraction(r).invoice_date.as_mut().unwrap().value = date_after(extraction(r));
    });
    let report = run(&[case]);
    assert_eq!(
        report.critical.wrong_silent, 1,
        "the date is wrong and its own confidence is fine"
    );
}

fn date_after(extraction: &Extraction) -> time::Date {
    extraction
        .invoice_date
        .as_ref()
        .unwrap()
        .value
        .next_day()
        .unwrap()
}

#[test]
fn a_validator_finding_about_a_field_flags_it() {
    // The printed line total no longer equals quantity x price.
    let case = case_with("g-0001", Split::Test, |r| {
        extraction(r).lines[0].value.line_total = Money::zar(1_499_999)
    });
    let report = run(&[case]);
    assert_eq!(report.critical.wrong_flagged, 1);
    assert_eq!(report.critical.wrong_silent, 0);
    assert!(report.silent_critical_errors.is_empty());
}

#[test]
fn a_consistent_but_wrong_price_is_caught_by_the_totals_cross_check() {
    // Price and line total changed together: the line is self-consistent, but
    // the printed totals no longer add up, and that flags every line amount.
    let case = case_with("g-0001", Split::Test, |r| {
        let line = &mut extraction(r).lines[0].value;
        line.unit_price = Money::zar(140_000);
        line.line_total = Money::zar(1_400_000);
    });
    let report = run(&[case]);
    assert!(
        report.silent_critical_errors.is_empty(),
        "{:?}",
        report.silent_critical_errors
    );
    assert_eq!(report.critical.wrong_flagged, 2, "unit price and line total");
}

#[test]
fn a_wrong_printed_total_is_flagged_by_the_validator() {
    let case = case_with("g-0001", Split::Test, |r| {
        extraction(r).stated_totals.as_mut().unwrap().value.vat = Money::zar(225_001);
    });
    let report = run(&[case]);
    assert_eq!(report.critical.wrong_flagged, 1);
    assert!(report.silent_critical_errors.is_empty());
}

#[test]
fn a_missing_value_the_sa_rules_notice_is_flagged_and_one_they_do_not_is_silent() {
    // The invoice date is required on a tax invoice, so its absence is a finding.
    let case = case_with("g-0001", Split::Test, |r| extraction(r).invoice_date = None);
    let report = run(&[case]);
    assert_eq!(report.critical.missing_flagged, 1);
    assert!(report.silent_critical_errors.is_empty());

    // Dropping the printed totals breaks no rule, so nobody is told.
    let case = case_with("g-0001", Split::Test, |r| extraction(r).stated_totals = None);
    let report = run(&[case]);
    assert_eq!(report.critical.missing_silent, 3);
    let fields: Vec<String> = report
        .silent_critical_errors
        .iter()
        .map(|e| e.field.to_string())
        .collect();
    assert_eq!(fields, ["totals.net", "totals.vat", "totals.gross"]);
    assert!(report
        .silent_critical_errors
        .iter()
        .all(|e| e.error == ErrorKind::Missing));
}

#[test]
fn an_invented_critical_value_nobody_flagged_is_silent_too() {
    let case = case_with("g-0001", Split::Test, |r| {
        extraction(r).credit_note = Some(clearcut_core::extraction::Sourced {
            value: clearcut_core::extraction::CreditNoteRef {
                original_invoice_number: "INV-1".into(),
                amount: Money::zar(100),
            },
            page: std::num::NonZeroU32::new(1).unwrap(),
            locator: None,
            confidence: Confidence::CERTAIN,
        });
    });
    let report = run(&[case]);
    assert_eq!(report.critical.spurious_silent, 1);
    assert_eq!(report.non_critical.spurious_silent, 1);
    assert_eq!(
        report.silent_critical_errors,
        [SilentError {
            case: "g-0001".into(),
            field: Field::CreditNoteAmount,
            error: ErrorKind::Spurious
        }]
    );
}

#[test]
fn a_silent_mistake_in_a_non_critical_field_does_not_trip_the_gate() {
    let case = case_with("g-0001", Split::Test, |r| {
        extraction(r).supplier.as_mut().unwrap().value.address = Some("Somewhere else".into());
        extraction(r).due = None;
    });
    let report = run(&[case]);
    assert!(report.silent_critical_errors.is_empty());
    assert_eq!(report.non_critical.wrong_silent, 1);
    assert_eq!(report.non_critical.missing_silent, 2);
    assert!(!report
        .gate
        .failures
        .iter()
        .any(|f| matches!(f, GateFailure::SilentCriticalErrors { .. })));
}

#[test]
fn silent_errors_are_listed_by_case_then_field() {
    let wrong_number = |r: &mut RunResult| extraction(r).invoice_number.as_mut().unwrap().value = "X".into();
    let wrong_name = |r: &mut RunResult| extraction(r).supplier.as_mut().unwrap().value.name = "Y".into();
    let report = run(&[
        case_with("g-0002", Split::Test, wrong_number),
        case_with("g-0001", Split::Test, |r| {
            wrong_number(r);
            wrong_name(r);
        }),
    ]);
    let listed: Vec<(String, String)> = report
        .silent_critical_errors
        .iter()
        .map(|e| (e.case.clone(), e.field.to_string()))
        .collect();
    assert_eq!(
        listed,
        [
            ("g-0001".into(), "invoice_number".into()),
            ("g-0001".into(), "supplier.name".into()),
            ("g-0002".into(), "invoice_number".into()),
        ]
    );
}

// ---- completion ----

#[test]
fn a_case_without_a_result_is_a_completion_failure_and_all_its_fields_are_missing() {
    let mut missing = perfect("g-0002", Split::Test);
    missing.result = None;
    let report = run(&[perfect("g-0001", Split::Test), missing]);
    assert_eq!(report.cases, 2);
    assert_eq!(report.completion, Ratio::new(1, 2));
    assert!(report
        .gate
        .failures
        .iter()
        .any(|f| matches!(f, GateFailure::Completion { .. })));
    // The absence is loud (the gate fails), so its fields are missing-but-flagged.
    assert_eq!(report.critical.missing_flagged, 11);
    assert_eq!(report.non_critical.missing_flagged, 8);
    assert!(report.silent_critical_errors.is_empty());
    // Review rate, citations and validation only look at cases that finished.
    assert_eq!(report.review_rate, Ratio::new(0, 1));
}

#[test]
fn a_finished_case_with_no_extraction_is_missing_everything_silently() {
    let case = case_with("g-0001", Split::Test, |r| r.extraction = None);
    let report = run(&[case]);
    assert_eq!(report.critical.missing_silent, 11);
    assert_eq!(report.silent_critical_errors.len(), 11);
    assert_eq!(report.completion, Ratio::new(1, 1));
}

#[test]
fn an_empty_selection_fails_the_gate_rather_than_passing_vacuously() {
    let report = run(&[]);
    assert_eq!(report.cases, 0);
    assert!(!report.gate.passed);
    assert_eq!(report.gate.failures, [GateFailure::NoCases]);

    // Cases exist, but none in the selected split.
    let report = run(&[perfect("g-0001", Split::Dev)]);
    assert_eq!(report.cases, 0);
    assert_eq!(report.gate.failures, [GateFailure::NoCases]);
}

// ---- splits ----

#[test]
fn only_the_selected_split_is_scored() {
    let bad = |id| {
        case_with(id, Split::Dev, |r| {
            extraction(r).invoice_number.as_mut().unwrap().value = "X".into()
        })
    };
    let cases = [perfect("g-0001", Split::Test), bad("g-0002")];

    let test = score(
        &cases,
        &Gate {
            split: SplitChoice::Test,
            ..Gate::default()
        },
        Vec::new(),
    );
    assert_eq!(test.cases, 1);
    assert!(test.gate.passed);

    let dev = score(
        &cases,
        &Gate {
            split: SplitChoice::Dev,
            ..Gate::default()
        },
        Vec::new(),
    );
    assert_eq!(dev.cases, 1);
    assert_eq!(dev.silent_critical_errors.len(), 1);
    assert!(!dev.gate.passed);

    let all = score(
        &cases,
        &Gate {
            split: SplitChoice::All,
            ..Gate::default()
        },
        Vec::new(),
    );
    assert_eq!(all.cases, 2);
    assert_eq!(all.silent_critical_errors.len(), 1);
}

#[test]
fn the_default_gate_looks_at_the_test_split() {
    assert_eq!(Gate::default().split, SplitChoice::Test);
}

// ---- accuracy gates ----

fn with_wrong_descriptions(id: &str, wrong: usize) -> CaseInput {
    // Non-critical mistakes: change the description, supplier address, ... as needed.
    case_with(id, Split::Test, |r| {
        let e = extraction(r);
        if wrong >= 1 {
            e.lines[0].value.description = "other".into();
        }
        if wrong >= 2 {
            e.supplier.as_mut().unwrap().value.address = Some("elsewhere".into());
        }
    })
}

#[test]
fn accuracy_gates_compare_exactly_at_the_boundary() {
    // 8 non-critical fields per case. 100 cases would be unwieldy, so ask for
    // a bar that 7 of 8 meets exactly (87.50%) and 6 of 8 does not.
    let gate = |min| Gate {
        min_noncritical_accuracy_bp: min,
        ..Gate::default()
    };
    let one_wrong = [with_wrong_descriptions("g-0001", 1)];
    assert!(score(&one_wrong, &gate(8_750), Vec::new()).gate.passed);
    let failed = score(&one_wrong, &gate(8_751), Vec::new());
    assert!(!failed.gate.passed);
    assert!(failed
        .gate
        .failures
        .iter()
        .any(|f| matches!(f, GateFailure::NonCriticalAccuracy { .. })));

    let two_wrong = [with_wrong_descriptions("g-0001", 2)];
    assert!(score(&two_wrong, &gate(7_500), Vec::new()).gate.passed);
    assert!(!score(&two_wrong, &gate(7_501), Vec::new()).gate.passed);
}

#[test]
fn flagged_errors_still_count_against_accuracy() {
    let case = case_with("g-0001", Split::Test, |r| {
        extraction(r).invoice_number.as_mut().unwrap().value = "X".into();
        r.disposition = Disposition::NeedsReview;
    });
    let report = run(&[case]);
    assert_eq!(report.critical.accuracy(), Ratio::new(10, 11));
}

#[test]
fn the_silent_error_limit_can_be_raised_by_the_gate() {
    let case = case_with("g-0001", Split::Test, |r| {
        extraction(r).invoice_number.as_mut().unwrap().value = "X".into()
    });
    let gate = Gate {
        max_silent_critical_errors: 1,
        min_critical_accuracy_bp: 0,
        ..Gate::default()
    };
    assert!(score(&[case], &gate, Vec::new()).gate.passed);
}

// ---- tags ----

fn tagged(mut case: CaseInput, tags: &[&str]) -> CaseInput {
    case.tags = tags.iter().map(|t| (*t).to_string()).collect();
    case
}

#[test]
fn results_are_broken_down_by_tag() {
    let bad = case_with("g-0002", Split::Test, |r| {
        extraction(r).invoice_number.as_mut().unwrap().value = "X".into()
    });
    let report = run(&[
        tagged(perfect("g-0001", Split::Test), &["text-layer", "multi-page"]),
        tagged(bad, &["scan", "multi-page"]),
    ]);
    let tag = |name: &str| {
        report
            .tags
            .get(name)
            .unwrap_or_else(|| panic!("no tag {name}"))
            .clone()
    };
    assert_eq!(tag("text-layer").cases, 1);
    assert_eq!(
        tag("text-layer").critical,
        Tally {
            correct: 11,
            ..Tally::default()
        }
    );
    assert_eq!(tag("text-layer").silent_critical_errors, 0);
    assert_eq!(tag("scan").cases, 1);
    assert_eq!(tag("scan").critical.wrong_silent, 1);
    assert_eq!(tag("scan").silent_critical_errors, 1);
    assert_eq!(tag("multi-page").cases, 2);
    assert_eq!(tag("multi-page").critical.correct, 21);
    assert_eq!(
        report.tags.keys().cloned().collect::<Vec<_>>(),
        ["multi-page", "scan", "text-layer"]
    );
}

#[test]
fn a_tag_far_below_the_overall_figure_fails_the_gate_once_it_has_enough_cases() {
    // Five "scan" cases each with one wrong critical field (flagged, so only
    // accuracy suffers), fifteen perfect "text-layer" cases.
    let mut cases = Vec::new();
    for n in 0..5 {
        let case = case_with(&format!("g-{:04}", n + 1), Split::Test, |r| {
            extraction(r).invoice_number.as_mut().unwrap().value = "X".into();
            r.disposition = Disposition::NeedsReview;
        });
        cases.push(tagged(case, &["scan"]));
    }
    for n in 5..20 {
        cases.push(tagged(
            perfect(&format!("g-{:04}", n + 1), Split::Test),
            &["text-layer"],
        ));
    }
    let gate = Gate {
        min_critical_accuracy_bp: 0,
        max_tag_shortfall_bp: 500,
        ..Gate::default()
    };
    let report = score(&cases, &gate, Vec::new());
    // Overall 215/220 = 97.72%; scan 50/55 = 90.90%: 6.82 points below.
    let failure = report.gate.failures.iter().find_map(|f| match f {
        GateFailure::TagShortfall {
            tag, shortfall_bp, ..
        } => Some((tag.clone(), *shortfall_bp)),
        _ => None,
    });
    assert_eq!(failure, Some(("scan".to_string(), 682)));

    // A bigger allowance passes.
    let lax = Gate {
        max_tag_shortfall_bp: 700,
        ..gate.clone()
    };
    assert!(score(&cases, &lax, Vec::new()).gate.passed);

    // A tag with too few cases is not judged.
    let few = Gate {
        min_tag_cases: 6,
        ..gate
    };
    assert!(score(&cases, &few, Vec::new()).gate.passed);
}

#[test]
fn a_case_without_a_result_still_counts_toward_its_tags_cases() {
    let mut missing = tagged(perfect("g-0001", Split::Test), &["scan"]);
    missing.result = None;
    let report = run(&[missing]);
    assert_eq!(report.tags["scan"].cases, 1);
    assert_eq!(report.tags["scan"].critical.missing_flagged, 11);
}

// ---- validation agreement, citations, tokens ----

#[test]
fn validation_agreement_compares_findings_with_what_the_label_expects() {
    // The document really prints a wrong VAT; the system reads it faithfully,
    // so the validator reports exactly what the label expects.
    let mut faithful = case_with("g-0001", Split::Test, |r| {
        extraction(r).stated_totals.as_mut().unwrap().value.vat = Money::zar(200_000);
    });
    faithful.truth.stated_totals.as_mut().unwrap().value.vat = Money::zar(200_000);
    faithful.expected_findings = vec!["totals_mismatch:vat".into()];

    // A clean document where the system makes up a mistake (a wrong VAT) that
    // the label does not expect; and the label expects one that never happens.
    let mut noisy = case_with("g-0002", Split::Test, |r| {
        extraction(r).stated_totals.as_mut().unwrap().value.vat = Money::zar(1);
    });
    noisy.expected_findings = vec!["due_before_issue".into()];

    let report = run(&[faithful, noisy]);
    assert_eq!(
        report.validation,
        ValidationAgreement {
            expected: 2,
            reported: 2,
            matched: 1
        }
    );
    assert_eq!(report.validation.precision(), Ratio::new(1, 2));
    assert_eq!(report.validation.recall(), Ratio::new(1, 2));
}

#[test]
fn validation_agreement_ignores_cases_that_did_not_finish() {
    let mut missing = perfect("g-0001", Split::Test);
    missing.expected_findings = vec!["no_lines".into()];
    missing.result = None;
    let report = run(&[missing]);
    assert_eq!(report.validation, ValidationAgreement::default());
    assert_eq!(report.validation.precision().basis_points(), None);
}

#[test]
fn citation_accuracy_counts_correct_values_on_the_right_page() {
    let case = case_with("g-0001", Split::Test, |r| {
        extraction(r).invoice_number.as_mut().unwrap().page = std::num::NonZeroU32::new(2).unwrap();
    });
    let report = run(&[case]);
    assert_eq!(report.citation, Ratio::new(17, 18));
}

#[test]
fn tokens_are_summed_over_cases_that_report_them() {
    let with_usage = |id: &str, input, output| {
        case_with(id, Split::Test, move |r| {
            r.usage = Some(TokenUsage {
                input_tokens: input,
                output_tokens: output,
            })
        })
    };
    let report = run(&[
        with_usage("g-0001", 1_000, 200),
        with_usage("g-0002", 500, 50),
        perfect("g-0003", Split::Test),
    ]);
    assert_eq!(
        report.tokens,
        TokenUsage {
            input_tokens: 1_500,
            output_tokens: 250
        }
    );
    assert_eq!(report.cases_with_usage, 2);
}

#[test]
fn problems_found_while_reading_results_are_carried_into_the_report() {
    let problems = vec![
        ResultProblem::Unreadable {
            case: "g-0001".into(),
        },
        ResultProblem::OrphanResults { count: 2 },
    ];
    let report = score(
        &[perfect("g-0001", Split::Test)],
        &Gate::default(),
        problems.clone(),
    );
    assert_eq!(report.result_problems, problems);
}

// ---- the rendered report ----

fn golden_report() -> Report {
    let bad = case_with("g-0002", Split::Test, |r| {
        extraction(r).invoice_number.as_mut().unwrap().value = "X".into()
    });
    let mut missing = perfect("g-0003", Split::Test);
    missing.result = None;
    let cases = [
        tagged(perfect("g-0001", Split::Test), &["text-layer"]),
        tagged(bad, &["scan"]),
        missing,
    ];
    score(
        &cases,
        &Gate::default(),
        vec![ResultProblem::OrphanResults { count: 1 }],
    )
}

#[test]
fn the_markdown_report_has_a_fixed_layout() {
    let expected = r#"# Gold-set score: test split, 3 cases

**Gate: FAILED**
- Silent critical errors: 1 found, at most 0 allowed.
- Completion: 66.66% (2/3), at least 100.00% required.
- Critical-field accuracy: 63.63% (21/33), at least 97.00% required.
- Non-critical accuracy: 66.66% (16/24), at least 95.00% required.

## Silent critical errors

| Case | Field | Error |
|---|---|---|
| g-0002 | invoice_number | wrong |

## Accuracy before review

| Fields | Total | Correct | Wrong (flagged / silent) | Missing (flagged / silent) | Spurious (flagged / silent) | Accuracy |
|---|---|---|---|---|---|---|
| Critical | 33 | 21 | 0 / 1 | 11 / 0 | 0 / 0 | 63.63% (21/33) |
| Non-critical | 24 | 16 | 0 / 0 | 8 / 0 | 0 / 0 | 66.66% (16/24) |

## Other measures

- Completion: 66.66% (2/3)
- Review rate (cases ending in review): 0.00% (0/2)
- Validation findings: precision n/a (0/0), recall n/a (0/0)
- Citation accuracy: 100.00% (35/35)
- Tokens: 0 in, 0 out (0 of 3 cases reported usage)
- Review threshold: confidence below 800 is flagged

## By tag

| Tag | Cases | Critical accuracy | Silent critical errors |
|---|---|---|---|
| scan | 1 | 90.90% (10/11) | 1 |
| text-layer | 1 | 100.00% (11/11) | 0 |

## Notes on result files

- 1 file(s) in the results folder belong to no case in the gold set
"#;
    assert_eq!(golden_report().render_markdown(), expected);
}

#[test]
fn a_passing_report_and_a_single_case_read_naturally() {
    let text = run(&[perfect("g-0001", Split::Test)]).render_markdown();
    assert!(
        text.starts_with(
            "# Gold-set score: test split, 1 case

**Gate: PASSED**

## Silent critical errors

None.
"
        ),
        "{text}"
    );
    assert!(text.contains("No tags."));
    assert!(!text.contains("Notes on result files"));
}

#[test]
fn the_report_as_json_has_the_same_numbers() {
    let json = serde_json::to_value(golden_report()).unwrap();
    assert_eq!(json["cases"], 3);
    assert_eq!(json["completion"], json!({ "num": 2, "den": 3 }));
    assert_eq!(
        json["silent_critical_errors"][0],
        json!({ "case": "g-0002", "field": "invoice_number", "error": "wrong" })
    );
    assert_eq!(json["gate"]["passed"], false);
    assert_eq!(json["tags"]["scan"]["silent_critical_errors"], 1);
    assert_eq!(json["split"], "test");
}

#[test]
fn nothing_from_the_documents_or_the_results_reaches_a_report() {
    let case = case_with("g-0001", Split::Test, |r| {
        let e = extraction(r);
        e.invoice_number.as_mut().unwrap().value = "SECRET-NUMBER".into();
        e.supplier.as_mut().unwrap().value.name = "Secretname Ltd".into();
        e.supplier.as_mut().unwrap().value.address = Some("1 Secretstreet".into());
        e.lines[0].value.description = "Secretdescription".into();
        e.stated_totals.as_mut().unwrap().value.gross = Money::zar(98_765_432);
        r.model = Some(result::ModelRef {
            name: "secret-model".into(),
            prompt_version: "secret-v".into(),
        });
    });
    let report = run(&[tagged(case, &["fine-tag"])]);
    let text = format!(
        "{}
{}
{:?}",
        report.render_markdown(),
        serde_json::to_string(&report).unwrap(),
        report
    );
    for secret in ["SECRET", "Secret", "secret", "987654", "9876543"] {
        assert!(
            !text.contains(secret),
            "{secret} leaked:
{text}"
        );
    }
}

#[test]
fn tag_breakdowns_include_non_critical_fields_too() {
    let bad = case_with("g-0002", Split::Test, |r| {
        extraction(r).lines[0].value.description = "something else".into();
    });
    let report = run(&[
        tagged(perfect("g-0001", Split::Test), &["text-layer"]),
        tagged(bad, &["scan"]),
    ]);
    assert_eq!(
        report.tags["scan"].non_critical,
        Tally {
            correct: 7,
            wrong_silent: 1,
            ..Tally::default()
        }
    );
    assert_eq!(
        report.tags["text-layer"].non_critical,
        Tally {
            correct: 8,
            ..Tally::default()
        }
    );
    assert_eq!(
        report.tags["scan"].critical,
        Tally {
            correct: 11,
            ..Tally::default()
        }
    );
}

#[test]
fn a_tag_exactly_at_the_allowed_shortfall_passes() {
    // Same shape as the tag test above: the shortfall is 6.82 points (682 bp).
    let mut cases = Vec::new();
    for n in 0..5 {
        let case = case_with(&format!("g-{:04}", n + 1), Split::Test, |r| {
            extraction(r).invoice_number.as_mut().unwrap().value = "X".into();
            r.disposition = Disposition::NeedsReview;
        });
        cases.push(tagged(case, &["scan"]));
    }
    for n in 5..20 {
        cases.push(tagged(
            perfect(&format!("g-{:04}", n + 1), Split::Test),
            &["text-layer"],
        ));
    }
    let gate = |max| Gate {
        min_critical_accuracy_bp: 0,
        max_tag_shortfall_bp: max,
        ..Gate::default()
    };
    assert!(score(&cases, &gate(682), Vec::new()).gate.passed);
    assert!(!score(&cases, &gate(681), Vec::new()).gate.passed);
}

#[test]
fn precision_and_recall_are_not_the_same_thing() {
    // One finding expected; the system reports two (the expected one and an extra).
    let mut case = case_with("g-0001", Split::Test, |r| {
        let totals = &mut extraction(r).stated_totals.as_mut().unwrap().value;
        totals.vat = Money::zar(200_000);
        totals.gross = Money::zar(1);
    });
    case.truth.stated_totals.as_mut().unwrap().value.vat = Money::zar(200_000);
    case.expected_findings = vec!["totals_mismatch:vat".into()];
    let report = run(&[case]);
    assert_eq!(
        report.validation,
        ValidationAgreement {
            expected: 1,
            reported: 2,
            matched: 1
        }
    );
    assert_eq!(report.validation.precision(), Ratio::new(1, 2));
    assert_eq!(report.validation.recall(), Ratio::new(1, 1));
}
