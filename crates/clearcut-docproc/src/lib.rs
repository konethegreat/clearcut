// Written by Kone Tshivhinda & Claude
//! Reading documents (ADR-0009, ADR-0016).
//!
//! The Python worker is the one place that opens a hostile file. This crate is
//! the Rust side of that boundary:
//!
//! - [`DocumentProcessor`] is the trait the pipeline is written against: look inside a
//!   document, read the text of its pages (with word boxes and font facts), read a page
//!   by OCR, draw a page, and paint boxes over a picture;
//! - [`HttpWorker`] is its implementation: an HTTP client for the worker;
//! - [`FakeWorker`] is a scripted stand-in, so the pipeline can be developed and
//!   tested with no Python and no network;
//! - [`WorkerMasker`] hides personal data in a page picture (the `ImageMasker` of
//!   ADR-0014) by turning redaction spans into boxes and having the worker paint them.
//!
//! Every way a call can go wrong is a [`ProcessError`], and
//! [`ProcessError::failure_kind`] says what the job queue should do about it
//! (ADR-0008): an unreachable or misconfigured worker is the platform's problem
//! and does not use up a document's attempts; a timeout or a server error is
//! retried; a problem with the document itself (encrypted, unreadable, too big)
//! goes to a person.
//!
//! Nothing here prints or returns what the worker said: error values carry fixed
//! reasons and numeric limits, never text from the document or the server. Everything
//! the worker reports is checked again before the rest of the system sees it.

use bytes::Bytes;
use clearcut_core::jobs::FailureKind;
use std::collections::VecDeque;
use std::future::Future;
use std::num::NonZeroU32;
use std::sync::{Arc, Mutex};

mod http;
mod masker;
mod page;
mod request;

pub use http::{
    ConfigError, HttpWorker, DEFAULT_TIMEOUT, MAX_IMAGE_RESPONSE_BYTES, MAX_RESPONSE_BYTES,
    MAX_TEXT_RESPONSE_BYTES,
};
pub use masker::{boxes_for_spans, MaskPlanError, WorkerMasker, MASK_MARGIN_PER_MILLE};
pub use page::{
    InvalidImage, InvalidPage, PageContent, PageParts, PageSource, PageWord, RenderedPage, WordBox,
    MAX_FONTS, MAX_IMAGE_EDGE, MAX_PAGE_TEXT_BYTES, MAX_WORDS_PER_PAGE,
};
pub use request::{
    ImageFormat, InvalidRequest, MaskBox, PageRange, RenderOptions, MAX_BOXES, MAX_DPI, MAX_EDGE, MIN_DPI,
    MIN_EDGE,
};

// ------------------------------------------------------------------ results ----

/// What kind of document the worker found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentKind {
    Pdf,
    /// A picture is a one-page document with no text layer.
    Png,
    Jpeg,
}

impl DocumentKind {
    pub fn media_type(self) -> &'static str {
        match self {
            DocumentKind::Pdf => "application/pdf",
            DocumentKind::Png => "image/png",
            DocumentKind::Jpeg => "image/jpeg",
        }
    }

    /// Exact match only: no case folding, no parameters.
    pub fn parse(text: &str) -> Option<DocumentKind> {
        match text {
            "application/pdf" => Some(DocumentKind::Pdf),
            "image/png" => Some(DocumentKind::Png),
            "image/jpeg" => Some(DocumentKind::Jpeg),
            _ => None,
        }
    }
}

/// The counts did not add up (more pages with text than pages).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("pages with text cannot be more than pages")]
pub struct InvalidInspection;

/// What the worker found inside a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Inspection {
    kind: DocumentKind,
    pages: u32,
    pages_with_text: u32,
}

impl Inspection {
    pub fn new(kind: DocumentKind, pages: u32, pages_with_text: u32) -> Result<Self, InvalidInspection> {
        if pages_with_text > pages {
            return Err(InvalidInspection);
        }
        Ok(Inspection {
            kind,
            pages,
            pages_with_text,
        })
    }

    pub fn kind(&self) -> DocumentKind {
        self.kind
    }

