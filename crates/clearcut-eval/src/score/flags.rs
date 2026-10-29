// Written by Kone Tshivhinda & Claude
//! Which fields a validation finding tells a reviewer to look at.
//!
//! A field counts as *flagged* (the system said a person must look) when the
//! case ended `needs_review`, or the field's confidence is under the review
//! threshold, or a finding is about it. This module is the third part. The
//! mapping errs on the side of flagging what a reviewer would have to recheck:
//!
//! - a problem with one line's amounts flags that line's quantity, unit price
//!   and total (any of the three could be the mistake);
//! - a mismatch between a printed total and the total rebuilt from the lines
//!   flags that total **and every line amount**, because the mistake may be in
//!   either, and a reviewer must look at both;
//! - an SA tax-invoice field rule flags the field it names.

use super::field::{Field, LinePart, PartyPart, TotalPart};
use clearcut_core::extraction::Finding;

/// Whether `finding` is about `field`.
pub fn is_about(finding: &Finding, field: Field) -> bool {
    match finding {
        Finding::NoLines => matches!(field, Field::Line(..)),
        Finding::CurrencyMismatch { line } => field == Field::Currency || is_line_amount_of(field, *line),
        Finding::NonPositiveQuantity { line }
        | Finding::AmountOutOfRange { line }
        | Finding::LineTotalMismatch { line, .. } => is_line_amount_of(field, *line),
        Finding::TotalsMismatch { field: total, .. } => {
            field == Field::Total(TotalPart::from(*total)) || field.is_line_amount()
        }
        Finding::TotalsOutOfRange => matches!(field, Field::Total(_)) || field.is_line_amount(),
        // A carried figure is not a field a label records yet (ADR-0022), so it flags nothing.
        Finding::CarriedBalanceMismatch { .. } => false,
        Finding::DueBeforeIssue => matches!(field, Field::InvoiceDate | Field::DueDate),
        Finding::Compliance(issue) => match issue.field {
            "supplier_name" => field == Field::Supplier(PartyPart::Name),
            "supplier_address" => field == Field::Supplier(PartyPart::Address),
            "supplier_vat_number" => field == Field::Supplier(PartyPart::VatNumber),
            "serial_number" => field == Field::InvoiceNumber,
            "date_of_issue" => field == Field::InvoiceDate,
            "description" => matches!(field, Field::Line(_, LinePart::Description)),
            "recipient_name" => field == Field::Recipient(PartyPart::Name),
            "recipient_address" => field == Field::Recipient(PartyPart::Address),
            "recipient_vat_number" => field == Field::Recipient(PartyPart::VatNumber),
            _ => false,
        },
    }
}

/// Whether any of `findings` is about `field`.
pub fn is_flagged_by(field: Field, findings: &[Finding]) -> bool {
    findings.iter().any(|finding| is_about(finding, field))
}

