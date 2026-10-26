// Written by Kone Tshivhinda & Claude
use super::estimate::*;

#[test]
fn text_is_counted_at_three_bytes_to_a_token_rounded_up() {
    // Real text is nearer four bytes to a token, so this over-counts: the estimate is a
    // reservation against a budget and must never be less than the bill.
    assert_eq!(text_tokens(0), 0);
    assert_eq!(text_tokens(1), 1);
    assert_eq!(text_tokens(3), 1);
    assert_eq!(text_tokens(4), 2);
    assert_eq!(text_tokens(3_000), 1_000);
    assert_eq!(text_tokens(3_001), 1_001);
}

#[test]
fn a_picture_is_a_token_for_every_28_by_28_patch() {
    // research 06 section 7: an A4 page at 150 dpi (1 240 by 1 754 pixels) is about 2 835 tokens.
    assert_eq!(image_tokens(1_240, 1_754), 45 * 63);
    assert_eq!(image_tokens(1_240, 1_754), 2_835);
    assert_eq!(image_tokens(28, 28), 1);
    assert_eq!(image_tokens(29, 28), 2);
    assert_eq!(image_tokens(1, 1), 1);
}

#[test]
fn a_picture_is_capped_at_what_the_models_take() {
    // 72 x 72 patches is 5 184, over the 4 784 the models take.
    assert_eq!(image_tokens(2_000, 2_000), 4_784);
    assert_eq!(image_tokens(2_576, 2_576), 4_784);
    // Under the cap it is not cut: 58 x 58 patches.
    assert_eq!(image_tokens(1_600, 1_600), 58 * 58);
    // Exactly at the cap is the cap: 4 784 is 4 784 patches of one by 4 784.
    assert_eq!(image_tokens(28, 28 * 4_784), 4_784);
    assert_eq!(image_tokens(28, 28 * 4_785), 4_784);
}

#[test]
fn a_huge_picture_does_not_overflow() {
    assert_eq!(image_tokens(u32::MAX, u32::MAX), 4_784);
}

#[test]
fn a_call_is_the_page_the_prefix_the_schema_and_the_answer_cap() {
    let estimate = call_estimate(&Sizes {
        prefix_bytes: 4_000,
        page_text_bytes: 3_000,
        schema_bytes: 600,
        width: 1_240,
        height: 1_754,
        max_output_tokens: 8_000,
    });
    // The prefix is its own part (it may have to be written to the cache).
    assert_eq!(estimate.prefix_tokens, 1_334);
    // After the breakpoint: the text, the schema (counted as input on every call, to be safe)
    // and the picture.
    // The page's own text is the document's, so it is counted at a token a byte; the schema is
    // ours and is counted at three bytes to a token.
    assert_eq!(estimate.page_input_tokens, 3_000 + 200 + 2_835);
    assert_eq!(estimate.max_output_tokens, 8_000);
}

#[test]
fn a_sum_that_would_overflow_saturates() {
    let estimate = call_estimate(&Sizes {
        prefix_bytes: usize::MAX,
        page_text_bytes: usize::MAX,
        schema_bytes: usize::MAX,
        width: u32::MAX,
        height: u32::MAX,
        max_output_tokens: u32::MAX,
    });
    assert!(estimate.prefix_tokens > 0);
    assert!(estimate.page_input_tokens > 0);
}

#[test]
fn only_the_uncached_text_counts_not_the_prefix_and_not_the_picture() {
    let section = |kind, bytes, cached| clearcut_pack::Section { kind, bytes, cached };
    let sections = [
        section("rules", 40_000, true),
        section("tenant_rules", 500, true),
        section("page_image", 900_000, false),
        section("document_context", 100, false),
        section("page_text", 2_000, false),
        section("previous_attempt", 300, false),
        section("instruction", 50, false),
    ];
    assert_eq!(uncached_text_bytes(&sections), 2_450);
    assert_eq!(uncached_text_bytes(&[]), 0);
}

#[test]
fn text_from_the_document_is_counted_a_token_a_byte() {
    // Digits, symbols and non-Latin text can cost a token for each byte, and the page's text is
    // the document's to write: a reservation must hold whatever it says.
    assert_eq!(untrusted_text_tokens(0), 0);
    assert_eq!(untrusted_text_tokens(1), 1);
    assert_eq!(untrusted_text_tokens(200_000), 200_000);
    // Our own text (the rules, the schema) is ordinary English and stays at three bytes.
    assert_eq!(text_tokens(3_000), 1_000);
}
