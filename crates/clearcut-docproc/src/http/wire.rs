// Written by Kone Tshivhinda & Claude
//! Reading the worker's page answers (`/v1/text` and `/v1/ocr`).
//!
//! The shape is checked field by field; fields this client does not know yet are
//! ignored (so the worker can add one without breaking its caller), but every field it
//! does know must be exactly what was promised. A page that cannot exist is a bad
//! response, and the reason is fixed text: nothing the worker said is repeated.

use crate::page::{PageContent, PageParts, PageSource, PageWord, WordBox};
use crate::request::PageRange;
use crate::ProcessError;
use clearcut_core::text_quality::FontFact;
use serde::Deserialize;
use std::num::NonZeroU32;

/// What the caller asked for, so the answer can be held to it.
pub(super) enum Expect {
    /// `page_texts`: these pages, in order, with or without words.
    Text { range: PageRange, words: bool },
    /// `ocr_page`: this one page, read by OCR, with an agreement if asked.
    Ocr { page: NonZeroU32, agreement: bool },
}

#[derive(Deserialize)]
struct WirePages {
    pages: Vec<WirePage>,
}

#[derive(Deserialize)]
struct WirePage {
    page: u64,
    source: String,
    text: String,
    width_pt: f64,
    height_pt: f64,
    fonts: Vec<FontFact>,
    #[serde(default)]
    words: Option<Vec<WireWord>>,
    #[serde(default)]
    agreement: Option<u64>,
}

#[derive(Deserialize)]
struct WireWord {
    start: u64,
    end: u64,
    x: u64,
    y: u64,
    w: u64,
    h: u64,
    #[serde(default)]
    confidence: Option<u64>,
}

const SHAPE: ProcessError = ProcessError::BadResponse("not the expected shape");
const PAGE: ProcessError = ProcessError::BadResponse("a page was not as promised");
const PAGES: ProcessError = ProcessError::BadResponse("not the pages that were asked for");

pub(super) fn parse_pages(body: &[u8], expect: &Expect) -> Result<Vec<PageContent>, ProcessError> {
    let wire: WirePages = serde_json::from_slice(body).map_err(|_| SHAPE)?;
    let pages = wire
        .pages
        .into_iter()
        .map(page_of)
        .collect::<Result<Vec<_>, _>>()?;
    match expect {
        Expect::Text { range, words } => check_text(&pages, *range, *words)?,
        Expect::Ocr { page, agreement } => check_ocr(&pages, *page, *agreement)?,
    }
    Ok(pages)
}

fn small<T: TryFrom<u64>>(value: u64) -> Result<T, ProcessError> {
    T::try_from(value).map_err(|_| PAGE)
}

fn word_of(wire: WireWord) -> Result<PageWord, ProcessError> {
    Ok(PageWord {
        start: small::<usize>(wire.start)?,
        end: small::<usize>(wire.end)?,
        bounds: WordBox {
            x: small(wire.x)?,
            y: small(wire.y)?,
            w: small(wire.w)?,
            h: small(wire.h)?,
        },
        confidence: wire.confidence.map(small::<u16>).transpose()?,
    })
}

fn page_of(wire: WirePage) -> Result<PageContent, ProcessError> {
    let page = NonZeroU32::new(small::<u32>(wire.page)?).ok_or(PAGE)?;
    let source = PageSource::parse(&wire.source).ok_or(PAGE)?;
    let words = wire
        .words
        .map(|list| list.into_iter().map(word_of).collect::<Result<Vec<_>, _>>())
        .transpose()?;
    PageContent::from_parts(PageParts {
        page,
        source,
        text: wire.text,
        width_pt: wire.width_pt,
        height_pt: wire.height_pt,
        fonts: wire.fonts,
        words,
        agreement: wire.agreement.map(small::<u16>).transpose()?,
    })
    .map_err(|_| PAGE)
}

fn check_text(pages: &[PageContent], range: PageRange, words: bool) -> Result<(), ProcessError> {
    let first = u64::from(range.first().get());
    let count = pages.len() as u64;
    let right_count = match range.last() {
        Some(last) => count == u64::from(last.get()) - first + 1,
        None => (1..=u64::from(PageRange::MAX_PAGES)).contains(&count),
    };
    if !right_count {
        return Err(PAGES);
    }
    for (offset, page) in pages.iter().enumerate() {
        let in_place = u64::from(page.page().get()) == first + offset as u64;
        // A text answer never comes from OCR, and carries words exactly when they were asked for.
        let source_ok = page.source() != PageSource::Ocr;
        if !in_place || !source_ok || page.words().is_some() != words {
            return Err(PAGES);
        }
    }
    Ok(())
}

fn check_ocr(pages: &[PageContent], page: NonZeroU32, agreement: bool) -> Result<(), ProcessError> {
    let [read] = pages else { return Err(PAGES) };
    if read.page() != page || read.source() != PageSource::Ocr {
        return Err(PAGES);
    }
    // An agreement nobody asked for is not believed; one that was asked for may be missing
    // (the page has no text layer to compare with).
    if read.agreement().is_some() && !agreement {
        return Err(PAGES);
    }
    Ok(())
}
