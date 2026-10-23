// Written by Kone Tshivhinda & Claude
use super::*;

/// Valid South African ID numbers (made up: valid date and check digit).
const ID_1: &str = "8001015009087";
const ID_2: &str = "7512315012085";

fn kinds(text: &str) -> Vec<(RedactionKind, &str)> {
    scan(text)
        .iter()
        .map(|s| (s.kind, &text[s.start..s.end]))
        .collect()
}

fn none(text: &str) {
    assert_eq!(kinds(text), vec![], "{text:?} should not be redacted");
}

fn one(text: &str, kind: RedactionKind, value: &str) {
    assert_eq!(kinds(text), vec![(kind, value)], "{text:?}");
}

// ---------- South African ID numbers ----------

#[test]
fn a_valid_id_number_is_found() {
    one(&format!("ID No {ID_1}."), RedactionKind::SaIdNumber, ID_1);
    one(ID_2, RedactionKind::SaIdNumber, ID_2);
    one(&format!("({ID_1})"), RedactionKind::SaIdNumber, ID_1);
}

#[test]
fn a_thirteen_digit_number_that_is_not_an_id_is_left_alone() {
    none("8001015009088"); // wrong check digit
    none("8013015009087"); // month 13
    none("8001325009087"); // day 32
    none("8002305009087"); // 30 February
    none("8001015009387"); // citizenship digit 3
    none("800101500908"); // 12 digits
    none("80010150090871"); // 14 digits
}

#[test]
fn an_id_number_inside_a_longer_token_is_left_alone() {
    none(&format!("REF{ID_1}"));
    none(&format!("{ID_1}X"));
    none(&format!("1{ID_1}"));
}

// ---------- cards ----------

#[test]
fn card_numbers_with_a_known_prefix_and_a_good_check_digit_are_found() {
    one(
        "Card 4111111111111111 expires",
        RedactionKind::CardNumber,
        "4111111111111111",
    );
    one(
        "4111 1111 1111 1111",
        RedactionKind::CardNumber,
        "4111 1111 1111 1111",
    );
    one(
        "4111-1111-1111-1111",
        RedactionKind::CardNumber,
        "4111-1111-1111-1111",
    );
    one("5555555555554444", RedactionKind::CardNumber, "5555555555554444");
    one("378282246310005", RedactionKind::CardNumber, "378282246310005");
    one(
        "3782 822463 10005",
        RedactionKind::CardNumber,
        "3782 822463 10005",
    );
    one("6011111111111117", RedactionKind::CardNumber, "6011111111111117");
    one("3056930009020004", RedactionKind::CardNumber, "3056930009020004");
}

#[test]
fn numbers_that_only_look_like_cards_are_left_alone() {
    none("4111111111111112"); // check digit
    none("1111111111111117"); // no card network starts with 1
    none("411111111111111"); // 15 digits is not a Visa length
    none("4111 1111 111 1111"); // odd grouping
    none("REF4111111111111111");
    none("41111111111111111"); // 17 digits
}

#[test]
fn table_rows_of_numbers_are_not_cards() {
    none("10 1500 15000 2250 17250 4111 1111");
    none("1 234 567 890 123 456");
}

// ---------- IBAN ----------

#[test]
fn iban_numbers_that_pass_the_mod_97_check_are_found() {
    one(
        "GB82WEST12345698765432",
        RedactionKind::Iban,
        "GB82WEST12345698765432",
    );
    one(
        "IBAN: GB82 WEST 1234 5698 7654 32 end",
        RedactionKind::Iban,
        "GB82 WEST 1234 5698 7654 32",
    );
    one(
        "DE89 3704 0044 0532 0130 00",
        RedactionKind::Iban,
        "DE89 3704 0044 0532 0130 00",
    );
}

#[test]
fn text_that_only_looks_like_an_iban_is_left_alone() {
    none("GB82WEST12345698765433"); // check
    none("AB12CD");
    none("gb82west12345698765432"); // lower case is not an IBAN
    none("XXGB82WEST12345698765432");
}

// ---------- labelled bank details ----------

#[test]
fn an_account_number_is_found_only_after_its_label() {
    one("Account No: 1234567890", RedactionKind::BankAccount, "1234567890");
    one("Acc no. 1234 567 890", RedactionKind::BankAccount, "1234 567 890");
    one(
        "ACCOUNT NUMBER 62012345678",
        RedactionKind::BankAccount,
        "62012345678",
    );
    one(
        "Bank account #: 9876543210",
        RedactionKind::BankAccount,
        "9876543210",
    );
    one("A/C 123456789", RedactionKind::BankAccount, "123456789");
    one(
        "account:1234567890\nnext line",
        RedactionKind::BankAccount,
        "1234567890",
    );
    none("1234567890");
    none("Invoice no: 1234567890");
    none("Account: John Smith");
    none("Account 12345"); // too short to be one
}

