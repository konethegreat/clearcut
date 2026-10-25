// Written by Kone Tshivhinda & Claude
//! What goes into a pack.

use clearcut_core::extraction::Finding;
use clearcut_core::page_read::{PageTextSource, ReadRoute};
use clearcut_core::reading::ReadingProblem;
use clearcut_core::Currency;
use clearcut_provider::{CacheTtl, CallContext, Effort, ImageType};
use std::num::NonZeroU32;

/// A rendered page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageImage {
    pub media: ImageType,
    pub bytes: Vec<u8>,
}

/// Where a page's text came from; the model is told, so it can weigh it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextSource {
    /// The document's own text layer, which passed its checks.
    TextLayer,
    /// Read from the picture by OCR, which can make mistakes.
    Ocr,
    /// The document's own text layer, which could not be checked (OCR only partly agreed with
    /// it, or was not run) and may be wrong: only a hint for reading unclear characters.
    UnverifiedTextLayer,
}

impl TextSource {
    pub fn as_str(self) -> &'static str {
        match self {
            TextSource::TextLayer => "text_layer",
            TextSource::Ocr => "ocr",
            TextSource::UnverifiedTextLayer => "unverified_text_layer",
        }
    }
}

/// A page's text. **Untrusted**, like everything on the page.
#[derive(Clone, PartialEq, Eq)]
pub struct PageText {
    pub source: TextSource,
    pub content: String,
}

impl PageText {
    /// The text that goes with a page and how it is labelled, from the page's reading route
    /// (ADR-0017): a checked layer for `text_and_image`, an unchecked one for `image_first`, OCR
    /// text for `image_only`. Anything else, and text that is only white space, is no text; in
    /// particular a rejected text layer is never sent.
    pub fn for_route(route: ReadRoute, source: PageTextSource, content: String) -> Option<PageText> {
        if content.trim().is_empty() {
            return None;
        }
        let source = match (route, source) {
            (ReadRoute::TextAndImage, PageTextSource::TextLayer) => TextSource::TextLayer,
            (ReadRoute::ImageFirst, PageTextSource::TextLayer) => TextSource::UnverifiedTextLayer,
            (ReadRoute::ImageOnly, PageTextSource::Ocr) => TextSource::Ocr,
            _ => return None,
        };
        Some(PageText { source, content })
    }
}

impl std::fmt::Debug for PageText {
    /// Invoice text: only the size is shown.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PageText({:?}, {} bytes)", self.source, self.content.len())
    }
}

/// One page to read. At least one of `image` and `text` must be present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageInput {
    /// 1-based.
    pub page: NonZeroU32,
    /// How many pages the document has.
    pub of: NonZeroU32,
    pub image: Option<PageImage>,
    pub text: Option<PageText>,
}

/// What earlier pages of the same document established. Values that were read
/// from the document by a model are still untrusted text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocumentContext {
    pub currency: Option<Currency>,
    pub invoice_number: Option<String>,
    pub supplier_name: Option<String>,
    pub totals_seen: bool,
    pub lines_read: u32,
}

/// The checks that failed on the previous tier's reading of this page, in
/// ClearCut's own vocabulary (never the model's or the document's words).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureReport {
    pub previous_model: String,
    pub problems: Vec<ReadingProblem>,
    pub findings: Vec<Finding>,
}

/// What to do about personal data in a page image that cannot be masked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedactionPolicy {
    /// Redact what can be redacted, send the rest, and record what could not be hidden.
    BestEffort,
    /// Send nothing that could not be checked or hidden; the page goes to review.
    Strict,
}

impl RedactionPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            RedactionPolicy::BestEffort => "best_effort",
            RedactionPolicy::Strict => "strict",
        }
    }
}

/// The parts of a request that do not depend on the page.
#[derive(Debug, Clone)]
pub struct PackSettings {
    pub model: String,
    pub call: CallContext,
    pub effort: Effort,
    pub max_output_tokens: u32,
    pub cache_ttl: CacheTtl,
    /// The schema the model must follow (`reading_schema()` in production).
    pub output_schema: serde_json::Value,
    /// Conventions confirmed by this tenant's reviewers (memory, P2-17); `None` until it exists.
    pub tenant_rules: Option<String>,
    pub redaction: RedactionPolicy,
}
