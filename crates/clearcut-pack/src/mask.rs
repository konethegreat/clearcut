// Written by Kone Tshivhinda & Claude
//! Hiding personal data in a page image.
//!
//! The rules for what counts as personal data are in `clearcut-core`, and they
//! work on text. To hide the same words in the picture, something has to find
//! the words' positions and paint over them: that is pixel work and it needs the
//! word boxes from reading the page, so it belongs in the document worker (P2-05),
//! which only paints what Rust decided. This is the interface; until the worker
//! implements it, [`NoMasker`] says honestly that nothing was masked.

use crate::input::{PageImage, PageText};
use clearcut_core::redact::Span;

/// Why an image could not be masked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MaskError {
    #[error("no image masker is available")]
    Unsupported,
    #[error("the image could not be masked")]
    Failed,
}

pub trait ImageMasker {
    /// Returns `image` with the words at `spans` (byte ranges of `text.content`,
    /// the **original** text) painted over.
    fn mask(&self, image: &PageImage, text: &PageText, spans: &[Span]) -> Result<PageImage, MaskError>;
}

/// The masker for when there is none: it always says so.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoMasker;

impl ImageMasker for NoMasker {
    fn mask(&self, _image: &PageImage, _text: &PageText, _spans: &[Span]) -> Result<PageImage, MaskError> {
        Err(MaskError::Unsupported)
    }
}