#[test]
fn a_branch_code_and_a_swift_code_are_found_after_their_labels() {
    one("Branch Code: 632005", RedactionKind::BranchCode, "632005");
    one("branch no 250655", RedactionKind::BranchCode, "250655");
    one("Sort code 20-00-00", RedactionKind::BranchCode, "20-00-00");
    none("Branch: Rosebank");
    none("632005");
    one("SWIFT: FIRNZAJJ", RedactionKind::SwiftCode, "FIRNZAJJ");
    one("BIC FIRNZAJJ123", RedactionKind::SwiftCode, "FIRNZAJJ123");
    none("SWIFT: our bank");
}

// ---------- e-mail and phone ----------

#[test]
fn an_email_address_is_found() {
    one(
        "write to accounts@example.co.za today",
        RedactionKind::Email,
        "accounts@example.co.za",
    );
    one(
        "(first.last+tag@sub.example.com)",
        RedactionKind::Email,
        "first.last+tag@sub.example.com",
    );
    none("a@b");
    none("user@localhost");
    none("@example.com");
    none("name @ example.com");
}

#[test]
fn a_phone_number_is_found_only_after_its_label() {
    one("Tel: 011 555 0100", RedactionKind::Phone, "011 555 0100");
    one("Cell +27 82 555 0123", RedactionKind::Phone, "+27 82 555 0123");
    one("Fax: (021) 555-0123", RedactionKind::Phone, "(021) 555-0123");
    one("Mobile no. 0825550123", RedactionKind::Phone, "0825550123");
    none("011 555 0100");
    none("Tel 12");
    none("Telephone: on request");
}

// ---------- what must never be touched ----------

#[test]
fn the_things_a_reading_needs_are_never_redacted() {
    for text in [
        "VAT No: 4123456789",
        "VAT Reg. No. 4987654321",
        "Invoice No: INV-000001",
        "Invoice Number 20260123456789",
        "Date: 2026-03-31",
        "Total R 18 250.00",
        "Net 16000.00 VAT 2250.00 Gross 18250.00",
        "Acme (Pty) Ltd, 1 Main Rd, Rosebank, 2196",
        "Order 4500012345 / PO 7788990011",
        "Due: 30 days from statement",
    ] {
        none(text);
    }
}

// ---------- replacing ----------

#[test]
fn a_redacted_text_has_fixed_placeholders_and_counts() {
    let text = format!("Acc No: 1234567890, ID {ID_1}, mail a@b.co, Acc No: 987654321");
    let redacted = redact(&text);
    assert_eq!(
        redacted.text,
        "Acc No: [REDACTED bank_account], ID [REDACTED sa_id_number], mail [REDACTED email], Acc No: [REDACTED bank_account]"
    );
    assert_eq!(redacted.spans.len(), 4);
    assert_eq!(redacted.count(RedactionKind::BankAccount), 2);
    assert_eq!(redacted.count(RedactionKind::SaIdNumber), 1);
    assert_eq!(redacted.count(RedactionKind::Email), 1);
    assert_eq!(redacted.count(RedactionKind::Phone), 0);
    assert_eq!(redacted.total(), 4);
}

#[test]
fn text_with_nothing_to_redact_comes_back_unchanged() {
    let text = "Consulting, October 2026: 10 x R1 500.00 = R15 000.00";
    let redacted = redact(text);
    assert_eq!(redacted.text, text);
    assert!(redacted.spans.is_empty());
    assert_eq!(redacted.total(), 0);
}

#[test]
fn the_placeholder_names_the_kind_and_holds_no_digits() {
    for kind in RedactionKind::ALL {
        let placeholder = kind.placeholder();
        assert!(placeholder.starts_with("[REDACTED ") && placeholder.ends_with(']'));
        assert!(placeholder.contains(kind.as_str()));
        assert!(!placeholder.bytes().any(|b| b.is_ascii_digit()), "{placeholder}");
    }
    assert_eq!(RedactionKind::ALL.len(), 8);
}

#[test]
fn redacting_twice_changes_nothing_more() {
    let text = format!("Account No: 1234567890 ID {ID_2} Tel: 011 555 0100 4111 1111 1111 1111 x@y.org");
    let once = redact(&text);
    let twice = redact(&once.text);
    assert_eq!(once.text, twice.text);
    assert!(twice.spans.is_empty());
}