    pub fn pages(&self) -> u32 {
        self.pages
    }

    /// How many pages carry any text in their text layer. A scan has none.
    pub fn pages_with_text(&self) -> u32 {
        self.pages_with_text
    }

    pub fn has_text_layer(&self) -> bool {
        self.pages_with_text > 0
    }
}

// ------------------------------------------------------------------- errors ----

/// What is wrong with the document itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DocumentProblem {
    #[error("the document is empty")]
    Empty,
    #[error("the document is larger than {max_bytes} bytes")]
    TooLarge { max_bytes: u64 },
    #[error("the document is not a supported type")]
    UnsupportedType,
    #[error("the document is encrypted")]
    Encrypted,
    #[error("the document could not be read")]
    Unreadable,
    #[error("the document has more than {max_pages} pages")]
    TooManyPages { max_pages: u32 },
    /// The pages asked for run past the end of the document.
    #[error("the document has only {pages} pages")]
    PageOutOfRange { pages: u32 },
    /// A page holds more text than the worker will report.
    #[error("a page holds more text than the worker will report")]
    PageTooLarge,
    /// A page is too big to draw (too long on a side, or too many pixels).
    #[error("a page is too large to draw")]
    PageTooBig,
    /// A picture has too many pixels to be opened safely.
    #[error("the picture is too large")]
    ImageTooLarge,
}

/// Why a document could not be processed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ProcessError {
    /// The worker could not be reached (nothing listening, name not found,
    /// connection refused).
    #[error("the document worker could not be reached")]
    Unavailable,
    /// Our token was refused. A configuration problem, not the document's fault.
    #[error("the document worker refused our credentials")]
    Auth,
    /// The worker is running but does not offer this (OCR is off unless a program is named).
    #[error("the document worker does not offer this")]
    NotAvailable,
    #[error("the document worker timed out")]
    Timeout,
    /// The connection broke after it was made (the worker died mid-request).
    #[error("the connection to the document worker was interrupted")]
    Interrupted,
    /// The worker itself failed.
    #[error("the document worker failed (HTTP {status})")]
    Server { status: u16 },
    /// An answer we have no meaning for (a wrong address, a rate limit, a
    /// redirect): a configuration or contract problem, not the document's fault.
    #[error("the document worker answered unexpectedly (HTTP {status})")]
    Unexpected { status: u16 },
    /// A success answer that does not make sense. The reason is fixed text.
    #[error("the document worker's answer was not understood: {0}")]
    BadResponse(&'static str),
    /// The worker looked at the document and refused it.
    #[error(transparent)]
    Document(DocumentProblem),
}

impl ProcessError {
    /// What the job queue should do about it (ADR-0008, ADR-0009).
    pub fn failure_kind(&self) -> FailureKind {
        match self {
            ProcessError::Unavailable
            | ProcessError::Auth
            | ProcessError::NotAvailable
            | ProcessError::Unexpected { .. } => FailureKind::ProviderUnavailable,
            ProcessError::Timeout
            | ProcessError::Interrupted
            | ProcessError::Server { .. }
            | ProcessError::BadResponse(_) => FailureKind::Transient,
            ProcessError::Document(_) => FailureKind::NeedsReview,
        }
    }

    /// True when the document itself is the problem, so a person must look.
    pub fn is_about_the_document(&self) -> bool {
        matches!(self, ProcessError::Document(_))
    }
}

// -------------------------------------------------------------------- trait ----

