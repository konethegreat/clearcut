// Written by Kone Tshivhinda & Claude
//! A safety net against personal data in a gold set (ADR-0010, privacy rule 3).
//!
//! Documents are anonymised by people before they enter the set; this module
//! only catches slips in the **label text** that a tired labeller might make:
//! an SA ID number, an email address, or a long number such as a phone or bank
//! account number, in a free-text field. It looks for shapes, so it can be
//! wrong in both directions (a person's name is invisible to it); it is a net,
//! not a guarantee, and says so.
//!
//! Fields that are identifiers by nature (invoice numbers, VAT numbers) are
//! not scanned, because long digit strings are normal there.

use crate::label::Truth;
use std::fmt;

/// A kind of personal data a piece of text looks like it contains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PersonalData {
    /// 13 digits with a valid date at the start and a valid check digit.
    SaIdNumber,
    EmailAddress,
    /// Nine or more digits in a row, or in groups of three or more digits
    /// joined by single spaces or dashes (a phone or bank account number).
    LongNumber,
}

impl fmt::Display for PersonalData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PersonalData::SaIdNumber => "an SA ID number",
            PersonalData::EmailAddress => "an email address",
            PersonalData::LongNumber => "a long number (phone or account number?)",
        })
    }
}

/// The kinds of personal data `text` looks like it contains, each once, in the
/// order of the [`PersonalData`] variants.
pub fn scan(text: &str) -> Vec<PersonalData> {
    let mut found = Vec::new();
    let groups = digit_groups(text);

    if groups.iter().any(looks_like_sa_id) {
        found.push(PersonalData::SaIdNumber);
    }
    if has_email(text) {
        found.push(PersonalData::EmailAddress);
    }
    if long_number(text, &groups) {
        found.push(PersonalData::LongNumber);
    }
    found
}

/// Every free-text field of the truth that holds personal data, named by field
/// (`supplier.address`), with the kind found. Never the text itself.
pub fn personal_data_in(truth: &Truth) -> Vec<(String, PersonalData)> {
    let mut fields: Vec<(String, &str)> = Vec::new();
    for (prefix, party) in [("supplier", &truth.supplier), ("recipient", &truth.recipient)] {
        if let Some(cited) = party {
            fields.push((format!("{prefix}.name"), &cited.value.name));
            if let Some(address) = &cited.value.address {
                fields.push((format!("{prefix}.address"), address));
            }
        }
    }
    for (index, line) in truth.lines.iter().enumerate() {
        fields.push((format!("lines[{index}].description"), &line.description));
    }
    if let Some(terms) = truth.due.as_ref().and_then(|due| due.terms.as_deref()) {
        fields.push(("due.terms".to_string(), terms));
    }

    let mut out = Vec::new();
    for (field, text) in fields {
        out.extend(scan(text).into_iter().map(|kind| (field.clone(), kind)));
    }
    out
}

/// A run of ASCII digits and where it sits in the text (byte offsets).
struct Group<'a> {
    start: usize,
    end: usize,
    digits: &'a str,
}

fn digit_groups(text: &str) -> Vec<Group<'_>> {
    let bytes = text.as_bytes();
    let mut groups = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            // Digits are ASCII, so these offsets are always on character edges.
            groups.push(Group {
                start,
                end: i,
                digits: &text[start..i],
            });
        } else {
            i += 1;
        }
    }
    groups
}

/// Exactly 13 digits: YYMMDD, then more digits, ending in a Luhn check digit.
/// Only the month, the day and the check digit are verified.
fn looks_like_sa_id(group: &Group<'_>) -> bool {
    let digits = group.digits.as_bytes();
    if digits.len() != 13 {
        return false;
    }
    let value = |i: usize| u32::from(digits[i] - b'0');
    let month = value(2) * 10 + value(3);
    let day = value(4) * 10 + value(5);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return false;
    }
    // Luhn: from the right, double every second digit starting with the second.
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(i, b)| {
            let d = u32::from(b - b'0');
            if i % 2 == 1 {
                let doubled = d * 2;
                if doubled > 9 {
                    doubled - 9
                } else {
                    doubled
                }
            } else {
                d
            }
        })
        .sum();
    sum.is_multiple_of(10)
}

fn has_email(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes
        .iter()
        .enumerate()
        .filter(|(_, b)| **b == b'@')
        .any(|(at, _)| {
            let before = at.checked_sub(1).map(|i| bytes[i]);
            let after = bytes.get(at + 1).copied();
            let local_ok = before
                .is_some_and(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'%' | b'+' | b'-'));
            let host_ok = after.is_some_and(|b| b.is_ascii_alphanumeric());
            if !(local_ok && host_ok) {
                return false;
            }
            // The host (the rest of the word, minus trailing punctuation) must
            // hold a dot. It starts with a letter or digit, so the dot is never
            // first, and trailing punctuation is trimmed, so it is never last.
            // (`at` is the offset of an ASCII byte, so `at + 1` is a character edge.)
            let rest = &text[at + 1..];
            let word = rest.split(char::is_whitespace).next().unwrap_or("");
            let word = word.trim_end_matches(|c: char| !c.is_alphanumeric());
            word.contains('.')
        })
}

/// Nine digits or more in one chain of groups (see [`PersonalData::LongNumber`]).
fn long_number(text: &str, groups: &[Group<'_>]) -> bool {
    let bytes = text.as_bytes();
    let mut chain = 0;
    let mut previous: Option<&Group<'_>> = None;
    for group in groups {
        let linked = previous.is_some_and(|prev| {
            prev.digits.len() >= 3
                && group.digits.len() >= 3
                && group.start == prev.end + 1
                && matches!(bytes[prev.end], b' ' | b'-')
        });
        chain = if linked {
            chain + group.digits.len()
        } else {
            group.digits.len()
        };
        if chain >= 9 {
            return true;
        }
        previous = Some(group);
    }
    false
}

#[cfg(test)]
mod tests;
