// Written by Kone Tshivhinda & Claude
use super::*;
use crate::{Currency, Money};

fn money(text: &str) -> Result<i64, DecimalError> {
    parse_scaled(text, 2)
}

fn quantity(text: &str) -> Result<i64, DecimalError> {
    parse_scaled(text, 3)
}

#[test]
fn money_is_read_exactly_in_cents() {
    assert_eq!(money("0"), Ok(0));
    assert_eq!(money("1500.00"), Ok(150_000));
    assert_eq!(money("1500"), Ok(150_000));
    assert_eq!(money("1500.5"), Ok(150_050));
    assert_eq!(money("0.05"), Ok(5));
    assert_eq!(money("0.00"), Ok(0));
    assert_eq!(money("-12.30"), Ok(-1_230));
    assert_eq!(money("19.99"), Ok(1_999));
}

#[test]
fn quantities_are_read_exactly_in_thousandths() {
    assert_eq!(quantity("10"), Ok(10_000));
    assert_eq!(quantity("2.5"), Ok(2_500));
    assert_eq!(quantity("0.001"), Ok(1));
    assert_eq!(quantity("12.345"), Ok(12_345));
}

#[test]
fn more_decimals_than_allowed_is_rejected_not_rounded() {
    assert_eq!(money("1.005"), Err(DecimalError::TooManyDecimals { max: 2 }));
    assert_eq!(money("1.000"), Err(DecimalError::TooManyDecimals { max: 2 }));
    assert_eq!(quantity("1.0005"), Err(DecimalError::TooManyDecimals { max: 3 }));
}

#[test]
fn only_plain_ascii_digits_a_dot_and_a_leading_minus_are_allowed() {
    for bad in [
        "", "-", ".5", "5.", "-.5", "1,500.00", "1 500.00", " 5", "5 ", "R100", "1e3", "+5", "1_000", "--5",
        "5-", "1.2.3", "٣٤", "１２", "0x10", "NaN", "inf",
    ] {
        assert!(money(bad).is_err(), "{bad:?} should be rejected");
    }
}

#[test]
fn a_bad_character_after_the_dot_is_reported_as_one() {
    for bad in ["1.5x", "1.x", "1.٣", "1.5 ", "1.-5"] {
        assert_eq!(money(bad), Err(DecimalError::BadCharacter), "{bad:?}");
    }
}

#[test]
fn error_kinds_are_specific() {
    assert_eq!(money(""), Err(DecimalError::Empty));
    assert_eq!(money("-"), Err(DecimalError::MissingDigits));
    assert_eq!(money(".5"), Err(DecimalError::MissingDigits));
    assert_eq!(money("5."), Err(DecimalError::MissingDigits));
    assert_eq!(money("1,5"), Err(DecimalError::BadCharacter));
    assert_eq!(money("01.00"), Err(DecimalError::LeadingZero));
    assert_eq!(money("00"), Err(DecimalError::LeadingZero));
}

#[test]
fn zero_is_written_one_way_only() {
    assert_eq!(money("-0"), Err(DecimalError::NegativeZero));
    assert_eq!(money("-0.00"), Err(DecimalError::NegativeZero));
    assert_eq!(money("0.00"), Ok(0));
    // A leading zero before a fraction is fine.
    assert_eq!(money("0.50"), Ok(50));
    assert_eq!(money("-0.50"), Ok(-50));
}

#[test]
fn the_i64_limits_are_enforced_without_wrapping() {
    assert_eq!(money("92233720368547758.07"), Ok(i64::MAX));
    assert_eq!(money("92233720368547758.08"), Err(DecimalError::OutOfRange));
    assert_eq!(money("-92233720368547758.07"), Ok(-i64::MAX));
    // i64::MIN is not representable as a magnitude here; core never needs it.
    assert_eq!(money("-92233720368547758.08"), Err(DecimalError::OutOfRange));
    assert_eq!(
        money("99999999999999999999999999999999"),
        Err(DecimalError::OutOfRange)
    );
}

#[test]
fn money_carries_its_currency() {
    assert_eq!(
        parse_money("10.50", Currency::Usd),
        Ok(Money::new(1_050, Currency::Usd))
    );
    assert_eq!(parse_money("x", Currency::Zar), Err(DecimalError::BadCharacter));
}

#[test]
fn parse_quantity_milli_reads_thousandths() {
    assert_eq!(parse_quantity_milli("1.5"), Ok(1_500));
}

#[test]
fn odd_input_never_panics() {
    let mut text = String::new();
    for byte in 0u8..=127 {
        text.push(byte as char);
        let _ = money(&text);
        let _ = quantity(&text);
    }
    for odd in [
        "\u{0}",
        "9".repeat(5000).as_str(),
        "-".repeat(10).as_str(),
        "\u{202e}5",
        "5\n",
    ] {
        assert!(money(odd).is_err());
    }
}

#[test]
fn error_text_never_echoes_the_input() {
    for bad in ["R1500secret", "1.00500secret", "secret"] {
        let message = money(bad).unwrap_err().to_string();
        assert!(!message.contains("secret"), "{message}");
    }
}

#[test]
fn money_is_written_with_exactly_two_decimals() {
    assert_eq!(format_money(150_000), "1500.00");
    assert_eq!(format_money(0), "0.00");
    assert_eq!(format_money(5), "0.05");
    assert_eq!(format_money(50), "0.50");
    assert_eq!(format_money(-1_230), "-12.30");
    assert_eq!(format_money(-5), "-0.05");
    assert_eq!(format_money(i64::MAX), "92233720368547758.07");
    assert_eq!(format_money(-i64::MAX), "-92233720368547758.07");
    // i64::MIN has no positive twin; it must not panic.
    assert_eq!(format_money(i64::MIN), "-92233720368547758.08");
}

#[test]
fn quantities_are_written_without_trailing_zeros() {
    assert_eq!(format_quantity(10_000), "10");
    assert_eq!(format_quantity(2_500), "2.5");
    assert_eq!(format_quantity(1), "0.001");
    assert_eq!(format_quantity(500), "0.5");
    assert_eq!(format_quantity(12_345), "12.345");
    assert_eq!(format_quantity(0), "0");
    assert_eq!(format_quantity(-1_500), "-1.5");
}

#[test]
fn what_is_written_reads_back_to_the_same_number() {
    let mut samples = vec![
        0,
        1,
        -1,
        99,
        100,
        101,
        999,
        1_000,
        123_456_789,
        -987_654_321,
        i64::MAX,
        -i64::MAX,
    ];
    samples.extend((0..2000).map(|n| n * 7919 - 5000));
    for n in samples {
        assert_eq!(parse_scaled(&format_money(n), 2), Ok(n), "money {n}");
        assert_eq!(parse_quantity_milli(&format_quantity(n)), Ok(n), "quantity {n}");
    }
}
