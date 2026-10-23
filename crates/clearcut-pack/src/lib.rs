// Written by Kone Tshivhinda & Claude
//! # clearcut-pack
//!
//! The context-pack builder (ADR-0014): the decision of **what the model is
//! shown** for one page. It is a pure function from a page and what is known
//! about its document to a `ModelRequest`, with three jobs:
//!
//! 1. put the right resources in front of the model, in an order that makes the
//!    prompt cache hit (the rules and worked examples first and byte-identical
//!    for every call; everything per page after the last breakpoint);
//! 2. send no more personal data than reading the page needs (the redaction
//!    rules are in `clearcut-core`; images are masked through an [`ImageMasker`]);
//! 3. make a hostile page unable to steer anything (the page is data inside
//!    tags with a fresh random id, there are no tools, and suspected injections
//!    are flagged and recorded, never obeyed and never silently removed).
//!
//! It does not choose models, count money or call the API. Every pack comes with
//! a [`PackManifest`] for the call log: what was sent, in sizes and names, and
//! never any of the text.

mod build;
mod input;
mod manifest;
mod mask;
mod prompt;

pub use build::{
    build_pack, build_pack_with_nonce, fresh_nonce, Pack, PackError, UnsafeReason, MAX_PAGE_TEXT_BYTES,
};
pub use clearcut_core::redact::{RedactionKind, Span};
pub use input::{
    DocumentContext, FailureReport, PackSettings, PageImage, PageInput, PageText, RedactionPolicy, TextSource,
};
pub use manifest::{ImageState, PackManifest, RedactionReport, Section};
pub use mask::{ImageMasker, MaskError, NoMasker};
pub use prompt::{prompt_version, static_prefix, PROMPT_VERSION};
