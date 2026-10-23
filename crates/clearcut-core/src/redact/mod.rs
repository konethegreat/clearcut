// Written by Kone Tshivhinda & Claude
//! Finding personal data that a reading does not need, so it can be left out of
//! what is sent to the model (ADR-0014 §2, POPIA data minimisation).
//!
//! The rules are deterministic and deliberately **conservative**: a number is
//! reported only when a check digit proves it is that kind of number (a South
//! African ID number, a card, an IBAN), or when it follows its own label
//! (`Account No:`, `Branch Code:`, `Tel:`). An account number on its own looks
//! exactly like an invoice number, so it is never guessed at.
//!
//! **Never redacted:** invoice numbers, dates, amounts, quantities, VAT
//! registration numbers, names and addresses. They are what a reading is for.
//!
//! This is minimisation, **not anonymisation**. Handwriting, scans with no text
//! layer and unusual formats are missed, and a number can be hidden in a row of
//! four-digit amounts by chance; the gold-set run is scored with redaction on
//! and off to show whether it ever costs accuracy. Spans are byte offsets into
//! the original text and always fall on character boundaries (every match is
//! made of ASCII).

#[cfg(test)]
mod tests;

use serde::Serialize;

/// What kind of personal data a span is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RedactionKind {
    // The order is the priority when two kinds match the same text: the more
    // specific kind comes first.
    SaIdNumber,
    CardNumber,
    Iban,
    BankAccount,
    BranchCode,
    SwiftCode,
    Email,
    Phone,
}

impl RedactionKind {
    pub const ALL: [RedactionKind; 8] = [
        RedactionKind::SaIdNumber,
        RedactionKind::CardNumber,
        RedactionKind::Iban,
        RedactionKind::BankAccount,
        RedactionKind::BranchCode,
        RedactionKind::SwiftCode,
        RedactionKind::Email,
        RedactionKind::Phone,
    ];

    /// The fixed name used in placeholders, logs and the manifest.
    pub fn as_str(self) -> &'static str {
        match self {
            RedactionKind::SaIdNumber => "sa_id_number",
            RedactionKind::CardNumber => "card_number",
            RedactionKind::Iban => "iban",
            RedactionKind::BankAccount => "bank_account",
            RedactionKind::BranchCode => "branch_code",
            RedactionKind::SwiftCode => "swift_code",
            RedactionKind::Email => "email",
            RedactionKind::Phone => "phone",
        }
    }

    /// What replaces a value of this kind. It holds no digits, so redacting an
    /// already redacted text changes nothing.
    pub fn placeholder(self) -> &'static str {
        match self {
            RedactionKind::SaIdNumber => "[REDACTED sa_id_number]",
            RedactionKind::CardNumber => "[REDACTED card_number]",
            RedactionKind::Iban => "[REDACTED iban]",
            RedactionKind::BankAccount => "[REDACTED bank_account]",
            RedactionKind::BranchCode => "[REDACTED branch_code]",
            RedactionKind::SwiftCode => "[REDACTED swift_code]",
            RedactionKind::Email => "[REDACTED email]",
            RedactionKind::Phone => "[REDACTED phone]",
        }
    }
}

/// A piece of text to redact: `start..end` are byte offsets into the original.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub kind: RedactionKind,
    pub start: usize,
    pub end: usize,
}

/// The text with placeholders in it, and what was replaced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redacted {
    pub text: String,
    /// In the **original** text's coordinates, in order, not overlapping.
    pub spans: Vec<Span>,
}

impl Redacted {
    pub fn count(&self, kind: RedactionKind) -> usize {
        self.spans.iter().filter(|span| span.kind == kind).count()
    }

    pub fn total(&self) -> usize {
        self.spans.len()
    }
}

/// Replaces every span found by [`scan`] with its placeholder.
pub fn redact(text: &str) -> Redacted {
    let spans = scan(text);
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for span in &spans {
        out.push_str(&text[last..span.start]);
        out.push_str(span.kind.placeholder());
        last = span.end;
    }
    out.push_str(&text[last..]);
    Redacted { text: out, spans }
}

/// Finds the personal data in `text`: sorted, not overlapping, never empty.
pub fn scan(text: &str) -> Vec<Span> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    scan_numbers(bytes, &mut found);
    scan_ibans(bytes, &mut found);
    scan_labelled(bytes, &mut found);
    scan_emails(bytes, &mut found);

    // Earliest first; for the same start the more specific kind, then the longer.
    found.sort_by(|a, b| {
        (a.start, a.kind, std::cmp::Reverse(a.end)).cmp(&(b.start, b.kind, std::cmp::Reverse(b.end)))
    });
    let mut kept: Vec<Span> = Vec::with_capacity(found.len());
    for span in found {
        if kept.last().is_none_or(|last| span.start >= last.end) {
            kept.push(span);
        }
    }
    kept
}

