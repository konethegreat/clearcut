// Written by Kone Tshivhinda & Claude
use super::*;
use crate::extraction::fixtures::{at, clean, line, zar};
use crate::extraction::CreditNoteRef;
use crate::TaxCategory;

/// A page that says everything `clean()` says.
const PAGE: &str = "TAX INVOICE\n\
    Invoice No: INV-000001\n\
    Acme (Pty) Ltd   VAT No 4123 456 789\n\
    Client Co\n\
    Consulting 10 R1 500.00 R15 000.00\n\
    Export licence 1 R1 000.00 R1 000.00\n\
    Subtotal R16 000.00\n\
    VAT 15% R2 250.00\n\
    Total R18 250.00\n";

fn check(extraction: &Extraction, text: &str, trust: TextTrust) -> TextCheckReport {
    check_against_text(extraction, text, trust)
}

fn trusted(extraction: &Extraction, text: &str) -> TextCheckReport {
    check(extraction, text, TextTrust::Trusted)
}

// ---------- the normal case ----------

#[test]
fn a_page_that_says_what_was_read_passes() {
    let report = trusted(&clean(), PAGE);
    assert_eq!(report.result, TextCheck::Passed);
    // Invoice number, supplier VAT number, 2 x (price, total), 3 totals.
    assert_eq!(report.checked, 9);
    assert_eq!(report.unconfirmed, Vec::<String>::new());
}

// ---------- how an amount may be printed ----------

fn gross_only(cents: i64) -> Extraction {
    let mut e = clean();
    e.invoice_number = None;
    e.supplier = None;
    e.lines.clear();
    e.stated_totals.as_mut().unwrap().value.gross = zar(cents);
    e.stated_totals.as_mut().unwrap().value.net = zar(0);
    e.stated_totals.as_mut().unwrap().value.vat = zar(0);
    e
}

fn found(text: &str, cents: i64) -> bool {
    let report = trusted(&gross_only(cents), text);
    report.checked == 1 && report.unconfirmed.is_empty()
}

#[test]
fn an_amount_is_found_however_the_thousands_and_cents_are_marked() {
    for printed in [
        "1,234.50",
        "1 234.50",
        "1 234,50",
        "1.234,50",
        "1'234.50",
        "1\u{a0}234,50",
        "1\u{2019}234.50",
        "R1234.50",
        "R 1 234.50",
        "1234.50",
        "1234,50",
        "ZAR 1,234.50 incl",
    ] {
        assert!(found(&format!("Total {printed}"), 123_450), "{printed}");
    }
}

#[test]
fn whole_rand_and_a_single_decimal_are_amounts_too() {
    assert!(found("Total R1500", 150_000));
    assert!(found("Total R1,500", 150_000));
    assert!(found("Total 1 500", 150_000));
    assert!(found("Total 1.500", 150_000));
    assert!(found("Total 12.5", 1_250));
    assert!(found("Total 0.50", 50));
    assert!(found("Total 0,5", 50));
    assert!(found("Total R5", 500));
}

#[test]
fn a_sign_or_brackets_do_not_matter() {
    assert!(found("Credit (R 100.00)", 10_000));
    assert!(found("Credit -100.00", 10_000));
    assert!(found("Credit 100.00", -10_000));
    assert!(found("Credit R100.00 CR", 10_000));
}

#[test]
fn an_amount_that_is_not_there_is_not_found() {
    assert!(!found("Total R1 234.51", 123_450));
    assert!(!found("Total R1 234.50", 123_460));
    assert!(!found("Total R1 235.50", 123_450));
    assert!(!found("Total R 234.50", 123_450));
    assert!(!found("no numbers here", 123_450));
    assert!(!found("", 123_450));
}

#[test]
fn a_bigger_number_does_not_confirm_a_smaller_one_that_it_ends_with() {
    // R115 000.00 is not R15 000.00.
    assert!(!found("Total R115 000.00", 1_500_000));
    assert!(!found("Total R115 000.00", 15_000));
    // R500.00 on its own is R500.00.
    assert!(found("Total R500.00", 50_000));
}

#[test]
fn two_amounts_side_by_side_are_both_found() {
    let text = "Qty 2 150.00 3 000.00";
    assert!(found(text, 15_000));
    assert!(found(text, 300_000));
}

