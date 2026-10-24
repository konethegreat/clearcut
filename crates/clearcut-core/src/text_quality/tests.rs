// Written by Kone Tshivhinda & Claude
use super::*;

const GOOD: &str = "TAX INVOICE\nInvoice No: INV-2024-001\nDate: 2024-03-01\n\
                    Description Quantity Price Amount\nTotal R1,150.00 VAT R150.00";

fn font(kind: FontKind, has_to_unicode: bool, encoding: FontEncoding) -> FontFact {
    FontFact {
        kind,
        has_to_unicode,
        encoding,
        embedded: true,
    }
}

fn risky() -> FontFact {
    font(FontKind::Type3, false, FontEncoding::None)
}

fn plain() -> FontFact {
    font(FontKind::Type1, false, FontEncoding::WinAnsi)
}

fn assess_with(text: &str, fonts: &[FontFact], agreement: Option<u16>) -> QualityAssessment {
    assess(&TextEvidence {
        text,
        fonts,
        ocr_agreement: agreement,
    })
}

fn verdict_of(text: &str, fonts: &[FontFact], agreement: Option<u16>) -> TextVerdict {
    assess_with(text, fonts, agreement).verdict
}

// ---------- counting characters ----------

#[test]
fn nothing_is_measured_as_nothing() {
    let m = measure("");
    assert_eq!(m, TextMeasures::default());
    assert_eq!(m.characters, 0);
    assert_eq!(m.letters_per_mille(), 0);
    assert_eq!(m.vowelless_per_mille(), 0);
}

#[test]
fn white_space_is_not_counted() {
    assert_eq!(measure("a b\n\tc \r\n").characters, 3);
    assert_eq!(measure("   \n\t ").characters, 0);
}

#[test]
fn letters_digits_and_punctuation_are_shares_in_thousandths_of_the_characters() {
    let m = measure("ab12");
    assert_eq!(m.characters, 4);
    assert_eq!(
        (
            m.letters_per_mille(),
            m.digits_per_mille(),
            m.punctuation_per_mille()
        ),
        (500, 500, 0)
    );
    let m = measure("a.,;");
    assert_eq!(
        (
            m.letters_per_mille(),
            m.digits_per_mille(),
            m.punctuation_per_mille()
        ),
        (250, 0, 750)
    );
    // R, six digits, a comma and a point: shares are rounded down.
    let m = measure("R1,150.00");
    assert_eq!(
        (
            m.letters_per_mille(),
            m.digits_per_mille(),
            m.punctuation_per_mille()
        ),
        (111, 666, 222)
    );
}

#[test]
fn accented_letters_currency_signs_and_fractions_are_sorted_sensibly() {
    let m = measure("é€½");
    assert_eq!(m.characters, 3);
    assert_eq!(m.letters_per_mille(), 333); // é
    assert_eq!(m.punctuation_per_mille(), 333); // €
    assert_eq!(m.digits_per_mille(), 333); // ½
    for sign in ['$', '£', '¥', '%', '+', '«', '–', '—', '“'] {
        assert_eq!(measure(&sign.to_string()).punctuation_per_mille(), 1000, "{sign}");
    }
}

#[test]
fn replacement_and_private_use_characters_are_counted() {
    assert_eq!(measure("ab\u{FFFD}d").replacement_or_private_use, 1);
    for c in [
        '\u{E000}',
        '\u{F8FF}',
        '\u{F0000}',
        '\u{FFFFD}',
        '\u{100000}',
        '\u{10FFFD}',
    ] {
        assert_eq!(measure(&format!("a{c}")).replacement_or_private_use, 1, "{c:?}");
    }
    for c in ['\u{D7FF}', '\u{F900}', '\u{FFFF}', '\u{1F600}', 'a', 'é'] {
        assert_eq!(measure(&c.to_string()).replacement_or_private_use, 0, "{c:?}");
    }
}

#[test]
fn control_characters_are_counted_apart_from_white_space() {
    let m = measure("a\u{1}b\u{7f}c\u{9f}\n\t");
    assert_eq!(m.control, 3);
    assert_eq!(m.characters, 6);
    assert_eq!(measure("\u{85}x").control, 0, "NEL is white space");
}

// ---------- words ----------

