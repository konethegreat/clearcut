// Written by Kone Tshivhinda & Claude
//! Hiding personal data in a page picture (ADR-0014, ADR-0016).
//!
//! The rules for what counts as personal data are in `clearcut-core` and work on text.
//! This turns the places they found into rectangles on the picture and has the worker
//! paint them, which is all the worker does: it has no rules.
//!
//! **Which words are hidden.** A redaction span is a range of bytes of the page's text,
//! and the page's words (from the text layer or from OCR) say which bytes are which word
//! and where it is. Every word the span touches, even partly, is hidden whole, with a small
//! margin all round. A span that touches no word cannot be located, so it cannot be
//! painted: that is an error, never a quiet omission, so that the strict policy sends
//! nothing rather than a picture with a number still in it.
//!
//! **Why `prepare` is asynchronous and `mask` is not.** The pack builder asks for the
//! masked picture while it builds, and it is not asynchronous; the worker is a network
//! call. So the call is made first, for the spans the builder is about to compute (the
//! redaction is a pure function of the text, so the caller can compute the same ones), and
//! the masker it gives back hands that picture over, but only if it is asked about the very
//! same picture, text and spans. Anything else is a failure, so a mismatch can only make a
//! page less sendable, never leak a number.

#[cfg(test)]
mod tests;

use crate::request::{ImageFormat, MaskBox, MAX_BOXES, MAX_EDGE};
use crate::{DocumentProcessor, PageContent, PageSource, PageWord};
use bytes::Bytes;
use clearcut_core::redact::Span;
use clearcut_pack::{ImageMasker, MaskError, PageImage, PageText, TextSource};

/// How far beyond a word, in thousandths of the picture on every side, the paint goes, so
/// the edges of letters (which a box can cut a hair short of) do not show.
pub const MASK_MARGIN_PER_MILLE: u16 = 3;

/// Why the spans could not be turned into boxes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MaskPlanError {
    /// A span touches no word (or has no length), so there is nowhere to paint.
    #[error("a value to hide could not be located on the page")]
    UnlocatedSpan,
    /// More words to hide than one request may carry.
    #[error("there are more words to hide than one request can carry")]
    TooManyBoxes,
}

fn touches(word: &PageWord, span: &Span) -> bool {
    word.start < span.end && span.start < word.end
}

/// The word's box with the margin, kept inside the picture.
fn with_margin(word: &PageWord) -> Option<MaskBox> {
    let margin = MASK_MARGIN_PER_MILLE;
    let b = word.bounds;
    let left = b.x.saturating_sub(margin);
    let top = b.y.saturating_sub(margin);
    let right = b.x.saturating_add(b.w).saturating_add(margin).min(1000);
    let bottom = b.y.saturating_add(b.h).saturating_add(margin).min(1000);
    MaskBox::new(left, top, right.checked_sub(left)?, bottom.checked_sub(top)?).ok()
}

/// The boxes to paint to hide `spans`: one for each word any span touches, in the order of
/// the words.
pub fn boxes_for_spans(words: &[PageWord], spans: &[Span]) -> Result<Vec<MaskBox>, MaskPlanError> {
    let located = |span: &Span| span.start < span.end && words.iter().any(|word| touches(word, span));
    if !spans.iter().all(located) {
        return Err(MaskPlanError::UnlocatedSpan);
    }
    let boxes = words
        .iter()
        .filter(|word| spans.iter().any(|span| touches(word, span)))
        .map(|word| with_margin(word).ok_or(MaskPlanError::UnlocatedSpan))
        .collect::<Result<Vec<_>, _>>()?;
    if boxes.len() > MAX_BOXES {
        return Err(MaskPlanError::TooManyBoxes);
    }
    Ok(boxes)
}

/// An [`ImageMasker`] that has already had the worker paint one page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerMasker {
    original: PageImage,
    text: PageText,
    spans: Vec<Span>,
    masked: PageImage,
}

impl WorkerMasker {
    /// Has the worker paint over `spans` (byte ranges of `text.content`) on `image`.
    ///
    /// `page` must be the page the words come from: the same text, read the same way.
    /// Nothing is asked of the worker when there is nothing to hide.
    pub async fn prepare<P: DocumentProcessor>(
        processor: &P,
        image: &PageImage,
        page: &PageContent,
        text: &PageText,
        spans: &[Span],
    ) -> Result<WorkerMasker, MaskError> {
        let same_reading = matches!(
            (page.source(), text.source),
            (PageSource::TextLayer, TextSource::TextLayer) | (PageSource::Ocr, TextSource::Ocr)
        );
        if !same_reading || page.text() != text.content {
            return Err(MaskError::Failed);
        }
        let masked = if spans.is_empty() {
            image.clone()
        } else {
            let words = page.words().ok_or(MaskError::Failed)?;
            let boxes = boxes_for_spans(words, spans).map_err(|_| MaskError::Failed)?;
            let painted = processor
                .mask_image(
                    Bytes::from(image.bytes.clone()),
                    boxes,
                    ImageFormat::of(image.media),
                    MAX_EDGE,
                )
                .await
                .map_err(|_| MaskError::Failed)?;
            PageImage {
                media: painted.media(),
                bytes: painted.bytes().to_vec(),
            }
        };
        Ok(WorkerMasker {
            original: image.clone(),
            text: text.clone(),
            spans: spans.to_vec(),
            masked,
        })
    }
}

impl ImageMasker for WorkerMasker {
    fn mask(&self, image: &PageImage, text: &PageText, spans: &[Span]) -> Result<PageImage, MaskError> {
        if *image == self.original && *text == self.text && spans == self.spans.as_slice() {
            Ok(self.masked.clone())
        } else {
            Err(MaskError::Failed)
        }
    }
}
