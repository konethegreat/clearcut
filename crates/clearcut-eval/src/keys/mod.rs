// Written by Kone Tshivhinda & Claude
//! Short, stable names for validation findings, used in `expect.findings` in a
//! label and when scoring ("did the validator report what it should?").
//!
//! A key names the kind of finding and where it applies (a line number or a
//! total), never an amount, so keys are safe to print.

use clearcut_core::event::TotalField;
use clearcut_core::extraction::Finding;

/// The key of one finding, such as `totals_mismatch:vat` or `line_total_mismatch:2`.
pub fn finding_key(finding: &Finding) -> String {
    match finding {
        Finding::NoLines => "no_lines".to_string(),
        Finding::CurrencyMismatch { line } => format!("currency_mismatch:{line}"),
        Finding::NonPositiveQuantity { line } => format!("non_positive_quantity:{line}"),
        Finding::AmountOutOfRange { line } => format!("amount_out_of_range:{line}"),
        Finding::LineTotalMismatch { line, .. } => format!("line_total_mismatch:{line}"),
        Finding::TotalsMismatch { field, .. } => {
            let field = match field {
                TotalField::Net => "net",
                TotalField::Vat => "vat",
                TotalField::Gross => "gross",
            };
            format!("totals_mismatch:{field}")
        }
        Finding::TotalsOutOfRange => "totals_out_of_range".to_string(),
        Finding::CarriedBalanceMismatch { after_page, .. } => {
            format!("balance_mismatch:{after_page}")
        }
        Finding::DueBeforeIssue => "due_before_issue".to_string(),
        Finding::Compliance(issue) => format!("compliance:{}", issue.field),
    }
}

/// The keys of all `findings`, sorted (repeats kept), so that order never
/// decides whether two lists agree.
pub fn finding_keys(findings: &[Finding]) -> Vec<String> {
    let mut keys: Vec<String> = findings.iter().map(finding_key).collect();
    keys.sort();
    keys
}

/// How an actual list of keys differs from the expected one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Difference {
    /// Expected but not reported.
    pub missing: Vec<String>,
    /// Reported but not expected.
    pub unexpected: Vec<String>,
}

impl Difference {
    pub fn is_empty(&self) -> bool {
        self.missing.is_empty() && self.unexpected.is_empty()
    }
}

/// Compares two lists of keys as multisets: order is ignored, repeats count.
pub fn compare(expected: &[String], actual: &[String]) -> Difference {
    let mut expected: Vec<&String> = expected.iter().collect();
    let mut actual: Vec<&String> = actual.iter().collect();
    expected.sort();
    actual.sort();

    // Walk both sorted lists together, like merging two sorted piles of cards.
    let (mut missing, mut unexpected) = (Vec::new(), Vec::new());
    let (mut e, mut a) = (0, 0);
    while e < expected.len() && a < actual.len() {
        match expected[e].cmp(actual[a]) {
            std::cmp::Ordering::Equal => {
                e += 1;
                a += 1;
            }
            std::cmp::Ordering::Less => {
                missing.push(expected[e].clone());
                e += 1;
            }
            std::cmp::Ordering::Greater => {
                unexpected.push(actual[a].clone());
                a += 1;
            }
        }
    }
    missing.extend(expected[e..].iter().map(|key| (*key).clone()));
    unexpected.extend(actual[a..].iter().map(|key| (*key).clone()));
    Difference { missing, unexpected }
}

#[cfg(test)]
mod tests;
