// Written by Kone Tshivhinda & Claude
use super::*;
use crate::compliance_za::Issue;
use crate::event::TotalField;
use crate::extraction::Finding;
use crate::reading::{ProblemKind, ReadingProblem};
use crate::Money;

fn problem(path: &str, kind: ProblemKind) -> ReadingProblem {
    ReadingProblem {
        path: path.to_string(),
        kind,
    }
}

fn zar(cents: i64) -> Money {
    Money::zar(cents)
}

fn line_mismatch() -> Finding {
    Finding::LineTotalMismatch {
        line: 1,
        stated: zar(1_000),
        recomputed: zar(2_000),
    }
}

fn totals_mismatch() -> Finding {
    Finding::TotalsMismatch {
        field: TotalField::Gross,
        stated: zar(1_000),
        recomputed: zar(2_000),
    }
}

fn evidence<'a>(
    problems: &'a [ReadingProblem],
    findings: &'a [Finding],
    text_check: TextCheck,
    whole_document: bool,
) -> ReadingEvidence<'a> {
    ReadingEvidence {
        problems,
        findings,
        text_check,
        whole_document,
    }
}

fn clean(text_check: TextCheck) -> Assessment {
    assess(&evidence(&[], &[], text_check, true))
}

// ---------- accepting ----------

#[test]
fn a_clean_reading_is_accepted() {
    let a = clean(TextCheck::Passed);
    assert_eq!(a.verdict, Verdict::Accept);
    assert_eq!(a.blocking, 0);
    assert_eq!(a.notes, Vec::<Note>::new());
    assert_eq!(a.reasons, Vec::<EscalationReason>::new());
}

#[test]
fn the_confidence_basis_follows_the_text_check() {
    assert_eq!(
        clean(TextCheck::Passed).confidence,
        ConfidenceBasis::TextConfirmed
    );
    assert_eq!(
        clean(TextCheck::NotAvailable).confidence,
        ConfidenceBasis::ArithmeticOnly
    );
    assert_eq!(
        clean(TextCheck::Failed { missing: 2 }).confidence,
        ConfidenceBasis::Unresolved
    );
}

// ---------- problems on values ----------

#[test]
fn a_problem_on_a_critical_value_blocks_and_names_reading_problems() {
    for path in [
        "invoice_number",
        "invoice_date",
        "supplier",
        "supplier/name",
        "supplier/vat_number",
        "totals",
        "totals/net",
        "totals/vat",
        "totals/gross",
        "lines/1/quantity",
        "lines/1/unit_price",
        "lines/1/line_total",
        "lines/2/description",
        "lines/12/tax",
        "lines/3",
        "credit_note",
        "credit_note/amount",
    ] {
        let problems = [problem(path, ProblemKind::BadAmount)];
        let a = assess(&evidence(&problems, &[], TextCheck::Passed, true));
        assert_eq!(
            a.verdict,
            Verdict::Escalate(EscalationReason::ReadingProblems),
            "{path}"
        );
        assert_eq!(a.blocking, 1, "{path}");
    }
}

#[test]
fn a_problem_on_a_value_that_is_not_critical_is_a_note_and_does_not_block() {
    for path in [
        "supplier/address",
        "recipient",
        "recipient/name",
        "recipient/address",
        "recipient/vat_number",
        "due",
        "due/date",
        "due/terms",
        "credit_note/original_invoice_number",
    ] {
        let problems = [problem(path, ProblemKind::TooLong)];
        let a = assess(&evidence(&problems, &[], TextCheck::Passed, true));
        assert_eq!(a.verdict, Verdict::Accept, "{path}");
        assert_eq!(a.blocking, 0, "{path}");
        assert_eq!(
            a.notes,
            vec![Note::NonCriticalProblem {
                path: path.to_string(),
                kind: ProblemKind::TooLong
            }],
            "{path}"
        );
    }
}

#[test]
fn an_unknown_path_is_treated_as_critical_so_nothing_slips_through() {
    let problems = [problem("something/new", ProblemKind::BadAmount)];
    let a = assess(&evidence(&problems, &[], TextCheck::Passed, true));
    assert_eq!(a.verdict, Verdict::Escalate(EscalationReason::ReadingProblems));
}

#[test]
fn a_path_that_only_starts_like_a_critical_one_is_not_confused_with_it() {
    // "supplier_note" and "linesx" are not "supplier" or "lines".
    for path in ["supplier_note", "linesx/1", "totals_extra", "invoice_numbers"] {
        let problems = [problem(path, ProblemKind::BadAmount)];
        let a = assess(&evidence(&problems, &[], TextCheck::Passed, true));
        // Unknown, hence critical: the point is that it is judged as unknown, not as a prefix match
        // of a non-critical path, so a typo in the converter cannot hide a problem.
        assert_eq!(
            a.verdict,
            Verdict::Escalate(EscalationReason::ReadingProblems),
            "{path}"
        );
    }
    // And a non-critical prefix does not swallow a longer unknown path.
    let problems = [problem("recipient_extra", ProblemKind::BadAmount)];
    let a = assess(&evidence(&problems, &[], TextCheck::Passed, true));
    assert_eq!(a.verdict, Verdict::Escalate(EscalationReason::ReadingProblems));
}

