// Written by Kone Tshivhinda & Claude
//! What the worker reports about a page, and the picture it draws.
//!
//! The worker is trusted to do its job but not to be perfect: everything it says is
//! checked again here before the rest of the system sees it, with the same rules the
//! worker applies to its own answer. A [`PageContent`] therefore can only hold a page
//! whose words lie inside its text, in order, on character boundaries, with boxes
//! inside the page. The text of a page is invoice text, so it is never printed.

#[cfg(test)]
mod tests;

use crate::request::MAX_EDGE;
use bytes::Bytes;
use clearcut_core::text_quality::FontFact;
use clearcut_provider::ImageType;
use std::fmt;
use std::num::NonZeroU32;

/// The most text of one page the worker will report.
pub const MAX_PAGE_TEXT_BYTES: usize = 1024 * 1024;
/// The most words of one page.
pub const MAX_WORDS_PER_PAGE: usize = 20_000;
/// The most fonts reported for one page.
pub const MAX_FONTS: usize = 64;
/// The longest side of a picture the worker sends back.
pub const MAX_IMAGE_EDGE: u32 = MAX_EDGE as u32;
const MAX_PAGE_POINTS: f64 = 100_000.0;

/// A page the worker described in a way it should not have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InvalidPage {
    #[error("a page's text was not acceptable")]
    Text,
    #[error("a page's size was not acceptable")]
    Size,
    #[error("a page had too many fonts")]
    Fonts,
    #[error("a page's words were not acceptable")]
    Words,
    #[error("a page's source did not fit what else was said about it")]
    Source,
    #[error("a page's OCR agreement was not acceptable")]
    Agreement,
}

/// Where a page's text came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageSource {
    /// The PDF's own text layer.
    TextLayer,
    /// The page has no text layer at all.
    None,
    /// Read from the picture of the page by OCR.
    Ocr,
}

impl PageSource {
    pub fn as_str(self) -> &'static str {
        match self {
            PageSource::TextLayer => "text_layer",
            PageSource::None => "none",
            PageSource::Ocr => "ocr",
        }
    }

    /// Exact match only.
    pub fn parse(text: &str) -> Option<PageSource> {
        match text {
            "text_layer" => Some(PageSource::TextLayer),
            "none" => Some(PageSource::None),
            "ocr" => Some(PageSource::Ocr),
            _ => None,
        }
    }
}

/// Where a word is, in thousandths of the page from its top left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WordBox {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

/// A word: where it is in the page's text (UTF-8 byte offsets) and on the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageWord {
    pub start: usize,
    pub end: usize,
    pub bounds: WordBox,
    /// How sure OCR was, in thousandths. Only an OCR page has it.
    pub confidence: Option<u16>,
}

/// The pieces of a page, before they are checked.
#[derive(Debug, Clone, PartialEq)]
pub struct PageParts {
    pub page: NonZeroU32,
    pub source: PageSource,
    pub text: String,
    pub width_pt: f64,
    pub height_pt: f64,
    pub fonts: Vec<FontFact>,
    pub words: Option<Vec<PageWord>>,
    pub agreement: Option<u16>,
}

/// One page, as the worker reported it and as checked here.
#[derive(Clone, PartialEq)]
pub struct PageContent {
    page: NonZeroU32,
    source: PageSource,
    text: String,
    width_pt: f64,
    height_pt: f64,
    fonts: Vec<FontFact>,
    words: Option<Vec<PageWord>>,
    agreement: Option<u16>,
}

impl fmt::Debug for PageContent {
    /// Invoice text: only the size is shown.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PageContent(page {}, {}, {} bytes, {} words)",
            self.page,
            self.source.as_str(),
            self.text.len(),
            self.words.as_ref().map_or(0, Vec::len)
        )
    }
}

