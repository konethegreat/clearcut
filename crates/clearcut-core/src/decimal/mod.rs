// Written by Kone Tshivhinda & Claude
//! Reading amounts exactly, from gold-set labels and from what a model read.
//!
//! People write `1500.00`, not `150000`, so labels hold decimal **text**. This
//! module turns that text into integer minor units (cents) or thousandths
//! without ever passing through a floating-point number (ADR-0002). It is
//! strict on purpose: a label is the truth the system is graded against, so
//! anything ambiguous (`1,500`, `1.005`, `01.5`, `-0`) is rejected, never
//! rounded or guessed at.
//!
//! Errors say what kind of mistake it was, never what the text was, because the
//! text came from a real document.

use crate::{Currency, Money};

/// What was wrong with a piece of decimal text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DecimalError {
    #[error("the value is empty")]
    Empty,
    #[error("the value has a character other than digits, one dot and a leading minus")]
    BadCharacter,
    #[error("a digit is missing before or after the dot, or after the minus")]
    MissingDigits,
    #[error("the whole part has a leading zero")]
    LeadingZero,
    #[error("the value has more than {max} decimal places")]
    TooManyDecimals { max: u32 },
    #[error("negative zero is not allowed; write 0")]
    NegativeZero,
    #[error("the value is too large")]
    OutOfRange,
}

/// Parses `text` as a decimal with at most `decimals` places and returns it
/// multiplied by 10^`decimals`: `"12.30"` with 2 places is `1230`.
///
/// Rules: an optional leading `-`; one or more ASCII digits; optionally a dot
/// followed by one to `decimals` digits. No spaces, no `+`, no thousands
/// separators, no exponent, no leading zeros (except a single `0`), no `-0`.
/// The result must fit an `i64` and is never `i64::MIN`.
pub fn parse_scaled(text: &str, decimals: u32) -> Result<i64, DecimalError> {
    if text.is_empty() {
        return Err(DecimalError::Empty);
    }
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let (whole, fraction) = match digits.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (digits, None),
    };

    if whole.is_empty() {
        return Err(DecimalError::MissingDigits);
    }
    if !whole.bytes().all(|b| b.is_ascii_digit()) {
        return Err(DecimalError::BadCharacter);
    }
    if whole.len() > 1 && whole.starts_with('0') {
        return Err(DecimalError::LeadingZero);
    }
    if let Some(fraction) = fraction {
        if fraction.is_empty() {
            return Err(DecimalError::MissingDigits);
        }
        if !fraction.bytes().all(|b| b.is_ascii_digit()) {
            return Err(DecimalError::BadCharacter);
        }
        if fraction.len() > decimals as usize {
            return Err(DecimalError::TooManyDecimals { max: decimals });
        }
    }

    // Build the magnitude digit by digit in a wide integer; `checked_*` turns
    // an overflow into an error instead of wrapping or panicking.
    let fraction = fraction.unwrap_or("");
    let mut magnitude: u128 = 0;
    for digit in whole.bytes().chain(fraction.bytes()) {
        magnitude = magnitude
            .checked_mul(10)
            .and_then(|m| m.checked_add(u128::from(digit - b'0')))
            .ok_or(DecimalError::OutOfRange)?;
    }
    // Pad a short fraction ("1.5" with 2 places is 150).
    for _ in fraction.len()..decimals as usize {
        magnitude = magnitude.checked_mul(10).ok_or(DecimalError::OutOfRange)?;
    }

    if magnitude == 0 && negative {
        return Err(DecimalError::NegativeZero);
    }
    let magnitude = i64::try_from(magnitude).map_err(|_| DecimalError::OutOfRange)?;
    Ok(if negative { -magnitude } else { magnitude })
}

/// An amount in `currency`, in minor units. Every supported currency has two
/// decimal places.
pub fn parse_money(text: &str, currency: Currency) -> Result<Money, DecimalError> {
    Ok(Money::new(parse_scaled(text, 2)?, currency))
}

/// A quantity in thousandths (`"2.5"` is `2500`).
pub fn parse_quantity_milli(text: &str) -> Result<i64, DecimalError> {
    parse_scaled(text, 3)
}

/// Money in minor units as decimal text with exactly two decimals
/// (`150000` is `"1500.00"`). The inverse of [`parse_money`].
pub fn format_money(minor: i64) -> String {
    format_scaled(minor, 2, false)
}

/// A quantity in thousandths as decimal text without trailing zeros
/// (`2500` is `"2.5"`, `10000` is `"10"`). The inverse of [`parse_quantity_milli`].
pub fn format_quantity(milli: i64) -> String {
    format_scaled(milli, 3, true)
}

fn format_scaled(value: i64, decimals: u32, trim: bool) -> String {
    // `unsigned_abs` is defined for every i64, including i64::MIN, where a
    // plain `-value` would overflow.
    let magnitude = value.unsigned_abs();
    let scale = 10u64.pow(decimals);
    let (whole, fraction) = (magnitude / scale, magnitude % scale);
    let sign = if value < 0 { "-" } else { "" };
    let mut fraction = format!("{fraction:0width$}", width = decimals as usize);
    if trim {
        fraction = fraction.trim_end_matches('0').to_string();
    }
    if fraction.is_empty() {
        format!("{sign}{whole}")
    } else {
        format!("{sign}{whole}.{fraction}")
    }
}

#[cfg(test)]
mod tests;
