// Written by Kone Tshivhinda & Claude
//! Did the model read what the page says? (ADR-0013 section 4 and ADR-0018 section 3; P2-13c.)
//!
//! A number the model "read" that is not on the page is the most dangerous silent error: it can
//! be arithmetically perfect and completely invented. The page's own text (its text layer, or
//! OCR) is a second, independent reading, so every figure the model returned can be looked for
//! in it. This is deterministic, free, and a check on *evidence*, not on meaning: it looks for
//! the digits.
//!
//! What is checked: each amount (unit prices and line totals, the three totals, a credit note's
//! amount), the invoice number and the supplier's VAT number. Amounts of zero are not checked (a
//! page may show a dash, or nothing). Quantities, dates, names and descriptions are not checked:
//! a small quantity is on every page, and a date or a name is printed in too many ways for a
//! miss to mean anything yet; the gold set will say whether that is worth adding.
//!
//! **How much a miss means depends on the text.** If the text is *trusted* (a text layer that
//! passed its checks), a figure that is not in it is a failure. If the text is *unverified* (a
//! layer that OCR only partly agreed with, or OCR text of a scan), the text may be the wrong one,
//! so a miss proves nothing and is only reported, while agreement is still good evidence: two
//! independent readers saw the same figures.
//!
//! Known limit, on purpose: a space is a thousands separator in South Africa and also separates
//! neighbouring numbers, so `10 150.00` is both R10 150.00 and R150.00 after a 10. Every genuine
//! reading counts as evidence, which means a model that merged two neighbours is confirmed. The
//! arithmetic check is what catches that.
//!
//! The result never holds a value from the page or from the reading: only the place of each
//! figure that was not found (`totals/gross`) and counts.

#[cfg(test)]
mod tests;

use crate::escalation::TextCheck;
use crate::extraction::Extraction;
use crate::Money;
use std::collections::HashSet;

/// How far the page's text can be believed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextTrust {
    /// A text layer that passed its checks (route `text_and_image`): a miss is a failure.
    Trusted,
    /// A text layer that is only a hint, or OCR text: a miss proves nothing.
    Unverified,
}

/// What the check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextCheckReport {
    /// What the assessment takes: not available, passed, or failed with a count.
    pub result: TextCheck,
    /// How many figures were looked for.
    pub checked: u32,
    /// The place of each one that was not found, in a fixed order: invoice number, supplier VAT
    /// number, each line's unit price then total, the totals, the credit note's amount.
    pub unconfirmed: Vec<String>,
}

/// A run of digit groups this long is cut: nothing real is longer, and it bounds the work.
const MAX_GROUPS: usize = 32;

/// What to look for, and where it came from.
enum Wanted {
    /// An amount in cents (sign ignored).
    Amount(u64),
    /// Letters and digits, lower case, nothing else.
    Alnum(String),
    /// Digits only.
    Digits(String),
}

fn cents_of(money: Money) -> Option<u64> {
    let cents = money.minor.unsigned_abs();
    (cents != 0).then_some(cents)
}