#[test]
fn words_of_three_letters_or_more_without_a_vowel_are_counted() {
    let m = measure("BTW Ltd rhythm");
    assert_eq!(m.words, 3);
    assert_eq!(m.vowelless_words, 2, "y counts as a vowel, so rhythm is fine");
    assert_eq!(m.vowelless_per_mille(), 666);
}

#[test]
fn short_words_and_digits_do_not_make_words() {
    assert_eq!(measure("no to by").words, 0);
    assert_eq!(measure("ab1cd").words, 0, "digits end a word: ab and cd");
    assert_eq!(measure("INV-2024-001").words, 1);
    assert_eq!(measure("R1500.00").words, 0);
}

#[test]
fn accented_vowels_and_capitals_are_vowels() {
    for word in [
        "bêre", "ÉÉÉ", "Weiß", "naïef", "faktuur", "INVOICE", "œuvre", "Zoë",
    ] {
        let m = measure(word);
        assert_eq!((m.words, m.vowelless_words), (1, 0), "{word}");
    }
}

#[test]
fn words_in_other_scripts_are_not_judged_by_latin_vowels() {
    for word in ["привет", "αβγδ", "日本語", "שלום"] {
        assert_eq!(measure(word).words, 0, "{word}");
    }
}

#[test]
fn a_capital_inside_a_word_is_counted() {
    let m = measure("iNvOicE Invoice INVOICE invoice");
    assert_eq!(m.words, 4);
    assert_eq!(m.mid_word_capital_words, 1);
    assert_eq!(m.mid_word_capital_per_mille(), 250);
    // McDonald and eBay are written that way on purpose, and are counted all the same:
    // this is a measure, and one word does not decide anything.
    assert_eq!(measure("McDonald eBay").mid_word_capital_words, 2);
    assert_eq!(measure("ab").mid_word_capital_words, 0);
}

#[test]
fn the_longest_unbroken_run_is_measured_in_characters() {
    assert_eq!(measure("a bb cccc").longest_token, 4);
    assert_eq!(measure(&"é".repeat(50)).longest_token, 50);
    assert_eq!(measure("x--------y").longest_token, 10);
    assert_eq!(measure("").longest_token, 0);
    assert_eq!(measure("  \n ").longest_token, 0);
}

// ---------- keywords ----------

#[test]
fn the_keywords_are_the_invoice_words_in_english_and_afrikaans() {
    let mut words: Vec<&str> = KEYWORDS.to_vec();
    words.sort_unstable();
    assert_eq!(
        words,
        [
            "amount",
            "bedrag",
            "beskrywing",
            "btw",
            "date",
            "datum",
            "description",
            "due",
            "faktuur",
            "hoeveelheid",
            "invoice",
            "price",
            "prys",
            "quantity",
            "tax",
            "totaal",
            "total",
            "vat",
            "vervaldatum",
        ]
    );
}

#[test]
fn keyword_hits_are_distinct_whole_words_whatever_the_case_or_punctuation() {
    assert_eq!(measure("Invoice invoice INVOICE").keyword_hits, 1);
    assert_eq!(measure("TOTAAL Datum").keyword_hits, 2);
    assert_eq!(measure("Total: R100 (VAT)").keyword_hits, 2);
    assert_eq!(measure("due-date").keyword_hits, 2);
    assert_eq!(measure("invoices subtotal taxes").keyword_hits, 0);
    assert_eq!(measure("").keyword_hits, 0);
    assert_eq!(measure(&KEYWORDS.join(" ")).keyword_hits, KEYWORDS.len());
}

// ---------- fonts ----------

#[test]
fn a_font_is_risky_when_its_characters_cannot_be_mapped_back_to_text() {
    use FontEncoding as E;
    use FontKind as K;
    let cases = [
        (K::Type3, false, E::None, true),
        (K::Type3, true, E::WinAnsi, true),
        (K::Type0, false, E::Identity, true),
        (K::Type0, true, E::Identity, false),
        (K::TrueType, false, E::Custom, true),
        (K::TrueType, true, E::Custom, false),
        (K::Type0, false, E::Custom, true),
        (K::Type1, false, E::WinAnsi, false),
        (K::Type1, false, E::None, false),
        (K::TrueType, false, E::MacRoman, false),
        (K::TrueType, false, E::Standard, false),
        (K::Other, false, E::Other, false),
        (K::Type1, false, E::Identity, true),
    ];
    for (kind, unicode, encoding, expected) in cases {
        assert_eq!(
            font(kind, unicode, encoding).is_risky(),
            expected,
            "{kind:?} {unicode} {encoding:?}"
        );
    }
}