#[test]
fn amounts_that_include_vat_go_to_a_person_without_escalating() {
    // A stronger model reads the same printed amounts (ADR-0014 section 6).
    let problems = [problem("amounts_include_vat", ProblemKind::AmountsIncludeVat)];
    let a = assess(&evidence(&problems, &[], TextCheck::Passed, true));
    assert_eq!(a.verdict, Verdict::Review(ReviewReason::AmountsIncludeVat));
}

#[test]
fn a_page_with_too_many_lines_goes_to_a_person_without_escalating() {
    let problems = [problem("lines", ProblemKind::TooManyLines)];
    let a = assess(&evidence(&problems, &[], TextCheck::Passed, true));
    assert_eq!(a.verdict, Verdict::Review(ReviewReason::TooLong));
}

#[test]
fn a_review_reason_wins_over_everything_else_on_the_page() {
    let problems = [
        problem("invoice_number", ProblemKind::BadCharacters),
        problem("amounts_include_vat", ProblemKind::AmountsIncludeVat),
        problem("lines", ProblemKind::TooManyLines),
    ];
    let a = assess(&evidence(
        &problems,
        &[line_mismatch()],
        TextCheck::Failed { missing: 1 },
        true,
    ));
    // Amounts that include VAT are listed first, so they decide.
    assert_eq!(a.verdict, Verdict::Review(ReviewReason::AmountsIncludeVat));
}

// ---------- the arithmetic ----------

#[test]
fn a_line_that_does_not_add_up_escalates_on_every_page() {
    let line_findings = [
        line_mismatch(),
        Finding::NonPositiveQuantity { line: 2 },
        Finding::AmountOutOfRange { line: 3 },
        Finding::CurrencyMismatch { line: 4 },
    ];
    for finding in line_findings {
        for whole_document in [true, false] {
            let findings = [finding.clone()];
            let a = assess(&evidence(&[], &findings, TextCheck::Passed, whole_document));
            assert_eq!(
                a.verdict,
                Verdict::Escalate(EscalationReason::LineArithmetic),
                "{finding:?} {whole_document}"
            );
            assert_eq!(a.blocking, 1);
        }
    }
}

#[test]
fn the_totals_check_applies_only_to_a_one_page_document() {
    for finding in [totals_mismatch(), Finding::TotalsOutOfRange] {
        let findings = [finding.clone()];
        let whole = assess(&evidence(&[], &findings, TextCheck::Passed, true));
        assert_eq!(
            whole.verdict,
            Verdict::Escalate(EscalationReason::TotalsMismatch),
            "{finding:?}"
        );
        assert_eq!(whole.blocking, 1);
        // Page 3 of 5 may hold lines whose total is printed on page 5: not an error here.
        let part = assess(&evidence(&[], &findings, TextCheck::Passed, false));
        assert_eq!(part.verdict, Verdict::Accept, "{finding:?}");
        assert_eq!(part.blocking, 0);
    }
}

#[test]
fn a_page_with_no_lines_is_allowed() {
    // A terms page or a continuation page.
    let a = assess(&evidence(&[], &[Finding::NoLines], TextCheck::Passed, true));
    assert_eq!(a.verdict, Verdict::Accept);
    assert_eq!(a.blocking, 0);
}

#[test]
fn facts_about_the_document_are_notes_and_never_a_reason_to_escalate() {
    let issue = Issue {
        field: "recipient_vat_number",
        message: "a tax invoice over R5 000 needs the recipient's VAT number",
    };
    let findings = [Finding::DueBeforeIssue, Finding::Compliance(issue)];
    let a = assess(&evidence(&[], &findings, TextCheck::Passed, true));
    assert_eq!(a.verdict, Verdict::Accept);
    assert_eq!(
        a.notes,
        vec![
            Note::DueBeforeIssue,
            Note::Compliance {
                field: "recipient_vat_number".to_string()
            }
        ]
    );
}

// ---------- the page's own text ----------

#[test]
fn an_amount_that_is_not_on_the_page_escalates() {
    let a = assess(&evidence(&[], &[], TextCheck::Failed { missing: 2 }, true));
    assert_eq!(a.verdict, Verdict::Escalate(EscalationReason::TextCheckFailed));
    assert_eq!(a.blocking, 1);
    assert_eq!(a.confidence, ConfidenceBasis::Unresolved);
}

#[test]
fn no_usable_text_is_not_a_failure() {
    let a = assess(&evidence(&[], &[], TextCheck::NotAvailable, true));
    assert_eq!(a.verdict, Verdict::Accept);
}

// ---------- which reason is named, and what is counted ----------

