// Written by Kone Tshivhinda & Claude
//! The static prompt: the same words for every call of one prompt version.
//!
//! The text lives in `prompts/prompt-v1/` and is compiled into the program with
//! `include_str!`, so it ships with the binary and cannot drift from what the
//! tests check. Line endings are normalised, because a Windows checkout and a
//! Linux build must produce the same bytes or the prompt cache never hits.
//!
//! Changing a word changes the cache and makes gold-set scores incomparable, so
//! a change needs a new `PROMPT_VERSION`; a golden file in the tests enforces it.

use clearcut_core::reading::READING_SHAPE_VERSION;

/// The version of the words below.
pub const PROMPT_VERSION: &str = "prompt-v1";

const RULES: &str = include_str!("../prompts/prompt-v1/rules.md");
const EXAMPLES: &str = include_str!("../prompts/prompt-v1/examples.md");

/// The rules and worked examples, as the first (cached) system block.
pub fn static_prefix() -> String {
    let rules = RULES.replace("\r\n", "\n");
    let examples = EXAMPLES.replace("\r\n", "\n");
    format!("{}\n\n{}", rules.trim(), examples.trim())
}

/// `reading-v2/prompt-v1`, recorded as the extractor's `prompt_version`.
pub fn prompt_version() -> String {
    format!("{READING_SHAPE_VERSION}/{PROMPT_VERSION}")
}