#[test]
fn a_known_limit_neighbouring_numbers_can_read_as_one_amount() {
    // "10 150.00" is also R10 150.00: nothing here can tell the two readings apart, so a model
    // that read R10 150.00 is confirmed. This is the cost of accepting spaces as thousands
    // separators, and is why the check is evidence and not proof.
    assert!(found("Qty 10 150.00", 1_015_000));
}

#[test]
fn a_zero_amount_is_not_checked_because_a_page_may_show_a_dash_or_nothing() {
    let mut e = clean();
    e.stated_totals.as_mut().unwrap().value.vat = zar(0);
    let report = trusted(&e, "Total R18 250.00");
    // The zero VAT is not counted; the rest is: number, VAT number, 4 line amounts, 2 totals.
    assert_eq!(report.checked, 8);
    assert!(!report.unconfirmed.contains(&"totals/vat".to_string()));
}

// ---------- what is checked, and the paths named ----------

#[test]
fn an_invented_amount_is_named_by_its_place_and_never_by_its_value() {
    let mut e = clean();
    e.stated_totals.as_mut().unwrap().value.gross = zar(1_825_001);
    let report = trusted(&e, PAGE);
    assert_eq!(report.result, TextCheck::Failed { missing: 1 });
    assert_eq!(report.unconfirmed, vec!["totals/gross".to_string()]);
    assert_eq!(report.checked, 9);
    let shown = format!("{report:?}");
    assert!(!shown.contains("1825001") && !shown.contains("18250"), "{shown}");
}

#[test]
fn each_kind_of_value_is_checked_and_named_in_a_fixed_order() {
    let mut e = clean();
    e.invoice_number = Some(at("INV-999999".to_string()));
    e.supplier.as_mut().unwrap().value.vat_number = Some("4999999999".to_string());
    e.lines = vec![
        line("Consulting", 10_000, 111_111, 222_222, TaxCategory::Standard),
        line("Export licence", 1_000, 100_000, 333_333, TaxCategory::ZeroRated),
    ];
    e.stated_totals.as_mut().unwrap().value.net = zar(444_444);
    e.stated_totals.as_mut().unwrap().value.vat = zar(555_555);
    e.stated_totals.as_mut().unwrap().value.gross = zar(666_666);
    e.credit_note = Some(at(CreditNoteRef {
        original_invoice_number: "INV-000001".to_string(),
        amount: zar(777_777),
    }));
    let report = trusted(&e, PAGE);
    assert_eq!(
        report.unconfirmed,
        [
            "invoice_number",
            "supplier/vat_number",
            "lines/1/unit_price",
            "lines/1/line_total",
            "lines/2/line_total",
            "totals/net",
            "totals/vat",
            "totals/gross",
            "credit_note/amount",
        ]
    );
    // The second line's unit price R1 000.00 is on the page.
    assert!(!report.unconfirmed.contains(&"lines/2/unit_price".to_string()));
    assert_eq!(report.result, TextCheck::Failed { missing: 9 });
}

#[test]
fn the_same_missing_amount_in_two_places_counts_twice() {
    let mut e = clean();
    e.lines = vec![line("A", 1_000, 123_456, 123_456, TaxCategory::Standard)];
    e.invoice_number = None;
    e.supplier = None;
    e.stated_totals = None;
    let report = trusted(&e, "nothing");
    assert_eq!(report.unconfirmed, ["lines/1/unit_price", "lines/1/line_total"]);
    assert_eq!(report.result, TextCheck::Failed { missing: 2 });
}

#[test]
fn quantities_dates_names_and_descriptions_are_not_checked() {
    let mut e = clean();
    e.invoice_number = None;
    e.supplier = None;
    e.lines.clear();
    e.stated_totals = None;
    e.credit_note = None;
    let report = trusted(&e, "A page that says nothing the extraction says");
    assert_eq!(report.checked, 0);
    assert_eq!(
        report.result,
        TextCheck::NotAvailable,
        "nothing to check is no evidence"
    );
}

// ---------- the invoice number and the VAT number ----------

fn number_only(number: &str) -> Extraction {
    let mut e = clean();
    e.invoice_number = Some(at(number.to_string()));
    e.supplier = None;
    e.lines.clear();
    e.stated_totals = None;
    e
}

fn number_found(printed: &str, read: &str) -> bool {
    let report = trusted(&number_only(read), printed);
    assert_eq!(report.checked, 1);
    report.unconfirmed.is_empty()
}