#[test]
fn a_font_embedded_or_not_is_risky_by_the_same_rule() {
    let mut f = risky();
    f.embedded = false;
    assert!(f.is_risky());
}

#[test]
fn the_workers_font_facts_are_read_as_they_are_written() {
    let f: FontFact = serde_json::from_str(
        r#"{"kind":"type0","has_to_unicode":false,"encoding":"identity","embedded":true}"#,
    )
    .unwrap();
    assert_eq!(f, font(FontKind::Type0, false, FontEncoding::Identity));
    for (name, kind) in [
        ("type1", FontKind::Type1),
        ("truetype", FontKind::TrueType),
        ("type0", FontKind::Type0),
        ("type3", FontKind::Type3),
        ("other", FontKind::Other),
    ] {
        let json = format!(r#"{{"kind":"{name}","has_to_unicode":true,"encoding":"none","embedded":false}}"#);
        assert_eq!(serde_json::from_str::<FontFact>(&json).unwrap().kind, kind);
    }
    for (name, encoding) in [
        ("none", FontEncoding::None),
        ("winansi", FontEncoding::WinAnsi),
        ("macroman", FontEncoding::MacRoman),
        ("standard", FontEncoding::Standard),
        ("identity", FontEncoding::Identity),
        ("custom", FontEncoding::Custom),
        ("other", FontEncoding::Other),
    ] {
        let json =
            format!(r#"{{"kind":"type1","has_to_unicode":true,"encoding":"{name}","embedded":false}}"#);
        assert_eq!(
            serde_json::from_str::<FontFact>(&json).unwrap().encoding,
            encoding
        );
    }
}

#[test]
fn font_facts_that_are_not_what_the_worker_promised_are_refused() {
    for json in [
        r#"{"kind":"type9","has_to_unicode":false,"encoding":"none","embedded":false}"#,
        r#"{"kind":"type1","has_to_unicode":false,"encoding":"rot13","embedded":false}"#,
        r#"{"kind":"type1","has_to_unicode":0,"encoding":"none","embedded":false}"#,
        r#"{"kind":"type1","has_to_unicode":false,"encoding":"none"}"#,
        r#"{"kind":"type1","has_to_unicode":false,"encoding":"none","embedded":false,"x":1}"#,
        r#"{"kind":"Type1","has_to_unicode":false,"encoding":"none","embedded":false}"#,
    ] {
        assert!(serde_json::from_str::<FontFact>(json).is_err(), "{json}");
    }
}

// ---------- the verdict ----------

#[test]
fn an_ordinary_invoice_with_ordinary_fonts_is_good() {
    let a = assess_with(GOOD, &[plain()], None);
    assert_eq!(a.verdict, TextVerdict::Good);
    assert!(a.reasons.is_empty());
    assert!(a.measures.keyword_hits >= 8);
}

#[test]
fn no_text_at_all_is_unusable_whatever_else_is_said() {
    for text in ["", "   ", "\n\t \r\n"] {
        let a = assess_with(text, &[], Some(1000));
        assert_eq!(a.verdict, TextVerdict::Unusable, "{text:?}");
        assert_eq!(a.reasons, [Reason::NoText]);
    }
}

#[test]
fn five_percent_replacement_characters_is_unusable_and_less_is_not() {
    let make = |bad: usize| format!("{}{}", "a".repeat(1000 - bad), "\u{FFFD}".repeat(bad));
    assert_eq!(verdict_of(&make(49), &[plain()], None), TextVerdict::Good);
    let a = assess_with(&make(50), &[plain()], None);
    assert_eq!(a.verdict, TextVerdict::Unusable);
    assert_eq!(a.reasons, [Reason::ReplacementCharacters]);
    // The same share written as one in twenty and one in twenty-one.
    assert_eq!(
        verdict_of(&format!("{}\u{FFFD}", "a".repeat(19)), &[], None),
        TextVerdict::Unusable
    );
    assert_eq!(
        verdict_of(&format!("{}\u{FFFD}", "a".repeat(20)), &[], None),
        TextVerdict::Good
    );
}

#[test]
fn private_use_characters_count_like_replacement_characters() {
    let text = format!("{}{}", "a".repeat(950), "\u{E000}".repeat(50));
    assert_eq!(verdict_of(&text, &[], None), TextVerdict::Unusable);
    let mixed = format!(
        "{}{}{}",
        "a".repeat(950),
        "\u{E000}".repeat(25),
        "\u{FFFD}".repeat(25)
    );
    assert_eq!(verdict_of(&mixed, &[], None), TextVerdict::Unusable);
    let plane = format!("{}{}", "a".repeat(950), "\u{F0001}".repeat(50));
    assert_eq!(verdict_of(&plane, &[], None), TextVerdict::Unusable);
}

#[test]
fn control_characters_alone_do_not_make_text_unusable() {
    let text = format!("{GOOD}{}", "\u{1}".repeat(30));
    assert_eq!(verdict_of(&text, &[plain()], None), TextVerdict::Good);
}

#[test]
fn what_ocr_saw_decides_by_the_agreement() {
    let cases = [
        (0, TextVerdict::Unusable, Reason::OcrDisagrees),
        (499, TextVerdict::Unusable, Reason::OcrDisagrees),
        (500, TextVerdict::NeedsCheck, Reason::OcrPartlyAgrees),
        (849, TextVerdict::NeedsCheck, Reason::OcrPartlyAgrees),
        (850, TextVerdict::Good, Reason::OcrAgrees),
        (1000, TextVerdict::Good, Reason::OcrAgrees),
        (1001, TextVerdict::Good, Reason::OcrAgrees),
        (u16::MAX, TextVerdict::Good, Reason::OcrAgrees),
    ];
    for (agreement, verdict, reason) in cases {
        let a = assess_with(GOOD, &[plain()], Some(agreement));
        assert_eq!(a.verdict, verdict, "{agreement}");
        assert_eq!(a.reasons, [reason], "{agreement}");
    }
}

#[test]
fn bad_characters_win_over_a_good_agreement_and_both_reasons_are_given() {
    let text = format!("{}{}", "a".repeat(900), "\u{FFFD}".repeat(100));
    let a = assess_with(&text, &[], Some(900));
    assert_eq!(a.verdict, TextVerdict::Unusable);
    assert_eq!(a.reasons, [Reason::ReplacementCharacters]);
    let both = assess_with(&text, &[], Some(100));
    assert_eq!(
        both.reasons,
        [Reason::ReplacementCharacters, Reason::OcrDisagrees]
    );
}

#[test]
fn a_risky_font_with_text_that_looks_wrong_needs_a_check() {
    let a = assess_with("hello there my friend", &[risky()], None);
    assert_eq!(a.verdict, TextVerdict::NeedsCheck);
    assert_eq!(a.reasons, [Reason::RiskyFont, Reason::NoKeywords]);
}

#[test]
fn many_words_without_vowels_are_weak_text_even_with_a_keyword() {
    let a = assess_with("total brr hmm tsk psst", &[risky()], None);
    assert_eq!(a.verdict, TextVerdict::NeedsCheck);
    assert_eq!(a.reasons, [Reason::RiskyFont, Reason::ManyVowellessWords]);
    let both = assess_with("zzz qqq vvv", &[risky()], None);
    assert_eq!(
        both.reasons,
        [Reason::RiskyFont, Reason::NoKeywords, Reason::ManyVowellessWords]
    );
}

#[test]
fn the_vowelless_limit_is_three_in_ten_and_four_in_ten_is_over_it() {
    let ten = |vowelless: &[&str], with: &[&str]| format!("{} {}", with.join(" "), vowelless.join(" "));
    let with7 = ["total", "alpha", "beta", "gamma", "delta", "omega", "sigma"];
    let three = ten(&["brr", "hmm", "tsk"], &with7);
    assert_eq!(measure(&three).vowelless_per_mille(), 300);
    assert_eq!(verdict_of(&three, &[risky()], None), TextVerdict::Good);
    let four = ten(&["brr", "hmm", "tsk", "psst"], &with7[..6]);
    assert_eq!(measure(&four).vowelless_per_mille(), 400);
    assert_eq!(verdict_of(&four, &[risky()], None), TextVerdict::NeedsCheck);
}

#[test]
fn one_keyword_is_enough_to_make_the_text_not_weak() {
    assert_eq!(verdict_of("total", &[risky()], None), TextVerdict::Good);
    assert_eq!(verdict_of("hello", &[risky()], None), TextVerdict::NeedsCheck);
}

#[test]
fn a_risky_font_with_good_text_is_good_and_weak_text_with_ordinary_fonts_is_too() {
    assert_eq!(verdict_of(GOOD, &[risky()], None), TextVerdict::Good);
    assert_eq!(
        verdict_of("hello there my friend", &[plain()], None),
        TextVerdict::Good
    );
    assert_eq!(verdict_of("hello there my friend", &[], None), TextVerdict::Good);
}

#[test]
fn it_is_enough_that_one_of_the_fonts_is_risky() {
    let fonts = [plain(), plain(), risky()];
    assert_eq!(verdict_of("hello there", &fonts, None), TextVerdict::NeedsCheck);
}

#[test]
fn a_high_agreement_settles_it_whatever_the_fonts_and_the_words_look_like() {
    assert_eq!(
        verdict_of("zzz qqq vvv", &[risky()], Some(850)),
        TextVerdict::Good
    );
    let middling = assess_with("zzz qqq vvv", &[risky()], Some(600));
    assert_eq!(middling.verdict, TextVerdict::NeedsCheck);
    assert_eq!(middling.reasons, [Reason::OcrPartlyAgrees]);
    assert_eq!(verdict_of(GOOD, &[plain()], Some(499)), TextVerdict::Unusable);
}

// ---------- what the answer holds ----------

#[test]
fn the_answer_carries_the_measures_and_never_the_text() {
    let a = assess_with("SECRET-INVOICE 12345", &[plain()], None);
    assert_eq!(a.measures.characters, 19);
    assert!(!format!("{a:?}").contains("SECRET"));
    assert!(!serde_json::to_string(&a).unwrap().contains("SECRET"));
}

#[test]
fn the_names_are_fixed_words() {
    let reasons = [
        (Reason::NoText, "no_text"),
        (Reason::ReplacementCharacters, "replacement_characters"),
        (Reason::OcrDisagrees, "ocr_disagrees"),
        (Reason::OcrPartlyAgrees, "ocr_partly_agrees"),
        (Reason::OcrAgrees, "ocr_agrees"),
        (Reason::RiskyFont, "risky_font"),
        (Reason::NoKeywords, "no_keywords"),
        (Reason::ManyVowellessWords, "many_vowelless_words"),
    ];
    for (reason, name) in reasons {
        assert_eq!(reason.as_str(), name);
        assert_eq!(serde_json::to_string(&reason).unwrap(), format!("\"{name}\""));
    }
    for (verdict, name) in [
        (TextVerdict::Good, "good"),
        (TextVerdict::NeedsCheck, "needs_check"),
        (TextVerdict::Unusable, "unusable"),
    ] {
        assert_eq!(verdict.as_str(), name);
        assert_eq!(serde_json::to_string(&verdict).unwrap(), format!("\"{name}\""));
    }
}

#[test]
fn the_thresholds_are_the_ones_the_design_names() {
    assert_eq!(UNUSABLE_BAD_CHARACTER_PERCENT, 5);
    assert_eq!(OCR_UNUSABLE_BELOW, 500);
    assert_eq!(OCR_GOOD_FROM, 850);
    assert_eq!(MANY_VOWELLESS_ABOVE_PER_MILLE, 300);
}

#[test]
fn the_same_evidence_always_gives_the_same_answer() {
    let evidence = TextEvidence {
        text: GOOD,
        fonts: &[risky(), plain()],
        ocr_agreement: Some(700),
    };
    assert_eq!(assess(&evidence), assess(&evidence));
}

// ---------- hostile and odd text ----------

#[test]
fn odd_text_never_breaks_the_measures() {
    // A fixed pseudo-random spread over letters, digits, marks, emoji, private use,
    // control characters and white space, so a failure can be replayed.
    let alphabet: Vec<char> = "aeiouAEIOUbcdZ09 .,;€½é\u{301}\u{200b}\u{FFFD}\u{E000}\u{F0000}\
                               \u{1}\u{7f}\n\t😀日本привет\u{10FFFF}"
        .chars()
        .collect();
    let mut state: u64 = 0x2545_F491_4F6C_DD1D;
    for _ in 0..300 {
        let mut text = String::new();
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let length = (state >> 33) % 120;
        for _ in 0..length {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            text.push(alphabet[((state >> 33) as usize) % alphabet.len()]);
        }
        let m = measure(&text);
        assert!(m.letters_per_mille() + m.digits_per_mille() + m.punctuation_per_mille() <= 1000);
        assert!(m.vowelless_words <= m.words);
        assert!(m.mid_word_capital_words <= m.words);
        assert!(m.keyword_hits <= KEYWORDS.len());
        assert!(m.replacement_or_private_use + m.control <= m.characters);
        assert!(m.longest_token <= text.chars().count());
        let a = assess_with(&text, &[risky()], Some((state >> 20) as u16));
        if m.replacement_or_private_use * 20 >= m.characters && m.characters > 0 {
            assert_eq!(a.verdict, TextVerdict::Unusable);
        }
        if m.characters == 0 {
            assert_eq!(a.reasons, [Reason::NoText]);
        }
    }
}

#[test]
fn a_page_of_a_megabyte_is_measured_in_one_pass() {
    let text = format!("{GOOD}\n").repeat(12_000);
    assert!(text.len() > 1_000_000);
    let started = std::time::Instant::now();
    let a = assess_with(&text, &[plain()], None);
    assert_eq!(a.verdict, TextVerdict::Good);
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
}

// ---------- scrambled text, as the design describes it ----------

/// The text with every letter replaced by the one thirteen places on: what a font that maps
/// glyphs to the wrong characters gives when copied.
fn scrambled(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            'a'..='z' => (((c as u8 - b'a') + 13) % 26 + b'a') as char,
            'A'..='Z' => (((c as u8 - b'A') + 13) % 26 + b'A') as char,
            other => other,
        })
        .collect()
}