impl PageContent {
    /// Checks the parts and keeps them if they make a possible page.
    pub fn from_parts(parts: PageParts) -> Result<Self, InvalidPage> {
        let PageParts {
            page,
            source,
            text,
            width_pt,
            height_pt,
            fonts,
            words,
            agreement,
        } = parts;

        if text.len() > MAX_PAGE_TEXT_BYTES || text.chars().any(|c| c.is_control() && c != '\n' && c != '\t')
        {
            return Err(InvalidPage::Text);
        }
        for side in [width_pt, height_pt] {
            if !side.is_finite() || side <= 0.0 || side > MAX_PAGE_POINTS {
                return Err(InvalidPage::Size);
            }
        }
        if fonts.len() > MAX_FONTS {
            return Err(InvalidPage::Fonts);
        }
        if let Some(shared) = agreement {
            if source != PageSource::Ocr || shared > 1000 {
                return Err(InvalidPage::Agreement);
            }
        }

        match source {
            PageSource::None => {
                let no_words = words.as_ref().is_none_or(Vec::is_empty);
                if !text.is_empty() || !fonts.is_empty() || !no_words {
                    return Err(InvalidPage::Source);
                }
            }
            PageSource::TextLayer => {
                if text.trim().is_empty() {
                    return Err(InvalidPage::Source);
                }
                if let Some(words) = &words {
                    check_words(&text, words, false)?;
                }
            }
            PageSource::Ocr => {
                if !fonts.is_empty() {
                    return Err(InvalidPage::Source);
                }
                let Some(words) = &words else {
                    return Err(InvalidPage::Source);
                };
                check_words(&text, words, true)?;
                // The text is made of the words, so there is text exactly when there are words.
                if text.is_empty() != words.is_empty() {
                    return Err(InvalidPage::Words);
                }
            }
        }

        Ok(PageContent {
            page,
            source,
            text,
            width_pt,
            height_pt,
            fonts,
            words,
            agreement,
        })
    }

    pub fn page(&self) -> NonZeroU32 {
        self.page
    }

    pub fn source(&self) -> PageSource {
        self.source
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn width_pt(&self) -> f64 {
        self.width_pt
    }

    pub fn height_pt(&self) -> f64 {
        self.height_pt
    }

    pub fn fonts(&self) -> &[FontFact] {
        &self.fonts
    }

    /// The words, if they were asked for (an OCR page always has them).
    pub fn words(&self) -> Option<&[PageWord]> {
        self.words.as_deref()
    }

    /// Per mille of the text layer's words that OCR also read, if it was asked for and
    /// could be worked out.
    pub fn agreement(&self) -> Option<u16> {
        self.agreement
    }

    /// The characters of one of this page's words.
    pub fn word_text(&self, word: &PageWord) -> &str {
        self.text.get(word.start..word.end).unwrap_or("")
    }
}

fn check_words(text: &str, words: &[PageWord], confidence: bool) -> Result<(), InvalidPage> {
    if words.len() > MAX_WORDS_PER_PAGE {
        return Err(InvalidPage::Words);
    }
    let mut previous_end = 0;
    for word in words {
        let inside = previous_end <= word.start && word.start < word.end && word.end <= text.len();
        let on_characters = text.is_char_boundary(word.start) && text.is_char_boundary(word.end);
        if !inside || !on_characters || text[word.start..word.end].chars().any(char::is_whitespace) {
            return Err(InvalidPage::Words);
        }
        let b = word.bounds;
        let (x, y, w, h) = (u32::from(b.x), u32::from(b.y), u32::from(b.w), u32::from(b.h));
        if w == 0 || h == 0 || x + w > 1000 || y + h > 1000 {
            return Err(InvalidPage::Words);
        }
        match (confidence, word.confidence) {
            (true, Some(sure)) if sure <= 1000 => {}
            (false, None) => {}
            _ => return Err(InvalidPage::Words),
        }
        previous_end = word.end;
    }
    Ok(())
}

// ------------------------------------------------------------------- pictures ----

/// A picture the worker could not have drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the document worker's picture was not acceptable")]
pub struct InvalidImage;

/// A picture the worker drew or painted over: PNG or JPEG bytes and the size it states.
#[derive(Clone, PartialEq, Eq)]
pub struct RenderedPage {
    media: ImageType,
    bytes: Bytes,
    width: u32,
    height: u32,
}

impl fmt::Debug for RenderedPage {
    /// A page of an invoice: only the size is shown.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "RenderedPage({:?}, {}x{}, {} bytes)",
            self.media,
            self.width,
            self.height,
            self.bytes.len()
        )
    }
}

impl RenderedPage {
    /// Keeps the picture if it starts like the kind it says it is and has a sensible size.
    pub fn new(media: ImageType, bytes: Bytes, width: u32, height: u32) -> Result<Self, InvalidImage> {
        const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";
        const JPEG: &[u8] = b"\xff\xd8\xff";
        let starts_right = match media {
            ImageType::Png => bytes.starts_with(PNG),
            ImageType::Jpeg => bytes.starts_with(JPEG),
        };
        let sized = (1..=MAX_IMAGE_EDGE).contains(&width) && (1..=MAX_IMAGE_EDGE).contains(&height);
        if !starts_right || !sized {
            return Err(InvalidImage);
        }
        Ok(RenderedPage {
            media,
            bytes,
            width,
            height,
        })
    }

    pub fn media(&self) -> ImageType {
        self.media
    }

    pub fn bytes(&self) -> &Bytes {
        &self.bytes
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }
}