/// Something that can look inside a document.
///
/// (`impl Future` in the return position lets implementations write
/// `async`-style code without boxing; the same style as `ObjectStore`.)
pub trait DocumentProcessor: Send + Sync + 'static {
    /// How many pages, and how many of them have a text layer.
    fn inspect(&self, document: Bytes) -> impl Future<Output = Result<Inspection, ProcessError>> + Send;

    /// The text, size and fonts of a range of pages (at most [`PageRange::MAX_PAGES`]),
    /// and, if `words` is true, where each word is.
    fn page_texts(
        &self,
        document: Bytes,
        range: PageRange,
        words: bool,
    ) -> impl Future<Output = Result<Vec<PageContent>, ProcessError>> + Send;

    /// One page read by OCR (English and Afrikaans), with its words and their confidence;
    /// with `agreement`, also how much of the page's text layer OCR read back.
    fn ocr_page(
        &self,
        document: Bytes,
        page: NonZeroU32,
        agreement: bool,
    ) -> impl Future<Output = Result<PageContent, ProcessError>> + Send;

    /// One page drawn as a picture that holds nothing but pixels.
    fn render_page(
        &self,
        document: Bytes,
        page: NonZeroU32,
        options: RenderOptions,
    ) -> impl Future<Output = Result<RenderedPage, ProcessError>> + Send;

    /// A PNG or JPEG with `boxes` painted solid black; the result holds nothing but pixels.
    fn mask_image(
        &self,
        image: Bytes,
        boxes: Vec<MaskBox>,
        format: ImageFormat,
        max_edge: u16,
    ) -> impl Future<Output = Result<RenderedPage, ProcessError>> + Send;
}

// --------------------------------------------------------------------- fake ----

type Answerer = Box<dyn Fn(&Bytes) -> Result<Inspection, ProcessError> + Send + Sync>;

/// A call to [`DocumentProcessor::page_texts`], as the fake saw it.
#[derive(Debug, Clone, PartialEq)]
pub struct TextCall {
    pub document: Bytes,
    pub range: PageRange,
    pub words: bool,
}

/// A call to [`DocumentProcessor::ocr_page`], as the fake saw it.
#[derive(Debug, Clone, PartialEq)]
pub struct OcrCall {
    pub document: Bytes,
    pub page: NonZeroU32,
    pub agreement: bool,
}

/// A call to [`DocumentProcessor::render_page`], as the fake saw it.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderCall {
    pub document: Bytes,
    pub page: NonZeroU32,
    pub options: RenderOptions,
}

/// A call to [`DocumentProcessor::mask_image`], as the fake saw it.
#[derive(Debug, Clone, PartialEq)]
pub struct MaskCall {
    pub image: Bytes,
    pub boxes: Vec<MaskBox>,
    pub format: ImageFormat,
    pub max_edge: u16,
}

type Scripted<T> = VecDeque<Result<T, ProcessError>>;

#[derive(Default)]
struct FakeState {
    script: Scripted<Inspection>,
    answerer: Option<Answerer>,
    documents: Vec<Bytes>,
    texts: Scripted<Vec<PageContent>>,
    ocrs: Scripted<PageContent>,
    renders: Scripted<RenderedPage>,
    masks: Scripted<RenderedPage>,
    text_calls: Vec<TextCall>,
    ocr_calls: Vec<OcrCall>,
    render_calls: Vec<RenderCall>,
    mask_calls: Vec<MaskCall>,
}

/// A worker for tests: no Python, no network. It gives scripted answers in
/// order, or computes each inspection from the document, and remembers every call
/// it was given. Clones share one script and one record.
#[derive(Clone)]
pub struct FakeWorker {
    state: Arc<Mutex<FakeState>>,
}

impl FakeWorker {
    /// Answers inspections with these, one per call, in order. When they run out, every call
    /// is [`ProcessError::Unexpected`] with status 0, so a test that makes more
    /// calls than it expected fails loudly. (The other calls are scripted with the
    /// `with_` methods and behave the same way.)
    pub fn scripted(answers: Vec<Result<Inspection, ProcessError>>) -> Self {
        FakeWorker::with_state(FakeState {
            script: answers.into(),
            ..FakeState::default()
        })
    }

    /// Answers each inspection by running `answerer` on the document.
    pub fn answering(
        answerer: impl Fn(&Bytes) -> Result<Inspection, ProcessError> + Send + Sync + 'static,
    ) -> Self {
        FakeWorker::with_state(FakeState {
            answerer: Some(Box::new(answerer)),
            ..FakeState::default()
        })
    }