// ---------------------------------------------------------------- helpers ----

fn alnum_before(bytes: &[u8], index: usize) -> bool {
    index > 0 && bytes[index - 1].is_ascii_alphanumeric()
}

fn alnum_at(bytes: &[u8], index: usize) -> bool {
    index < bytes.len() && bytes[index].is_ascii_alphanumeric()
}

/// The Luhn check used by card numbers and South African ID numbers.
fn luhn_ok(digits: &[u8]) -> bool {
    let mut sum = 0u32;
    for (position, digit) in digits.iter().rev().enumerate() {
        let mut value = u32::from(digit - b'0');
        if !position.is_multiple_of(2) {
            value *= 2;
            if value > 9 {
                value -= 9;
            }
        }
        sum += value;
    }
    sum.is_multiple_of(10)
}

fn number_of(digits: &[u8]) -> u32 {
    digits.iter().fold(0, |n, d| n * 10 + u32::from(d - b'0'))
}

// ----------------------------------------------------- ID numbers and cards ----

/// 13 digits: a real birth date (YYMMDD), citizenship digit 0 or 1, and the check digit.
fn is_sa_id(d: &[u8]) -> bool {
    if d.len() != 13 {
        return false;
    }
    let month = number_of(&d[2..4]);
    let day = number_of(&d[4..6]);
    let longest = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => 29, // the century is unknown, so a leap day is allowed
        _ => return false,
    };
    (1..=longest).contains(&day) && matches!(d[10], b'0' | b'1') && luhn_ok(d)
}

/// A card number: a known network prefix, a length that network uses, and the Luhn check.
fn is_card(d: &[u8]) -> bool {
    let n = d.len();
    if !(13..=19).contains(&n) {
        return false;
    }
    let p2 = number_of(&d[..2]);
    let p3 = number_of(&d[..3]);
    let p4 = number_of(&d[..4]);
    let known = (d[0] == b'4' && matches!(n, 13 | 16 | 19))
        || (n == 16 && ((51..=55).contains(&p2) || (2221..=2720).contains(&p4)))
        || (n == 15 && matches!(p2, 34 | 37))
        || (matches!(n, 14 | 16) && ((300..=305).contains(&p3) || matches!(p2, 36 | 38 | 39)))
        || (matches!(n, 16 | 19) && (p4 == 6011 || p2 == 65 || (644..=649).contains(&p3)))
        || (n == 16 && (3528..=3589).contains(&p4));
    known && luhn_ok(d)
}

/// Groups as cards print them: 4-4-4-4, 4-4-4-4-3, or the 4-6-5 / 4-6-4 of Amex and Diners.
fn is_card_grouping(lengths: &[usize]) -> bool {
    matches!(lengths, [4, 4, 4, 4] | [4, 4, 4, 4, 3] | [4, 6, 5] | [4, 6, 4])
}

fn scan_numbers(bytes: &[u8], out: &mut Vec<Span>) {
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() || alnum_before(bytes, i) {
            i += 1;
            continue;
        }
        // A chain of digit groups joined by single spaces or dashes: `4111 1111 1111 1111`.
        let mut groups: Vec<(usize, usize)> = Vec::new();
        let mut pos = i;
        loop {
            let start = pos;
            while pos < bytes.len() && bytes[pos].is_ascii_digit() {
                pos += 1;
            }
            groups.push((start, pos));
            let joined =
                pos + 1 < bytes.len() && matches!(bytes[pos], b' ' | b'-') && bytes[pos + 1].is_ascii_digit();
            if !joined {
                break;
            }
            pos += 1;
        }
        let chain_end = pos;

        // Each group on its own (an ID number can be followed by `12 items`).
        for &(start, end) in &groups {
            if alnum_at(bytes, end) {
                continue;
            }
            let digits = &bytes[start..end];
            if is_sa_id(digits) {
                out.push(Span {
                    kind: RedactionKind::SaIdNumber,
                    start,
                    end,
                });
            } else if is_card(digits) {
                out.push(Span {
                    kind: RedactionKind::CardNumber,
                    start,
                    end,
                });
            }
        }
        // The whole chain, as a printed card.
        if groups.len() > 1 && !alnum_at(bytes, chain_end) {
            let lengths: Vec<usize> = groups.iter().map(|(s, e)| e - s).collect();
            if is_card_grouping(&lengths) {
                let digits: Vec<u8> = groups
                    .iter()
                    .flat_map(|&(s, e)| bytes[s..e].iter().copied())
                    .collect();
                if is_card(&digits) {
                    out.push(Span {
                        kind: RedactionKind::CardNumber,
                        start: i,
                        end: chain_end,
                    });
                }
            }
        }
        i = chain_end;
    }
}

