// Written by Kone Tshivhinda & Claude
//! The record of one pack, for the per-call log (ADR-0014 §4).
//!
//! It answers "was context missing?" and "did the cache have a chance?" without
//! keeping what was read: sizes, names, hashes and counts only. **No document
//! text, no redacted value, no nonce.**

use serde::Serialize;
use std::collections::BTreeMap;

/// What happened to the page image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageState {
    /// There is no image.
    NotApplicable,
    /// The page's text was checked and had nothing to hide.
    NotNeeded,
    /// Personal data was found and painted over.
    Masked,
    /// Personal data was found in the text and the image could not be masked.
    Unmasked,
    /// There is no text to check the image against, or only text that could not itself be
    /// checked (an unverified text layer), so nothing is known.
    Unchecked,
}

/// One part of the request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Section {
    pub kind: &'static str,
    pub bytes: usize,
    pub cached: bool,
}

/// What redaction did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RedactionReport {
    pub policy: &'static str,
    /// How many values of each kind were replaced (kinds with none are left out).
    pub counts: BTreeMap<&'static str, usize>,
    pub image: ImageState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PackManifest {
    /// `reading-v2/prompt-v1`: the shape and the prompt that produced this call.
    pub prompt_version: String,
    /// SHA-256 of the cached prefix (all system blocks); equal hashes mean a cache can hit.
    pub prefix_sha256: String,
    pub prefix_bytes: usize,
    /// SHA-256 of the output schema; a change also breaks the cache.
    pub schema_sha256: String,
    pub model: String,
    pub effort: &'static str,
    pub cache_ttl: &'static str,
    pub page: u32,
    pub of: u32,
    /// `text_layer`, `ocr`, or `none`.
    pub text_source: &'static str,
    pub sections: Vec<Section>,
    pub redaction: RedactionReport,
    /// Kinds of instruction-like text found (never the text).
    pub injection_suspected: Vec<&'static str>,
    /// The model whose reading failed, when this is an escalation.
    pub escalated_from: Option<String>,
}