/// The quantity, unit price or total of the line the validator calls `line`
/// (counted from 1).
fn is_line_amount_of(field: Field, line: u32) -> bool {
    match field {
        Field::Line(index, _) if field.is_line_amount() => u32::try_from(index + 1) == Ok(line),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clearcut_core::compliance_za::Issue;
    use clearcut_core::event::TotalField;
    use clearcut_core::Money;

    const AMOUNT: Money = Money::zar(1);

    fn about(finding: Finding, field: Field) -> bool {
        is_about(&finding, field)
    }

    fn line(index: usize, part: LinePart) -> Field {
        Field::Line(index, part)
    }

    #[test]
    fn a_line_problem_flags_that_lines_three_amounts_and_nothing_else() {
        for finding in [
            Finding::NonPositiveQuantity { line: 2 },
            Finding::AmountOutOfRange { line: 2 },
            Finding::LineTotalMismatch {
                line: 2,
                stated: AMOUNT,
                recomputed: AMOUNT,
            },
        ] {
            for part in [LinePart::Quantity, LinePart::UnitPrice, LinePart::LineTotal] {
                assert!(about(finding.clone(), line(1, part)), "{finding:?} {part}");
                assert!(!about(finding.clone(), line(0, part)), "{finding:?} {part}");
                assert!(!about(finding.clone(), line(2, part)), "{finding:?} {part}");
            }
            assert!(!about(finding.clone(), line(1, LinePart::Description)));
            assert!(!about(finding.clone(), line(1, LinePart::Tax)));
            assert!(!about(finding.clone(), Field::Total(TotalPart::Net)));
            assert!(!about(finding.clone(), Field::Currency));
        }
    }

    #[test]
    fn a_currency_mismatch_flags_the_currency_and_that_lines_amounts() {
        let finding = Finding::CurrencyMismatch { line: 1 };
        assert!(about(finding.clone(), Field::Currency));
        assert!(about(finding.clone(), line(0, LinePart::UnitPrice)));
        assert!(!about(finding.clone(), line(1, LinePart::UnitPrice)));
        assert!(!about(finding, Field::InvoiceNumber));
    }

    #[test]
    fn no_lines_flags_every_line_field_that_might_have_been_missed() {
        let finding = Finding::NoLines;
        for part in [
            LinePart::Quantity,
            LinePart::UnitPrice,
            LinePart::LineTotal,
            LinePart::Description,
            LinePart::Tax,
        ] {
            assert!(about(finding.clone(), line(0, part)));
            assert!(about(finding.clone(), line(5, part)));
        }
        assert!(!about(finding.clone(), Field::Total(TotalPart::Gross)));
        assert!(!about(finding, Field::Supplier(PartyPart::Name)));
    }

    #[test]
    fn a_totals_mismatch_flags_that_total_and_every_line_amount() {
        let finding = Finding::TotalsMismatch {
            field: TotalField::Vat,
            stated: AMOUNT,
            recomputed: AMOUNT,
        };
        assert!(about(finding.clone(), Field::Total(TotalPart::Vat)));
        assert!(!about(finding.clone(), Field::Total(TotalPart::Net)));
        assert!(!about(finding.clone(), Field::Total(TotalPart::Gross)));
        for index in [0, 3] {
            for part in [LinePart::Quantity, LinePart::UnitPrice, LinePart::LineTotal] {
                assert!(about(finding.clone(), line(index, part)));
            }
            assert!(!about(finding.clone(), line(index, LinePart::Description)));
            assert!(!about(finding.clone(), line(index, LinePart::Tax)));
        }
        assert!(!about(finding, Field::InvoiceNumber));
    }

    #[test]
    fn each_total_has_its_own_mismatch() {
        for (total, part) in [
            (TotalField::Net, TotalPart::Net),
            (TotalField::Vat, TotalPart::Vat),
            (TotalField::Gross, TotalPart::Gross),
        ] {
            let finding = Finding::TotalsMismatch {
                field: total,
                stated: AMOUNT,
                recomputed: AMOUNT,
            };
            assert!(about(finding, Field::Total(part)));
        }
    }

    #[test]
    fn totals_out_of_range_flags_all_totals_and_line_amounts() {
        let finding = Finding::TotalsOutOfRange;
        for part in [TotalPart::Net, TotalPart::Vat, TotalPart::Gross] {
            assert!(about(finding.clone(), Field::Total(part)));
        }
        assert!(about(finding.clone(), line(2, LinePart::LineTotal)));
        assert!(!about(finding, line(2, LinePart::Description)));
    }

    #[test]
    fn a_due_date_before_the_invoice_date_flags_both_dates() {
        let finding = Finding::DueBeforeIssue;
        assert!(about(finding.clone(), Field::InvoiceDate));
        assert!(about(finding.clone(), Field::DueDate));
        assert!(!about(finding.clone(), Field::DueTerms));
        assert!(!about(finding, Field::InvoiceNumber));
    }

    #[test]
    fn each_sa_field_rule_flags_the_field_it_names() {
        let rules = [
            ("supplier_name", Field::Supplier(PartyPart::Name)),
            ("supplier_address", Field::Supplier(PartyPart::Address)),
            ("supplier_vat_number", Field::Supplier(PartyPart::VatNumber)),
            ("serial_number", Field::InvoiceNumber),
            ("date_of_issue", Field::InvoiceDate),
            ("description", line(3, LinePart::Description)),
            ("recipient_name", Field::Recipient(PartyPart::Name)),
            ("recipient_address", Field::Recipient(PartyPart::Address)),
            ("recipient_vat_number", Field::Recipient(PartyPart::VatNumber)),
        ];
        for (rule, named) in rules {
            let finding = Finding::Compliance(Issue {
                field: rule,
                message: "m",
            });
            assert!(about(finding.clone(), named), "{rule}");
            // ...and none of the other fields in this table.
            for (_, other) in rules.iter().filter(|(r, _)| *r != rule) {
                assert!(!about(finding.clone(), *other), "{rule} flagged {other}");
            }
            assert!(!about(finding, Field::Total(TotalPart::Gross)));
        }
    }

    #[test]
    fn an_unknown_rule_name_flags_nothing() {
        let finding = Finding::Compliance(Issue {
            field: "something_new",
            message: "m",
        });
        assert!(!about(finding, Field::InvoiceNumber));
    }

    #[test]
    fn a_field_is_flagged_if_any_finding_is_about_it() {
        let findings = vec![Finding::DueBeforeIssue, Finding::NoLines];
        assert!(is_flagged_by(Field::DueDate, &findings));
        assert!(is_flagged_by(line(0, LinePart::Tax), &findings));
        assert!(!is_flagged_by(Field::InvoiceNumber, &findings));
        assert!(!is_flagged_by(Field::DueDate, &[]));
    }
}