/// The text with every digit replaced by a different digit: still digits, still plausible.
fn digits_swapped(text: &str) -> String {
    text.chars()
        .map(|c| match c.to_digit(10) {
            Some(d) => char::from_digit((d * 7 + 3) % 10, 10).unwrap(),
            None => c,
        })
        .collect()
}

#[test]
fn a_scrambled_alphabet_from_a_risky_font_needs_a_check() {
    let text = scrambled(GOOD);
    assert_eq!(measure(&text).keyword_hits, 0);
    let a = assess_with(&text, &[risky()], None);
    assert_eq!(a.verdict, TextVerdict::NeedsCheck);
    assert_eq!(a.reasons[0], Reason::RiskyFont);
    assert!(a.reasons.contains(&Reason::NoKeywords));
}

#[test]
fn the_same_scramble_is_settled_by_what_ocr_saw() {
    let text = scrambled(GOOD);
    assert_eq!(verdict_of(&text, &[risky()], Some(30)), TextVerdict::Unusable);
    assert_eq!(verdict_of(&text, &[risky()], Some(700)), TextVerdict::NeedsCheck);
}

#[test]
fn a_scramble_from_ordinary_looking_fonts_is_not_caught_by_the_text_alone() {
    // The design says so plainly: measures of the text are one layer, not the whole defence.
    // The page image, the totals re-derived to the cent and the OCR check are the others.
    assert_eq!(verdict_of(&scrambled(GOOD), &[plain()], None), TextVerdict::Good);
}

