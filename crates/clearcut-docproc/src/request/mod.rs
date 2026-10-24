// Written by Kone Tshivhinda & Claude
//! What may be asked of the worker, in terms it will accept.
//!
//! Each type can only hold values the worker would take, so a request that the worker is
//! certain to refuse cannot be built. (The worker checks again; this makes a mistake
//! in the pipeline fail where it is made, not as an HTTP error.)

#[cfg(test)]
mod tests;

use clearcut_provider::ImageType;
use std::num::NonZeroU32;

/// The picture is drawn at least this sharp (dots per inch).
pub const MIN_DPI: u16 = 100;
/// And at most this.
pub const MAX_DPI: u16 = 300;
/// The longest side of a picture, in pixels, is at least this.
pub const MIN_EDGE: u16 = 500;
/// And at most this (what the model API takes on the models ClearCut uses).
pub const MAX_EDGE: u16 = 2576;
/// Boxes in one masking request.
pub const MAX_BOXES: usize = 500;

/// A request the worker would refuse for its shape alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the request is outside what the document worker accepts")]
pub struct InvalidRequest;

/// Which pages of a document to read: from one page to another, or to the end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageRange {
    first: NonZeroU32,
    last: Option<NonZeroU32>,
}

impl PageRange {
    /// The most pages one call reads.
    pub const MAX_PAGES: u32 = 50;

    /// Pages `first` to `last`, or `first` to the end of the document (at most
    /// [`PageRange::MAX_PAGES`] pages) when `last` is `None`. Pages count from 1.
    pub fn new(first: NonZeroU32, last: Option<NonZeroU32>) -> Result<Self, InvalidRequest> {
        if let Some(last) = last {
            let span = u64::from(last.get()) + 1;
            let start = u64::from(first.get());
            if span <= start || span - start > u64::from(Self::MAX_PAGES) {
                return Err(InvalidRequest);
            }
        }
        Ok(PageRange { first, last })
    }

    /// Just this page.
    pub fn single(page: NonZeroU32) -> Self {
        PageRange {
            first: page,
            last: Some(page),
        }
    }

    pub fn first(&self) -> NonZeroU32 {
        self.first
    }

    pub fn last(&self) -> Option<NonZeroU32> {
        self.last
    }
}

/// How a picture is sent back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
}

impl ImageFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpeg",
        }
    }

    pub fn image_type(self) -> ImageType {
        match self {
            ImageFormat::Png => ImageType::Png,
            ImageFormat::Jpeg => ImageType::Jpeg,
        }
    }

    pub fn of(media: ImageType) -> Self {
        match media {
            ImageType::Png => ImageFormat::Png,
            ImageType::Jpeg => ImageFormat::Jpeg,
        }
    }
}

/// How a page is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderOptions {
    dpi: u16,
    format: ImageFormat,
    max_edge: u16,
}

impl RenderOptions {
    pub fn new(dpi: u16, format: ImageFormat, max_edge: u16) -> Result<Self, InvalidRequest> {
        if !(MIN_DPI..=MAX_DPI).contains(&dpi) || !(MIN_EDGE..=MAX_EDGE).contains(&max_edge) {
            return Err(InvalidRequest);
        }
        Ok(RenderOptions {
            dpi,
            format,
            max_edge,
        })
    }

    pub fn dpi(&self) -> u16 {
        self.dpi
    }

    pub fn format(&self) -> ImageFormat {
        self.format
    }

    pub fn max_edge(&self) -> u16 {
        self.max_edge
    }
}

impl Default for RenderOptions {
    /// 150 dpi, PNG, the longest side at most 2000 pixels: about 2 800 tokens for an A4 page.
    fn default() -> Self {
        RenderOptions {
            dpi: 150,
            format: ImageFormat::Png,
            max_edge: 2000,
        }
    }
}

/// A rectangle to paint over, in thousandths of the picture from its top left (the same
/// unit as the word boxes the worker reports).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MaskBox {
    x: u16,
    y: u16,
    w: u16,
    h: u16,
}

impl MaskBox {
    pub fn new(x: u16, y: u16, w: u16, h: u16) -> Result<Self, InvalidRequest> {
        let (x32, y32, w32, h32) = (u32::from(x), u32::from(y), u32::from(w), u32::from(h));
        if w == 0 || h == 0 || x32 + w32 > 1000 || y32 + h32 > 1000 {
            return Err(InvalidRequest);
        }
        Ok(MaskBox { x, y, w, h })
    }

    pub fn x(&self) -> u16 {
        self.x
    }

    pub fn y(&self) -> u16 {
        self.y
    }

    pub fn w(&self) -> u16 {
        self.w
    }

    pub fn h(&self) -> u16 {
        self.h
    }
}