// ------------------------------------------------------------------- IBANs ----

fn iban_mod97_ok(alnum: &[u8]) -> bool {
    if !(15..=34).contains(&alnum.len()) {
        return false;
    }
    let mut remainder: u32 = 0;
    for &byte in alnum[4..].iter().chain(alnum[..4].iter()) {
        remainder = if byte.is_ascii_digit() {
            (remainder * 10 + u32::from(byte - b'0')) % 97
        } else {
            (remainder * 100 + u32::from(byte - b'A') + 10) % 97
        };
    }
    remainder == 1
}

fn iban_char(byte: u8) -> bool {
    byte.is_ascii_uppercase() || byte.is_ascii_digit()
}

fn scan_ibans(bytes: &[u8], out: &mut Vec<Span>) {
    let mut i = 0;
    while i + 4 <= bytes.len() {
        let starts = bytes[i].is_ascii_uppercase()
            && bytes[i + 1].is_ascii_uppercase()
            && bytes[i + 2].is_ascii_digit()
            && bytes[i + 3].is_ascii_digit()
            && !alnum_before(bytes, i);
        if !starts {
            i += 1;
            continue;
        }
        // Groups of capitals and digits separated by single spaces.
        let mut groups: Vec<(usize, usize)> = Vec::new();
        let mut pos = i;
        let mut total = 0;
        loop {
            let start = pos;
            while pos < bytes.len() && iban_char(bytes[pos]) {
                pos += 1;
            }
            total += pos - start;
            if pos == start || total > 34 {
                break;
            }
            groups.push((start, pos));
            if pos + 1 < bytes.len() && bytes[pos] == b' ' && iban_char(bytes[pos + 1]) {
                pos += 1;
            } else {
                break;
            }
        }
        // The longest run of groups that passes the check and ends cleanly.
        for take in (1..=groups.len()).rev() {
            let end = groups[take - 1].1;
            if alnum_at(bytes, end) {
                continue;
            }
            let alnum: Vec<u8> = groups[..take]
                .iter()
                .flat_map(|&(s, e)| bytes[s..e].iter().copied())
                .collect();
            if iban_mod97_ok(&alnum) {
                out.push(Span {
                    kind: RedactionKind::Iban,
                    start: i,
                    end,
                });
                break;
            }
        }
        i += 1;
        // Skip the rest of this token so it is not tried again from the middle.
        while i < bytes.len() && iban_char(bytes[i]) {
            i += 1;
        }
    }
}

// ---------------------------------------------------------- labelled values ----

/// Words that may sit between a label and its value: `Account No.: 123`.
const NUMBER_WORDS: [&str; 4] = ["number", "num", "nr", "no"];

fn skip_filler(bytes: &[u8], mut pos: usize) -> usize {
    loop {
        let before = pos;
        while pos < bytes.len() && matches!(bytes[pos], b' ' | b'\t' | b'.' | b':' | b'#' | b'-') {
            pos += 1;
        }
        for word in NUMBER_WORDS {
            if bytes[pos..].starts_with(word.as_bytes()) && !alnum_at(bytes, pos + word.len()) {
                pos += word.len();
                break;
            }
        }
        if pos == before {
            return pos;
        }
    }
}

/// Digits with single spaces or dashes between them, `min..=max` digits in all.
fn digits_value(bytes: &[u8], start: usize, min: usize, max: usize) -> Option<(usize, usize)> {
    if start >= bytes.len() || !bytes[start].is_ascii_digit() {
        return None;
    }
    let mut pos = start;
    let mut end = start;
    let mut count = 0;
    while pos < bytes.len() {
        if bytes[pos].is_ascii_digit() {
            count += 1;
            pos += 1;
            end = pos;
        } else if matches!(bytes[pos], b' ' | b'-')
            && pos + 1 < bytes.len()
            && bytes[pos + 1].is_ascii_digit()
        {
            pos += 1;
        } else {
            break;
        }
    }
    ((min..=max).contains(&count) && !alnum_at(bytes, end)).then_some((start, end))
}