#[test]
fn swapped_digits_look_like_valid_text_and_only_ocr_can_tell() {
    let text = digits_swapped(GOOD);
    assert_ne!(text, GOOD);
    assert_eq!(measure(&text).keyword_hits, measure(GOOD).keyword_hits);
    assert_eq!(verdict_of(&text, &[risky()], None), TextVerdict::Good);
    // Digits swapped in four of nine distinct words: a fair share of the layer is wrong.
    assert_eq!(verdict_of(&text, &[risky()], Some(550)), TextVerdict::NeedsCheck);
    assert_eq!(verdict_of(&text, &[risky()], Some(400)), TextVerdict::Unusable);
}

// ---------- the edges of the character classes ----------

#[test]
fn the_first_and_last_character_of_every_punctuation_range_count_as_punctuation() {
    let edges = [
        '\u{A1}', '\u{BF}', '\u{D7}', '\u{F7}', '\u{2010}', '\u{2027}', '\u{2030}', '\u{205E}', '\u{20A0}',
        '\u{20CF}', '\u{3001}', '\u{303F}',
    ];
    for c in edges {
        let m = measure(&c.to_string());
        assert_eq!((m.characters, m.punctuation), (1, 1), "{c:?}");
    }
}

#[test]
fn characters_just_outside_the_punctuation_ranges_are_not_punctuation() {
    // Letters, a combining mark, a format character and a word-joining sign.
    for c in ['À', 'Ö', 'Ø', 'ö', 'ø', '\u{200F}', '\u{20D0}', '\u{3041}'] {
        assert_eq!(measure(&c.to_string()).punctuation, 0, "{c:?}");
    }
}