    fn with_state(state: FakeState) -> Self {
        FakeWorker {
            state: Arc::new(Mutex::new(state)),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, FakeState> {
        // A test that panicked while holding the lock must not hide the rest.
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Scripts the answers to [`DocumentProcessor::page_texts`].
    pub fn with_texts(self, answers: Vec<Result<Vec<PageContent>, ProcessError>>) -> Self {
        self.lock().texts = answers.into();
        self
    }

    /// Scripts the answers to [`DocumentProcessor::ocr_page`].
    pub fn with_ocr(self, answers: Vec<Result<PageContent, ProcessError>>) -> Self {
        self.lock().ocrs = answers.into();
        self
    }

    /// Scripts the answers to [`DocumentProcessor::render_page`].
    pub fn with_renders(self, answers: Vec<Result<RenderedPage, ProcessError>>) -> Self {
        self.lock().renders = answers.into();
        self
    }

    /// Scripts the answers to [`DocumentProcessor::mask_image`].
    pub fn with_masks(self, answers: Vec<Result<RenderedPage, ProcessError>>) -> Self {
        self.lock().masks = answers.into();
        self
    }

    /// How many calls of any kind have been made, answered or not.
    pub fn call_count(&self) -> usize {
        let state = self.lock();
        state.documents.len()
            + state.text_calls.len()
            + state.ocr_calls.len()
            + state.render_calls.len()
            + state.mask_calls.len()
    }

    /// Every document received by [`DocumentProcessor::inspect`], in order.
    pub fn documents(&self) -> Vec<Bytes> {
        self.lock().documents.clone()
    }

    pub fn text_calls(&self) -> Vec<TextCall> {
        self.lock().text_calls.clone()
    }

    pub fn ocr_calls(&self) -> Vec<OcrCall> {
        self.lock().ocr_calls.clone()
    }

    pub fn render_calls(&self) -> Vec<RenderCall> {
        self.lock().render_calls.clone()
    }

    pub fn mask_calls(&self) -> Vec<MaskCall> {
        self.lock().mask_calls.clone()
    }
}

fn next<T>(script: &mut Scripted<T>) -> Result<T, ProcessError> {
    script
        .pop_front()
        .unwrap_or(Err(ProcessError::Unexpected { status: 0 }))
}

impl DocumentProcessor for FakeWorker {
    fn inspect(&self, document: Bytes) -> impl Future<Output = Result<Inspection, ProcessError>> + Send {
        let result = {
            let mut state = self.lock();
            let result = match &state.answerer {
                Some(answerer) => answerer(&document),
                None => next(&mut state.script),
            };
            state.documents.push(document);
            result
        };
        std::future::ready(result)
    }

    fn page_texts(
        &self,
        document: Bytes,
        range: PageRange,
        words: bool,
    ) -> impl Future<Output = Result<Vec<PageContent>, ProcessError>> + Send {
        let mut state = self.lock();
        state.text_calls.push(TextCall {
            document,
            range,
            words,
        });
        std::future::ready(next(&mut state.texts))
    }

    fn ocr_page(
        &self,
        document: Bytes,
        page: NonZeroU32,
        agreement: bool,
    ) -> impl Future<Output = Result<PageContent, ProcessError>> + Send {
        let mut state = self.lock();
        state.ocr_calls.push(OcrCall {
            document,
            page,
            agreement,
        });
        std::future::ready(next(&mut state.ocrs))
    }

    fn render_page(
        &self,
        document: Bytes,
        page: NonZeroU32,
        options: RenderOptions,
    ) -> impl Future<Output = Result<RenderedPage, ProcessError>> + Send {
        let mut state = self.lock();
        state.render_calls.push(RenderCall {
            document,
            page,
            options,
        });
        std::future::ready(next(&mut state.renders))
    }

    fn mask_image(
        &self,
        image: Bytes,
        boxes: Vec<MaskBox>,
        format: ImageFormat,
        max_edge: u16,
    ) -> impl Future<Output = Result<RenderedPage, ProcessError>> + Send {
        let mut state = self.lock();
        state.mask_calls.push(MaskCall {
            image,
            boxes,
            format,
            max_edge,
        });
        std::future::ready(next(&mut state.masks))
    }
}

#[cfg(test)]
mod tests;