#[test]
fn an_invoice_number_may_be_spaced_or_cased_differently() {
    assert!(number_found("Invoice No: INV-000001", "INV-000001"));
    assert!(number_found("Invoice No: inv 000001", "INV-000001"));
    assert!(number_found("Invoice No: INV000001", "INV-000001"));
    assert!(number_found("Invoice No: INV/000001 date", "INV-000001"));
    assert!(number_found("2041", "2041"));
    assert!(number_found("Tax invoice 2041\n", "2041"));
}

#[test]
fn an_invoice_number_must_stand_alone_not_sit_inside_a_longer_one() {
    assert!(!number_found("Invoice No: INV-0000011", "INV-000001"));
    assert!(!number_found("Invoice No: XINV-000001", "INV-000001"));
    assert!(!number_found("Ref 12041", "2041"));
    assert!(!number_found("Ref 20415", "2041"));
    assert!(!number_found("Invoice No: INV-000002", "INV-000001"));
}

fn vat_only(number: &str) -> Extraction {
    let mut e = clean();
    e.invoice_number = None;
    e.supplier.as_mut().unwrap().value.vat_number = Some(number.to_string());
    e.lines.clear();
    e.stated_totals = None;
    e
}

fn vat_found(printed: &str, read: &str) -> bool {
    let report = trusted(&vat_only(read), printed);
    assert_eq!(report.checked, 1);
    report.unconfirmed.is_empty()
}

#[test]
fn a_vat_number_may_be_grouped_with_spaces_or_dashes() {
    assert!(vat_found("VAT No 4123456789", "4123456789"));
    assert!(vat_found("VAT No 4123 456 789", "4123456789"));
    assert!(vat_found("VAT: 4123-456-789.", "4123456789"));
    assert!(vat_found("VAT Reg. No. 4123 456 789 Tel", "4123456789"));
}

#[test]
fn a_vat_number_must_not_be_part_of_a_longer_run_of_digits() {
    assert!(!vat_found("Acc 04123456789", "4123456789"));
    assert!(!vat_found("Acc 41234567890", "4123456789"));
    assert!(!vat_found("VAT No 4123 456 780", "4123456789"));
}

#[test]
fn a_blank_number_is_not_checked() {
    let mut e = clean();
    e.invoice_number = Some(at("  ".to_string()));
    e.supplier.as_mut().unwrap().value.vat_number = Some(String::new());
    e.lines.clear();
    e.stated_totals = None;
    let report = trusted(&e, PAGE);
    assert_eq!(report.checked, 0);
}

// ---------- when the text can be believed ----------

#[test]
fn text_that_is_unverified_confirms_but_never_condemns() {
    let mut wrong = clean();
    wrong.stated_totals.as_mut().unwrap().value.gross = zar(1_825_001);
    // All found: two independent readers agree.
    let agree = check(&clean(), PAGE, TextTrust::Unverified);
    assert_eq!(agree.result, TextCheck::Passed);
    // A miss: the text may be the wrong one (a lying layer, a misread scan), so it is
    // reported but is not a failure that sends a correct reading up the ladder.
    let differ = check(&wrong, PAGE, TextTrust::Unverified);
    assert_eq!(differ.result, TextCheck::NotAvailable);
    assert_eq!(differ.unconfirmed, vec!["totals/gross".to_string()]);
    assert_eq!(differ.checked, 9);
}

#[test]
fn trusted_text_that_disagrees_is_a_failure() {
    let mut wrong = clean();
    wrong.stated_totals.as_mut().unwrap().value.gross = zar(1_825_001);
    assert_eq!(trusted(&wrong, PAGE).result, TextCheck::Failed { missing: 1 });
}

#[test]
fn a_page_with_no_usable_text_cannot_be_checked() {
    for text in ["", "   ", "\n\t\n"] {
        for trust in [TextTrust::Trusted, TextTrust::Unverified] {
            let report = check(&clean(), text, trust);
            assert_eq!(report.result, TextCheck::NotAvailable, "{text:?} {trust:?}");
            assert_eq!(report.checked, 0, "no text, so nothing was tried");
            assert!(report.unconfirmed.is_empty());
        }
    }
}

// ---------- robustness ----------

#[test]
fn a_page_of_nothing_but_numbers_is_read_in_bounded_time() {
    // 200 000 groups of digits separated by spaces: every run is cut at 32 groups.
    let text = "1 ".repeat(100_000) + &"12,345.67 ".repeat(10_000);
    let started = std::time::Instant::now();
    let report = trusted(&clean(), &text);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(report.result, TextCheck::Failed { missing: 9 });
}