#[test]
fn white_space_that_looks_like_punctuation_is_still_white_space() {
    for c in ['\u{A0}', '\u{2028}', '\u{2029}', '\u{205F}', '\u{3000}'] {
        assert_eq!(measure(&c.to_string()).characters, 0, "{c:?}");
    }
}

#[test]
fn latin_letters_from_the_first_accented_one_to_the_end_of_latin_extended_b_are_judged() {
    for c in ['À', 'ÿ', 'ŝ', 'Ǆ', 'ɏ'] {
        let word: String = std::iter::repeat_n(c, 3).collect();
        assert_eq!(measure(&word).words, 1, "{c:?}");
    }
    // The next block is phonetic letters, not Latin script for this purpose.
    assert_eq!(measure("ɐɐɐ").words, 0);
}

#[test]
fn every_vowel_with_an_accent_makes_a_word_that_has_one() {
    let vowels = "àáâãäåæèéêëìíîïòóôõöøùúûüýÿœÀÁÂÃÄÅÆÈÉÊËÌÍÎÏÒÓÔÕÖØÙÚÛÜÝŸŒ";
    for c in vowels.chars() {
        let m = measure(&format!("b{c}b"));
        assert_eq!((m.words, m.vowelless_words), (1, 0), "{c:?}");
    }
}

