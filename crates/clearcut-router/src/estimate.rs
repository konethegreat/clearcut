// Written by Kone Tshivhinda & Claude
//! How big a call will be, worked out before it is sent (ADR-0018 section 6).
//!
//! The estimate is a **reservation against a budget**, not a forecast, so every figure here
//! errs high: our own text (the rules, the schema) is counted at three bytes to a token (real
//! English is nearer four), **the document's text at a token a byte** (digits, symbols and
//! non-Latin text can cost that much, and the page is the document's to write), the schema is
//! counted as input on every call, and a picture is counted by the 28 by 28 pixel patches it
//! holds, up to the most the models take. A call whose worst case fits the budget can never
//! cost more than it.

use clearcut_core::cost::CallEstimate;
use clearcut_pack::Section;

/// The most tokens one picture can cost (research 06 section 7).
const MAX_IMAGE_TOKENS: u64 = 4_784;
/// The side of the square patch that costs one token.
const PATCH: u64 = 28;
const BYTES_PER_TOKEN: u64 = 3;

/// Tokens in `bytes` of text, rounded up, at three bytes to a token.
pub(crate) fn text_tokens(bytes: usize) -> u64 {
    (bytes as u64).div_ceil(BYTES_PER_TOKEN)
}

/// Tokens in text that came from the document: a token for every byte, the most any text can
/// cost, so no page can make a call dearer than its reservation.
pub(crate) fn untrusted_text_tokens(bytes: usize) -> u64 {
    bytes as u64
}

/// Tokens in a picture of `width` by `height` pixels: one for every 28 by 28 patch, at most
/// what the models take. Saturating, so no size can overflow.
pub(crate) fn image_tokens(width: u32, height: u32) -> u64 {
    let across = u64::from(width).div_ceil(PATCH);
    let down = u64::from(height).div_ceil(PATCH);
    across.saturating_mul(down).min(MAX_IMAGE_TOKENS)
}

/// The sizes a call is estimated from.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Sizes {
    /// The cached system blocks (the rules, the examples, any tenant rules).
    pub prefix_bytes: usize,
    /// Every text block after the cache breakpoint: the page text, the document context, the
    /// report of the previous attempt and the instruction. Counted a token a byte.
    pub page_text_bytes: usize,
    pub schema_bytes: usize,
    /// The picture, in pixels.
    pub width: u32,
    pub height: u32,
    pub max_output_tokens: u32,
}

pub(crate) fn call_estimate(sizes: &Sizes) -> CallEstimate {
    let page_input_tokens = untrusted_text_tokens(sizes.page_text_bytes)
        .saturating_add(text_tokens(sizes.schema_bytes))
        .saturating_add(image_tokens(sizes.width, sizes.height));
    CallEstimate {
        page_input_tokens,
        prefix_tokens: text_tokens(sizes.prefix_bytes),
        max_output_tokens: sizes.max_output_tokens,
    }
}

/// The text sent after the cache breakpoint: every section that is not cached and is not the
/// picture (a picture is counted by its pixels, never by its bytes).
pub(crate) fn uncached_text_bytes(sections: &[Section]) -> usize {
    sections
        .iter()
        .filter(|section| !section.cached && section.kind != "page_image")
        .map(|section| section.bytes)
        .sum()
}