#[test]
fn very_long_digit_runs_do_not_overflow() {
    let digits = "9".repeat(60);
    let text = format!("Total {digits}.99 and {digits}");
    let report = trusted(&gross_only(150_000), &text);
    assert_eq!(report.result, TextCheck::Failed { missing: 1 });
}

#[test]
fn only_ascii_digits_count() {
    // Arabic-Indic and full-width digits are not amounts here.
    assert!(!found("Total \u{661}\u{662}\u{663}\u{664}.50", 123_450));
    assert!(!found("Total \u{ff11}\u{ff12}\u{ff13}\u{ff14}.50", 123_450));
}

#[test]
fn the_report_holds_paths_and_counts_and_nothing_from_the_page() {
    let mut e = clean();
    e.invoice_number = Some(at("SECRET-INV-77".to_string()));
    let report = trusted(&e, "a page that does not have it");
    let shown = format!("{report:?}");
    assert!(!shown.contains("SECRET"), "{shown}");
    assert!(!shown.contains("a page that"), "{shown}");
}

// ---------- readings that look like amounts and are not ----------

#[test]
fn a_space_is_not_a_decimal_point() {
    // "12 50" is twelve and fifty, not R12.50.
    assert!(!found("Qty 12 50 pcs", 1_250));
    assert!(!found("Ref 1 5", 150));
    // But a point or a comma is.
    assert!(found("Qty 12.50 pcs", 1_250));
    assert!(found("Qty 12,50 pcs", 1_250));
}

#[test]
fn thousands_must_be_grouped_in_threes_after_a_first_group_of_up_to_three_digits() {
    // Not thousands, so not R12 345 678.00, R1 234 567.00 or R12 345.50.
    assert!(!found("Ref 12345 678", 1_234_567_800));
    assert!(!found("Ref 1234 567", 123_456_700));
    assert!(!found("Ref 1 2345.50", 1_234_550));
    assert!(!found("Total 1 23.45", 12_345));
    // Properly grouped, they are.
    assert!(found("Ref 12 345 678", 1_234_567_800));
    assert!(found("Ref 123 456.50", 12_345_650));
    assert!(found("Ref 1 234 567", 123_456_700));
}

#[test]
fn a_group_of_exactly_three_digits_first_is_a_valid_start() {
    assert!(found("Total 123 456", 12_345_600));
    assert!(found("Total 123,456.78", 12_345_678));
}

#[test]
fn a_vat_number_is_read_as_its_digits_whatever_letters_come_with_it() {
    assert!(vat_found("VAT No 4123 456 789", "ZA 4123456789"));
    assert!(vat_found("VAT No 4123456789", "VAT4123-456-789"));
    assert!(!vat_found("VAT No 4123456780", "ZA 4123456789"));
}

#[test]
fn nothing_is_found_for_an_empty_target() {
    assert!(!stands_alone("anything 123", "", char::is_alphanumeric));
    assert!(!stands_alone("", "", char::is_alphanumeric));
}

// ---------- carried figures (ADR-0022) ----------

fn carried_only(brought: Option<i64>, forward: Option<i64>) -> Extraction {
    let mut e = clean();
    e.invoice_number = None;
    e.supplier = None;
    e.lines.clear();
    e.stated_totals = None;
    e.carried = vec![at(crate::extraction::CarriedTotals {
        brought_forward: brought.map(zar),
        carried_forward: forward.map(zar),
    })];
    e
}

#[test]
fn a_carried_figure_is_looked_for_on_the_page() {
    let extraction = carried_only(Some(100_000), Some(250_000));
    let report = trusted(&extraction, "Brought forward R1 000.00\nCarried forward R2 500.00");
    assert_eq!(report.result, TextCheck::Passed);
    assert_eq!(report.checked, 2);
}

#[test]
fn a_carried_figure_the_page_does_not_show_is_flagged() {
    let extraction = carried_only(Some(100_000), Some(250_000));
    let report = trusted(&extraction, "Brought forward R1 000.00"); // carried forward invented
    assert_eq!(report.result, TextCheck::Failed { missing: 1 });
    assert_eq!(report.unconfirmed, vec!["carried/carried_forward".to_string()]);
}