#[test]
fn accented_consonants_are_not_vowels() {
    for c in "çðñßþÇÐÑÞ".chars() {
        let m = measure(&format!("b{c}b"));
        assert_eq!((m.words, m.vowelless_words), (1, 1), "{c:?}");
    }
}

#[test]
fn the_longest_run_stays_the_longest_when_shorter_ones_follow() {
    assert_eq!(measure("cccc bb a").longest_token, 4);
    assert_eq!(measure("a\ncccc\tb").longest_token, 4);
}

// ---------- names that are stored (ADR-0017 reads them back) ----------

#[test]
fn verdict_names_are_exactly_the_agreed_list_and_parse_back() {
    assert_eq!(
        TextVerdict::ALL,
        [TextVerdict::Good, TextVerdict::NeedsCheck, TextVerdict::Unusable]
    );
    for verdict in TextVerdict::ALL {
        assert_eq!(TextVerdict::parse(verdict.as_str()), Some(verdict));
    }
    for bad in ["", "Good", " good", "good ", "ok", "needs check"] {
        assert_eq!(TextVerdict::parse(bad), None, "{bad:?}");
    }
}

#[test]
fn reason_names_are_exactly_the_agreed_list_and_parse_back() {
    let names: Vec<&str> = Reason::ALL.iter().map(|r| r.as_str()).collect();
    assert_eq!(
        names,
        [
            "no_text",
            "replacement_characters",
            "ocr_disagrees",
            "ocr_partly_agrees",
            "ocr_agrees",
            "risky_font",
            "no_keywords",
            "many_vowelless_words",
        ]
    );
    for reason in Reason::ALL {
        assert_eq!(Reason::parse(reason.as_str()), Some(reason));
    }
    for bad in ["", "No_Text", " no_text", "no_text ", "text", "*"] {
        assert_eq!(Reason::parse(bad), None, "{bad:?}");
    }
}