/// A phone number: digits with spaces, dashes, dots, brackets and a leading plus.
fn phone_value(bytes: &[u8], start: usize) -> Option<(usize, usize)> {
    if start >= bytes.len() || !matches!(bytes[start], b'0'..=b'9' | b'+' | b'(') {
        return None;
    }
    let mut pos = start;
    let mut end = start;
    let mut count = 0;
    while pos < bytes.len()
        && (bytes[pos].is_ascii_digit() || matches!(bytes[pos], b' ' | b'-' | b'(' | b')' | b'+' | b'.'))
    {
        if bytes[pos].is_ascii_digit() {
            count += 1;
            end = pos + 1;
        }
        pos += 1;
    }
    ((9..=15).contains(&count) && !alnum_at(bytes, end)).then_some((start, end))
}

/// A SWIFT or BIC code: 8 or 11 characters, the first six capital letters.
fn swift_value(bytes: &[u8], start: usize) -> Option<(usize, usize)> {
    let mut end = start;
    while end < bytes.len() && bytes[end].is_ascii_alphanumeric() {
        end += 1;
    }
    let value = &bytes[start..end];
    let shaped = matches!(value.len(), 8 | 11)
        && value[..6].iter().all(u8::is_ascii_uppercase)
        && value.iter().all(|&b| iban_char(b));
    shaped.then_some((start, end))
}

fn scan_labelled(bytes: &[u8], out: &mut Vec<Span>) {
    // Lower-casing ASCII keeps every byte offset the same as in the original.
    let lower: Vec<u8> = bytes.to_ascii_lowercase();
    let find = |phrases: &[&str]| -> Vec<usize> {
        let mut ends = Vec::new();
        for phrase in phrases {
            for (start, _) in lower_matches(&lower, phrase.as_bytes()) {
                let end = start + phrase.len();
                if !alnum_before(&lower, start) && !alnum_at(&lower, end) {
                    ends.push(end);
                }
            }
        }
        ends
    };

    let rules: [(RedactionKind, &[&str]); 4] = [
        (RedactionKind::BankAccount, &["account", "acc", "a/c"]),
        (
            RedactionKind::BranchCode,
            &["branch code", "branch number", "branch no", "branch", "sort code"],
        ),
        (
            RedactionKind::SwiftCode,
            &["swift code", "swift", "bic code", "bic"],
        ),
        (
            RedactionKind::Phone,
            &[
                "telephone",
                "tel",
                "cellphone",
                "cell",
                "mobile",
                "mob",
                "fax",
                "phone",
                "whatsapp",
            ],
        ),
    ];
    for (kind, phrases) in rules {
        for end in find(phrases) {
            let start = skip_filler(&lower, end);
            let value = match kind {
                RedactionKind::BankAccount => digits_value(bytes, start, 6, 16),
                RedactionKind::BranchCode => digits_value(bytes, start, 5, 8),
                RedactionKind::SwiftCode => swift_value(bytes, start),
                RedactionKind::Phone => phone_value(bytes, start),
                _ => None,
            };
            if let Some((start, end)) = value {
                out.push(Span { kind, start, end });
            }
        }
    }
}

/// Every place `needle` occurs in `haystack`, as `(start, end)`.
fn lower_matches(haystack: &[u8], needle: &[u8]) -> Vec<(usize, usize)> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return Vec::new();
    }
    (0..=haystack.len() - needle.len())
        .filter(|&i| &haystack[i..i + needle.len()] == needle)
        .map(|i| (i, i + needle.len()))
        .collect()
}

// ------------------------------------------------------------------- e-mail ----

fn local_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'%' | b'+' | b'-')
}

fn domain_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-')
}

fn scan_emails(bytes: &[u8], out: &mut Vec<Span>) {
    for (at, _) in bytes.iter().enumerate().filter(|(_, &b)| b == b'@') {
        let mut start = at;
        while start > 0 && local_char(bytes[start - 1]) {
            start -= 1;
        }
        while start < at && matches!(bytes[start], b'.' | b'-') {
            start += 1;
        }
        let mut end = at + 1;
        while end < bytes.len() && domain_char(bytes[end]) {
            end += 1;
        }
        while end > at + 1 && matches!(bytes[end - 1], b'.' | b'-') {
            end -= 1;
        }
        if start == at || end == at + 1 {
            continue;
        }
        let domain = &bytes[at + 1..end];
        let labels: Vec<&[u8]> = domain.split(|&b| b == b'.').collect();
        let tld_ok = labels
            .last()
            .is_some_and(|tld| tld.len() >= 2 && tld.iter().all(u8::is_ascii_alphabetic));
        if labels.len() >= 2 && labels.iter().all(|label| !label.is_empty()) && tld_ok {
            out.push(Span {
                kind: RedactionKind::Email,
                start,
                end,
            });
        }
    }
}