fn alnum_of(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn digits_of(text: &str) -> String {
    text.chars().filter(char::is_ascii_digit).collect()
}

/// Every figure of the extraction that can be looked for, with its place.
fn wanted(extraction: &Extraction) -> Vec<(String, Wanted)> {
    let mut out = Vec::new();
    if let Some(number) = &extraction.invoice_number {
        let normalised = alnum_of(&number.value);
        if !normalised.is_empty() {
            out.push(("invoice_number".to_string(), Wanted::Alnum(normalised)));
        }
    }
    if let Some(vat) = extraction
        .supplier
        .as_ref()
        .and_then(|supplier| supplier.value.vat_number.as_deref())
    {
        let digits = digits_of(vat);
        if !digits.is_empty() {
            out.push(("supplier/vat_number".to_string(), Wanted::Digits(digits)));
        }
    }
    let mut amount = |path: String, money: Money| {
        if let Some(cents) = cents_of(money) {
            out.push((path, Wanted::Amount(cents)));
        }
    };
    for (index, line) in extraction.lines.iter().enumerate() {
        let n = index + 1;
        amount(format!("lines/{n}/unit_price"), line.value.unit_price);
        amount(format!("lines/{n}/line_total"), line.value.line_total);
    }
    if let Some(totals) = &extraction.stated_totals {
        amount("totals/net".to_string(), totals.value.net);
        amount("totals/vat".to_string(), totals.value.vat);
        amount("totals/gross".to_string(), totals.value.gross);
    }
    if let Some(credit) = &extraction.credit_note {
        amount("credit_note/amount".to_string(), credit.value.amount);
    }
    out
}

/// Looks for what the model read in the page's own text.
pub fn check_against_text(extraction: &Extraction, text: &str, trust: TextTrust) -> TextCheckReport {
    if text.trim().is_empty() {
        return TextCheckReport {
            result: TextCheck::NotAvailable,
            checked: 0,
            unconfirmed: Vec::new(),
        };
    }
    let wanted = wanted(extraction);
    let checked = u32::try_from(wanted.len()).unwrap_or(u32::MAX);
    // Only worked out if something needs it.
    let amounts = wanted
        .iter()
        .any(|(_, w)| matches!(w, Wanted::Amount(_)))
        .then(|| printed_amounts(text));
    let unconfirmed: Vec<String> = wanted
        .into_iter()
        .filter(|(_, w)| {
            let found = match w {
                Wanted::Amount(cents) => amounts.as_ref().is_some_and(|set| set.contains(cents)),
                Wanted::Alnum(target) => stands_alone(text, target, char::is_alphanumeric),
                Wanted::Digits(target) => stands_alone(text, target, |c| c.is_ascii_digit()),
            };
            !found
        })
        .map(|(path, _)| path)
        .collect();
    let result = if checked == 0 {
        TextCheck::NotAvailable
    } else if unconfirmed.is_empty() {
        TextCheck::Passed
    } else {
        match trust {
            TextTrust::Trusted => TextCheck::Failed {
                missing: u32::try_from(unconfirmed.len()).unwrap_or(u32::MAX),
            },
            TextTrust::Unverified => TextCheck::NotAvailable,
        }
    };
    TextCheckReport {
        result,
        checked,
        unconfirmed,
    }
}

/// Whether `target` (lower-case symbols) occurs in the text's symbols (the characters `keep`
/// accepts, anything else being a separator) as a whole: it must start where a run of symbols
/// starts and be followed by the start of another run or the end. So `INV-000001` is found in
/// `Invoice No: inv 000001` and `INV/000001`, but not in `INV-0000011` or `XINV-000001`.
fn stands_alone(text: &str, target: &str, keep: fn(char) -> bool) -> bool {
    let target: Vec<char> = target.chars().collect();
    if target.is_empty() {
        return false;
    }
    // Each symbol, and whether it begins a run (the character before it was not a symbol).
    let mut symbols: Vec<(char, bool)> = Vec::new();
    let mut previous_was_symbol = false;
    for c in text.chars() {
        if keep(c) {
            for lower in c.to_lowercase() {
                symbols.push((lower, !previous_was_symbol));
            }
            previous_was_symbol = true;
        } else {
            previous_was_symbol = false;
        }
    }
    let n = target.len();
    if symbols.len() < n {
        return false;
    }
    (0..=symbols.len() - n).any(|p| {
        symbols[p].1
            && symbols[p..p + n].iter().map(|s| s.0).eq(target.iter().copied())
            && symbols.get(p + n).is_none_or(|next| next.1)
    })
}

fn is_separator(c: char) -> bool {
    matches!(c, ' ' | '\u{a0}' | ',' | '.' | '\'' | '\u{2019}')
}

/// Every amount, in cents, that some reading of the page's numbers gives.
fn printed_amounts(text: &str) -> HashSet<u64> {
    let chars: Vec<char> = text.chars().collect();
    let mut amounts = HashSet::new();
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        // A run: digit groups joined by single separators, each followed by another digit.
        let mut groups: Vec<String> = Vec::new();
        let mut separators: Vec<char> = Vec::new();
        loop {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            groups.push(chars[start..i].iter().collect());
            let joined = i + 1 < chars.len() && is_separator(chars[i]) && chars[i + 1].is_ascii_digit();
            if groups.len() >= MAX_GROUPS || !joined {
                break;
            }
            separators.push(chars[i]);
            i += 1;
        }
        // Every stretch of the run is a reading: "10 150.00" is 10 150.00 and 150.00.
        for first in 0..groups.len() {
            for last in first..groups.len() {
                if let Some(cents) = read_amount(&groups[first..=last], &separators[first..last]) {
                    amounts.insert(cents);
                }
            }
        }
    }
    amounts
}

/// The thousands groups of a number written with separators: one group of any size, or a
/// first group of up to three digits and then groups of exactly three. (A group is never empty:
/// a separator only joins groups when a digit follows it.) Digits beyond what fits in a `u64`
/// are no amount, and the checked arithmetic below drops them.
fn grouped(groups: &[String]) -> bool {
    match groups {
        [_] => true,
        [first, rest @ ..] => first.len() <= 3 && rest.iter().all(|g| g.len() == 3),
        [] => false,
    }
}

fn whole_rand(groups: &[String]) -> Option<u64> {
    groups.concat().parse().ok()
}

/// The value in cents of digit groups and the separators between them, if they make an amount.
fn read_amount(groups: &[String], separators: &[char]) -> Option<u64> {
    let [.., last] = groups else { return None };
    if groups.len() == 1 {
        return whole_rand(groups)?.checked_mul(100);
    }
    let decimal_separator = matches!(separators.last(), Some('.' | ','));
    if last.len() <= 2 && decimal_separator {
        // 1 234,50 / 1,234.50 / 12.5: the last group is the cents.
        let integer = &groups[..groups.len() - 1];
        if !grouped(integer) {
            return None;
        }
        let cents: u64 = last.parse().ok()?;
        let cents = if last.len() == 1 { cents * 10 } else { cents };
        whole_rand(integer)?.checked_mul(100)?.checked_add(cents)
    } else if grouped(groups) {
        // 1 500 / 1,500 / 1.500: thousands, no cents.
        whole_rand(groups)?.checked_mul(100)
    } else {
        None
    }
}