#[test]
fn spans_are_in_the_original_text_in_order_and_do_not_overlap() {
    // The account label's value is also a valid ID number: one span, and the more specific kind wins.
    let text = format!("Account No: {ID_1}");
    assert_eq!(kinds(&text), vec![(RedactionKind::SaIdNumber, ID_1)]);

    let text = "Tel: 011 555 0100 Account No: 1234567890";
    let spans = scan(text);
    assert_eq!(spans.len(), 2);
    assert!(spans[0].end <= spans[1].start);
    assert_eq!(&text[spans[0].start..spans[0].end], "011 555 0100");
}

#[test]
fn non_ascii_text_around_a_match_is_kept_whole() {
    let text = "Café – Account No: 1234567890 — Straße, naïve €100";
    let redacted = redact(text);
    assert_eq!(
        redacted.text,
        "Café – Account No: [REDACTED bank_account] — Straße, naïve €100"
    );
    for span in scan(text) {
        assert!(text.is_char_boundary(span.start) && text.is_char_boundary(span.end));
    }
}

#[test]
fn no_input_makes_the_scanner_panic_or_return_nonsense() {
    // A deterministic spread of awkward strings: digits, labels, separators, multi-byte letters.
    let alphabet: Vec<&str> = vec![
        "0", "1", "4", "5", "8", "9", " ", "-", ".", ":", "#", "\n", "@", "+", "(", ")", "A", "Z", "a", "é",
        "€", "Account", "No", "Tel", "Branch", "SWIFT", "GB82", "4111", "800101", "5009087",
    ];
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..20_000 {
        let length = (next() % 30) as usize;
        let text: String = (0..length)
            .map(|_| alphabet[(next() % alphabet.len() as u64) as usize])
            .collect();
        let spans = scan(&text);
        let mut last_end = 0;
        for span in &spans {
            assert!(
                span.start >= last_end && span.start < span.end && span.end <= text.len(),
                "{text:?}"
            );
            assert!(
                text.is_char_boundary(span.start) && text.is_char_boundary(span.end),
                "{text:?}"
            );
            last_end = span.end;
        }
        let redacted = redact(&text);
        assert_eq!(redact(&redacted.text).text, redacted.text, "{text:?}");
    }
}

#[test]
fn the_edges_of_each_length_rule_are_exact() {
    // Bank accounts: 6 to 16 digits after the label.
    one("Account No: 123456", RedactionKind::BankAccount, "123456");
    one(
        "Account No: 1234567890123456",
        RedactionKind::BankAccount,
        "1234567890123456",
    );
    none("Account No: 12345");
    none("Account No: 12345678901234567");
    // Branch codes: 5 to 8 digits.
    one("Branch Code: 12345", RedactionKind::BranchCode, "12345");
    one("Branch Code: 12345678", RedactionKind::BranchCode, "12345678");
    none("Branch Code: 1234");
    none("Branch Code: 123456789");
    // Phone numbers: 9 to 15 digits.
    one("Tel: 011 555 010", RedactionKind::Phone, "011 555 010");
    one(
        "Tel: +27 82 555 0123 456",
        RedactionKind::Phone,
        "+27 82 555 0123 456",
    );
    none("Tel: 011 555 01");
    none("Tel: 1234567890123456");
    // A leap day is a real birth date; a 13-digit Visa is a real card.
    one("0002295009084", RedactionKind::SaIdNumber, "0002295009084");
    one("4222222222222", RedactionKind::CardNumber, "4222222222222");
    // A 31st only in the months that have one.
    none("8004315009087");
    none("8006315009087");
}

#[test]
fn check_digits_are_checked_exactly_not_roughly() {
    // Off by five: a sum that is a multiple of five but not of ten.
    none("8001015009082");
    none("4111111111111116");
    // The 31st of a 30-day month, with a correct check digit.
    for id in ["8004315009088", "8006315009083", "8009315009087", "8011315009083"] {
        none(id);
    }
    // The citizenship digit may be 1 as well as 0.
    one("8001015009186", RedactionKind::SaIdNumber, "8001015009186");
    // Mastercard's 2-series ends at 2720.
    one("2720990000000007", RedactionKind::CardNumber, "2720990000000007");
    none("2721990000000006");
}

#[test]
fn long_ibans_and_long_phone_numbers_are_found_to_their_limit() {
    one(
        "FR1420041010050500013M02606",
        RedactionKind::Iban,
        "FR1420041010050500013M02606",
    );
    one(
        "FR14 2004 1010 0505 0001 3M02 606",
        RedactionKind::Iban,
        "FR14 2004 1010 0505 0001 3M02 606",
    );
    one(
        "Tel: +27 82 555 0123 4567",
        RedactionKind::Phone,
        "+27 82 555 0123 4567",
    );
}

#[test]
fn a_label_must_be_a_word_of_its_own() {
    none("Myaccount: 1234567890");
    none("Reacc no 1234567890");
    none("Subtel: 011 555 0100");
}
