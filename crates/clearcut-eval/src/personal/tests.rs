// Written by Kone Tshivhinda & Claude
use super::*;
use crate::label::tests::base;
use crate::label::Label;
use serde_json::json;

/// Adds the Luhn check digit to 12 digits, to make a well-formed 13-digit id.
fn with_check_digit(twelve: &str) -> String {
    let mut sum = 0;
    // The check digit goes last, so the rightmost of the twelve is doubled.
    for (i, c) in twelve.chars().rev().enumerate() {
        let mut d = c.to_digit(10).unwrap();
        if i % 2 == 0 {
            d *= 2;
            if d > 9 {
                d -= 9;
            }
        }
        sum += d;
    }
    format!("{twelve}{}", (10 - sum % 10) % 10)
}

#[test]
fn a_well_formed_sa_id_number_is_found() {
    let id = with_check_digit("800101500908");
    assert_eq!(id.len(), 13);
    assert_eq!(
        scan(&format!("Contact: {id}.")),
        [PersonalData::SaIdNumber, PersonalData::LongNumber]
    );
    assert_eq!(scan(&id), [PersonalData::SaIdNumber, PersonalData::LongNumber]);
}

#[test]
fn thirteen_digits_that_are_not_an_id_are_only_a_long_number() {
    let id = with_check_digit("800101500908");
    // Break the check digit.
    let last = id.chars().last().unwrap().to_digit(10).unwrap();
    let broken = format!("{}{}", &id[..12], (last + 1) % 10);
    assert_eq!(scan(&broken), [PersonalData::LongNumber]);
    // Month 13 is not a date.
    let bad_month = with_check_digit("801301500908");
    assert_eq!(scan(&bad_month), [PersonalData::LongNumber]);
    // Day 00 is not a date.
    let bad_day = with_check_digit("800100500908");
    assert_eq!(scan(&bad_day), [PersonalData::LongNumber]);
}

#[test]
fn email_addresses_are_found() {
    for text in [
        "jane@example.com",
        "write to jane.doe+x@mail.example.co.za now",
        "a@b.c",
    ] {
        assert_eq!(scan(text), [PersonalData::EmailAddress], "{text}");
    }
    for text in [
        "",
        "no at sign",
        "@",
        "a @ b",
        "price @ 5.00",
        "x@",
        "@y.com",
        "user@host",
        "a@.b",
    ] {
        assert!(scan(text).is_empty(), "{text}");
    }
}

#[test]
fn long_digit_runs_are_found_even_when_grouped() {
    for text in [
        "123456789",
        "0821234567",
        "082 123 4567",
        "082-123-4567",
        "1234 5678 9012",
        "acc 62012345678 cheque",
    ] {
        assert_eq!(scan(text), [PersonalData::LongNumber], "{text}");
    }
}

#[test]
fn ordinary_numbers_in_text_are_left_alone() {
    for text in [
        "12345678",
        "Order 2026-03-31 due 2026-04-30",
        "Invoice period 01/03/2026 to 31/03/2026",
        "12 Main Rd, Rosebank, 2196",
        "Unit 4, 15 000 units at 12 each",
        "Section 7 and 12",
        "1 2 3 4 5 6 7 8 9 10",
        "R15 000 000",
    ] {
        assert!(scan(text).is_empty(), "{text}");
    }
}

#[test]
fn several_kinds_are_listed_once_each_in_a_fixed_order() {
    let id = with_check_digit("800101500908");
    let text = format!("{id} a@b.co 0821234567 0827654321");
    assert_eq!(
        scan(&text),
        [
            PersonalData::SaIdNumber,
            PersonalData::EmailAddress,
            PersonalData::LongNumber
        ]
    );
}

#[test]
fn odd_text_never_panics() {
    let long_digits = "9".repeat(10_000);
    let long_ats = "@.".repeat(500);
    for text in [
        "\u{0}",
        "é@é.é",
        "🙂@🙂.🙂",
        "٣٤٥٦٧٨٩٠١٢٣٤٥٦",
        &long_digits,
        &long_ats,
        "1-2-3-4-5-6-7-8-9-0",
    ] {
        let _ = scan(text);
    }
}

fn label_with(pointer: &str, text: &str) -> Label {
    let mut value = base();
    *value.pointer_mut(pointer).unwrap() = json!(text);
    Label::parse(value.to_string().as_bytes()).unwrap()
}

#[test]
fn the_example_label_has_no_personal_data() {
    let label = Label::parse(base().to_string().as_bytes()).unwrap();
    assert!(personal_data_in(&label.truth).is_empty());
}

#[test]
fn every_free_text_field_is_scanned_and_named() {
    let cases = [
        ("/truth/supplier/value/name", "supplier.name"),
        ("/truth/supplier/value/address", "supplier.address"),
        ("/truth/recipient/value/name", "recipient.name"),
        ("/truth/recipient/value/address", "recipient.address"),
        ("/truth/lines/0/description", "lines[0].description"),
        ("/truth/due/terms", "due.terms"),
    ];
    for (pointer, field) in cases {
        let label = label_with(pointer, "call 0821234567 or a@b.co");
        assert_eq!(
            personal_data_in(&label.truth),
            [
                (field.to_string(), PersonalData::EmailAddress),
                (field.to_string(), PersonalData::LongNumber)
            ],
            "{pointer}"
        );
    }
}

#[test]
fn identifiers_that_are_long_numbers_by_nature_are_not_scanned() {
    // VAT numbers (10 digits) and invoice numbers legitimately look like this.
    let mut value = base();
    value["truth"]["invoice_number"]["value"] = json!("202603310001234");
    value["truth"]["supplier"]["value"]["vat_number"] = json!("4999900001");
    value["truth"]["credit_note"] =
        json!({ "original_invoice_number": "000000123456789", "amount": "1.00", "page": 1 });
    let label = Label::parse(value.to_string().as_bytes()).unwrap();
    assert!(personal_data_in(&label.truth).is_empty());
}

#[test]
fn the_report_names_the_kind_but_never_the_text() {
    let label = label_with("/truth/supplier/value/address", "secret 0821234567");
    let found = personal_data_in(&label.truth);
    let shown = format!("{found:?} {}", PersonalData::LongNumber);
    assert!(
        !shown.contains("secret") && !shown.contains("0821234567"),
        "{shown}"
    );
}

#[test]
fn a_widely_published_example_id_number_is_recognised() {
    // Written independently of the test helper above, to check the checksum
    // against a number that was not made by this code.
    assert!(scan("8001015009087").contains(&PersonalData::SaIdNumber));
    assert!(!scan("8001015009088").contains(&PersonalData::SaIdNumber));
}

#[test]
fn every_well_formed_id_is_recognised_whatever_its_digits() {
    // Many ids, so that every digit appears in every position the checksum doubles.
    for n in 0..300u32 {
        let id = with_check_digit(&format!("800101{:06}", n * 3331 % 1_000_000));
        assert!(scan(&id).contains(&PersonalData::SaIdNumber), "{id}");
    }
}

#[test]
fn numbers_split_into_short_pieces_or_by_wide_gaps_are_not_phone_numbers() {
    for text in [
        "12 345 678 90",
        "123 456 78 90",
        "123  456  789",
        "123 , 456 , 789",
        "123456 / 7890",
        "12 3456 789",
        "1234 567 89",
    ] {
        assert!(scan(text).is_empty(), "{text}");
    }
}
