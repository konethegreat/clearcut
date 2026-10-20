// Written by Kone Tshivhinda & Claude
//! Finds the figures and dates in a piece of narration so the guard can check
//! each one. No regex crate: the shapes are few and a small scanner is easier
//! to read and test.
//!
//! What counts as a numeral: a run of digits, optionally joined by no-break
//! spaces into groups of three (the way `narration::format` writes amounts)
//! and optionally followed by `.digits`. `R15 000.00` becomes `15000.00`.
//! A plain space does not join groups, so a sloppy `R15 000.00` splits into
//! `15` and `000.00` and fails verification. That is intended: the guard is
//! strict, and a false rejection only costs a fall back to template text.
//!
//! A numeral keeps its sign. The sign is the `-` (or `−`) just before the
//! number, allowing for a currency prefix between them (`-R5.00`,
//! `-EUR 2 500.00`). Dropping a minus sign changes the meaning of a figure.
//!
//! What counts as a date, checked as one unit: `30 April 2026` (full month
//! name) and `2026-04-30`. Reading them whole means "30 May 2026" cannot pass
//! just because 30 and 2026 appear elsewhere. Abbreviated months are not
//! recognised as dates; their numbers are checked as plain numerals instead.

use time::{Date, Month};

/// No-break space: the only thing that joins digit groups.
const NBSP: char = '\u{a0}';

const MONTH_NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Everything numeric found in a text, in the order it appears.
#[derive(Debug, Default)]
pub(super) struct Scan {
    /// Normalised numerals: separators removed, sign kept (`-2500.00`).
    pub numerals: Vec<String>,
    pub dates: Vec<ScannedDate>,
}

#[derive(Debug)]
pub(super) struct ScannedDate {
    /// The date as it was written.
    pub raw: String,
    /// The calendar date, or `None` when it is impossible (31 April).
    pub date: Option<Date>,
}

pub(super) fn scan(text: &str) -> Scan {
    let chars: Vec<char> = text.chars().collect();
    let mut found = Scan::default();
    let mut i = 0;
    while i < chars.len() {
        // Every digit run is consumed whole below, so a digit here always
        // starts a new run.
        if !chars[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        if let Some((date, end)) = date_at(&chars, i) {
            found.dates.push(date);
            i = end;
            continue;
        }
        let (value, end) = number_at(&chars, i);
        let sign = if is_negative(&chars, i) { "-" } else { "" };
        found.numerals.push(format!("{sign}{value}"));
        i = end;
    }
    found
}

/// Index just past the run of ASCII digits starting at `start`.
fn digit_run(chars: &[char], start: usize) -> usize {
    let mut end = start;
    while end < chars.len() && chars[end].is_ascii_digit() {
        end += 1;
    }
    end
}

fn collect(chars: &[char]) -> String {
    chars.iter().collect()
}

/// The number starting at `start`, normalised, and the index just past it.
fn number_at(chars: &[char], start: usize) -> (String, usize) {
    let mut end = digit_run(chars, start);
    let mut value = collect(&chars[start..end]);

    // Further groups of exactly three digits, each joined by a no-break space.
    if end - start <= 3 {
        while end + 3 < chars.len()
            && chars[end] == NBSP
            && chars[end + 1..=end + 3].iter().all(char::is_ascii_digit)
            && chars.get(end + 4).is_none_or(|c| !c.is_ascii_digit())
        {
            value.push_str(&collect(&chars[end + 1..=end + 3]));
            end += 4;
        }
    }

    // A decimal part, but not a full stop that merely ends the sentence.
    if chars.get(end) == Some(&'.') && chars.get(end + 1).is_some_and(char::is_ascii_digit) {
        let fraction_end = digit_run(chars, end + 1);
        value.push('.');
        value.push_str(&collect(&chars[end + 1..fraction_end]));
        end = fraction_end;
    }
    (value, end)
}

/// True when the number starting at `start` is written with a minus sign:
/// `-5`, `-R5`, `-EUR 5`. Looks back over one optional space and a currency
/// prefix of up to three capital letters.
fn is_negative(chars: &[char], start: usize) -> bool {
    let mut p = start;
    if p > 0 && chars[p - 1] == ' ' {
        p -= 1;
    }
    let mut letters = 0;
    while p > 0 && chars[p - 1].is_ascii_uppercase() && letters < 3 {
        p -= 1;
        letters += 1;
    }
    p > 0 && matches!(chars[p - 1], '-' | '\u{2212}')
}

/// A date starting at `start`, if one is written there, and the index just past it.
fn date_at(chars: &[char], start: usize) -> Option<(ScannedDate, usize)> {
    written_date_at(chars, start).or_else(|| iso_date_at(chars, start))
}

/// `30 April 2026`.
fn written_date_at(chars: &[char], start: usize) -> Option<(ScannedDate, usize)> {
    let day_end = digit_run(chars, start);
    if !(1..=2).contains(&(day_end - start)) || chars.get(day_end) != Some(&' ') {
        return None;
    }
    let word_start = day_end + 1;
    let mut word_end = word_start;
    while word_end < chars.len() && chars[word_end].is_ascii_alphabetic() {
        word_end += 1;
    }
    let word = collect(&chars[word_start..word_end]);
    let month_index = MONTH_NAMES.iter().position(|name| *name == word)?;
    if chars.get(word_end) != Some(&' ') {
        return None;
    }
    let year_start = word_end + 1;
    let year_end = digit_run(chars, year_start);
    if year_end - year_start != 4 {
        return None;
    }
    let day: u8 = collect(&chars[start..day_end]).parse().ok()?;
    let year: i32 = collect(&chars[year_start..year_end]).parse().ok()?;
    let month = Month::try_from(u8::try_from(month_index + 1).ok()?).ok()?;
    Some((
        ScannedDate {
            raw: collect(&chars[start..year_end]),
            date: Date::from_calendar_date(year, month, day).ok(),
        },
        year_end,
    ))
}

/// `2026-04-30`.
fn iso_date_at(chars: &[char], start: usize) -> Option<(ScannedDate, usize)> {
    let year_end = digit_run(chars, start);
    if year_end - start != 4 || chars.get(year_end) != Some(&'-') {
        return None;
    }
    let month_start = year_end + 1;
    let month_end = digit_run(chars, month_start);
    if month_end - month_start != 2 || chars.get(month_end) != Some(&'-') {
        return None;
    }
    let day_start = month_end + 1;
    let day_end = digit_run(chars, day_start);
    if day_end - day_start != 2 {
        return None;
    }
    let year: i32 = collect(&chars[start..year_end]).parse().ok()?;
    let month: u8 = collect(&chars[month_start..month_end]).parse().ok()?;
    let day: u8 = collect(&chars[day_start..day_end]).parse().ok()?;
    let date = Month::try_from(month)
        .ok()
        .and_then(|month| Date::from_calendar_date(year, month, day).ok());
    Some((
        ScannedDate {
            raw: collect(&chars[start..day_end]),
            date,
        },
        day_end,
    ))
}