#[test]
fn the_first_applicable_reason_is_named_and_all_of_them_are_listed() {
    let problems = [problem("invoice_date", ProblemKind::BadDate)];
    let findings = [line_mismatch(), totals_mismatch()];
    let a = assess(&evidence(
        &problems,
        &findings,
        TextCheck::Failed { missing: 1 },
        true,
    ));
    assert_eq!(a.verdict, Verdict::Escalate(EscalationReason::ReadingProblems));
    assert_eq!(
        a.reasons,
        vec![
            EscalationReason::ReadingProblems,
            EscalationReason::LineArithmetic,
            EscalationReason::TotalsMismatch,
            EscalationReason::TextCheckFailed,
        ]
    );
    // One problem, two blocking findings, one failed text check.
    assert_eq!(a.blocking, 4);

    let a = assess(&evidence(&[], &findings, TextCheck::Passed, true));
    assert_eq!(a.verdict, Verdict::Escalate(EscalationReason::LineArithmetic));
    assert_eq!(
        a.reasons,
        vec![EscalationReason::LineArithmetic, EscalationReason::TotalsMismatch]
    );

    let a = assess(&evidence(
        &[],
        &[totals_mismatch()],
        TextCheck::Failed { missing: 1 },
        true,
    ));
    assert_eq!(a.verdict, Verdict::Escalate(EscalationReason::TotalsMismatch));
}

#[test]
fn every_blocking_problem_and_finding_is_counted_once() {
    let problems = [
        problem("lines/1/unit_price", ProblemKind::BadAmount),
        problem("lines/2/unit_price", ProblemKind::BadAmount),
        problem("recipient/name", ProblemKind::TooLong),
    ];
    let findings = [
        line_mismatch(),
        Finding::NonPositiveQuantity { line: 2 },
        Finding::NoLines,
        Finding::DueBeforeIssue,
    ];
    let a = assess(&evidence(&problems, &findings, TextCheck::Passed, true));
    // Two critical problems and two blocking findings; the recipient problem is a note.
    assert_eq!(a.blocking, 4);
    assert_eq!(a.notes.len(), 2);
}

// ---------- the reasons that go straight to a person, in detail ----------

#[test]
fn a_review_problem_counts_as_blocking() {
    for (problem, reason) in [
        (
            problem("amounts_include_vat", ProblemKind::AmountsIncludeVat),
            ReviewReason::AmountsIncludeVat,
        ),
        (problem("lines", ProblemKind::TooManyLines), ReviewReason::TooLong),
    ] {
        let problems = [problem];
        let a = assess(&evidence(&problems, &[], TextCheck::Passed, true));
        assert_eq!(a.verdict, Verdict::Review(reason));
        assert_eq!(a.blocking, 1, "{reason:?}");
        assert_eq!(a.confidence, ConfidenceBasis::Unresolved);
    }
}

#[test]
fn a_tax_that_is_not_printed_goes_to_a_person_not_to_a_stronger_model() {
    let tax = problem("lines/1/tax", ProblemKind::TaxNotShown);
    let only_tax = std::slice::from_ref(&tax);
    let a = assess(&evidence(only_tax, &[], TextCheck::Passed, true));
    assert_eq!(a.verdict, Verdict::Review(ReviewReason::TaxNotShown));
    assert_eq!(a.blocking, 1);

    // Printed totals that the dropped line leaves short are not a reason to try again.
    let a = assess(&evidence(only_tax, &[totals_mismatch()], TextCheck::Passed, true));
    assert_eq!(a.verdict, Verdict::Review(ReviewReason::TaxNotShown));

    // While something else is wrong that a stronger model could fix, that comes first.
    let bad = problem("lines/2/line_total", ProblemKind::BadAmount);
    let a = assess(&evidence(&[tax, bad], &[], TextCheck::Passed, true));
    assert_eq!(a.verdict, Verdict::Escalate(EscalationReason::ReadingProblems));
}

#[test]
fn amounts_that_include_vat_win_whatever_order_the_problems_come_in() {
    let vat = problem("amounts_include_vat", ProblemKind::AmountsIncludeVat);
    let many = problem("lines", ProblemKind::TooManyLines);
    for problems in [[vat.clone(), many.clone()], [many, vat]] {
        let a = assess(&evidence(&problems, &[], TextCheck::Passed, true));
        assert_eq!(a.verdict, Verdict::Review(ReviewReason::AmountsIncludeVat));
        assert_eq!(a.blocking, 2);
    }
}

#[test]
fn a_page_that_is_too_long_stays_too_long_with_other_problems_around_it() {
    let problems = [
        problem("lines", ProblemKind::TooManyLines),
        problem("totals/net", ProblemKind::BadAmount),
    ];
    let a = assess(&evidence(&problems, &[line_mismatch()], TextCheck::Passed, true));
    assert_eq!(a.verdict, Verdict::Review(ReviewReason::TooLong));
    assert_eq!(a.blocking, 3);
}
