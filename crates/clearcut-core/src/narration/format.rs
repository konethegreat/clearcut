// Written by Kone Tshivhinda & Claude
//! How facts are written in prose. One module, used by the narrator now and by
//! the number guard (P3-06) later: the guard can format the facts of the cited
//! events with these same functions and check that every figure in a narrative
//! matches one of them exactly.
//!
//! Choices, all aimed at being unambiguous to a reader *and* to a parser:
//! - Rand amounts look like `R15 000.00`. The gap between thousands groups is a
//!   no-break space (U+00A0), so a figure is one unbreakable token.
//! - Dates are written out in full (`31 March 2026`), never `03/04/2026`.
//! - Rates are percentages with no trailing zeros (`15%`, `7.5%`).

use crate::event::{DuplicateReason, TotalField};
use crate::{Currency, Money, TaxCategory};
use time::Date;

/// No-break space: keeps `R15 000.00` on one line and as one token.
const NBSP: char = '\u{a0}';

/// `R15 000.00`, `-R5.00`, `USD 10.00`.
pub fn amount(money: Money) -> String {
    let sign = if money.minor < 0 { "-" } else { "" };
    // `unsigned_abs` is safe for i64::MIN, where plain negation would overflow.
    let abs = money.minor.unsigned_abs();
    let major = group_thousands(abs / 100);
    let cents = abs % 100;
    match money.currency {
        Currency::Zar => format!("{sign}R{major}.{cents:02}"),
        other => format!("{sign}{} {major}.{cents:02}", other.code()),
    }
}

fn group_thousands(n: u64) -> String {
    let digits: Vec<char> = n.to_string().chars().collect();
    // The first group takes the 1-3 leftover digits; every later group has 3.
    let first = match digits.len() % 3 {
        0 => 3,
        leftover => leftover,
    };
    let (head, rest) = digits.split_at(first.min(digits.len()));
    let mut out: String = head.iter().collect();
    for group in rest.chunks(3) {
        out.push(NBSP);
        out.extend(group);
    }
    out
}

/// A quantity held in thousandths: `10_000` is `10`, `10_500` is `10.5`.
pub fn quantity_milli(milli: i64) -> String {
    let sign = if milli < 0 { "-" } else { "" };
    let abs = milli.unsigned_abs();
    with_decimals(sign, abs / 1_000, &format!("{:03}", abs % 1_000))
}

/// A rate held in basis points: `1_500` is `15%`, `750` is `7.5%`.
pub fn rate_bps(bps: u32) -> String {
    let mut text = with_decimals("", u64::from(bps / 100), &format!("{:02}", bps % 100));
    text.push('%');
    text
}

/// `whole` plus a decimal part with trailing zeros removed (and no point if nothing is left).
fn with_decimals(sign: &str, whole: u64, fraction_digits: &str) -> String {
    let fraction = fraction_digits.trim_end_matches('0');
    if fraction.is_empty() {
        format!("{sign}{whole}")
    } else {
        format!("{sign}{whole}.{fraction}")
    }
}

/// `31 March 2026`.
pub fn date(date: Date) -> String {
    format!("{} {} {}", date.day(), date.month(), date.year())
}

/// How a tax category reads in a sentence.
pub fn tax_category(category: TaxCategory) -> &'static str {
    match category {
        TaxCategory::Standard => "standard-rated",
        TaxCategory::ZeroRated => "zero-rated",
        TaxCategory::Exempt => "VAT-exempt",
        TaxCategory::OutOfScope => "outside the scope of VAT",
    }
}

/// How a mismatched total reads in a sentence.
pub fn total_field(field: TotalField) -> &'static str {
    match field {
        TotalField::Net => "net total",
        TotalField::Vat => "VAT",
        TotalField::Gross => "gross total",
    }
}

/// Why two documents look alike, as a phrase.
pub fn duplicate_reason(reason: DuplicateReason) -> &'static str {
    match reason {
        DuplicateReason::ContentHash => "identical content",
        DuplicateReason::SupplierNumberDateTotal => "same supplier, invoice number, date and total",
    }
}
