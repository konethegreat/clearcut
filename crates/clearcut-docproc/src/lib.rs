// Written by Kone Tshivhinda & Claude
//! Reading documents (ADR-0009).
//!
//! The Python worker is the one place that opens a hostile file. This crate is
//! the Rust side of that boundary:
//!
//! - [`DocumentProcessor`] is the trait the pipeline is written against;
//! - [`HttpWorker`] is its implementation: an HTTP client for the worker;
//! - [`FakeWorker`] is a scripted stand-in, so the pipeline can be developed and
//!   tested with no Python and no network.
//!
//! Every way a call can go wrong is a [`ProcessError`], and
//! [`ProcessError::failure_kind`] says what the job queue should do about it
//! (ADR-0008): an unreachable or misconfigured worker is the platform's problem
//! and does not use up a document's attempts; a timeout or a server error is
//! retried; a problem with the document itself (encrypted, unreadable, too big)
//! goes to a person.
//!
//! Nothing here prints or returns what the worker said: error values carry fixed
//! reasons and numeric limits, never text from the document or the server.

use bytes::Bytes;
use clearcut_core::jobs::FailureKind;
use std::collections::VecDeque;
use std::future::Future;
use std::sync::{Arc, Mutex};

mod http;

pub use http::{ConfigError, HttpWorker, DEFAULT_TIMEOUT, MAX_RESPONSE_BYTES};

// ------------------------------------------------------------------ results ----

/// What kind of document the worker found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentKind {
    Pdf,
}

impl DocumentKind {
    pub fn media_type(self) -> &'static str {
        match self {
            DocumentKind::Pdf => "application/pdf",
        }
    }

    /// Exact match only: no case folding, no parameters.
    pub fn parse(text: &str) -> Option<DocumentKind> {
        match text {
            "application/pdf" => Some(DocumentKind::Pdf),
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
}

/// Why a document could not be inspected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ProcessError {
    /// The worker could not be reached (nothing listening, name not found,
    /// connection refused).
    #[error("the document worker could not be reached")]
    Unavailable,
    /// Our token was refused. A configuration problem, not the document's fault.
    #[error("the document worker refused our credentials")]
    Auth,
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
            ProcessError::Unavailable | ProcessError::Auth | ProcessError::Unexpected { .. } => {
                FailureKind::ProviderUnavailable
            }
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
    fn inspect(&self, document: Bytes) -> impl Future<Output = Result<Inspection, ProcessError>> + Send;
}

// --------------------------------------------------------------------- fake ----

type Answerer = Box<dyn Fn(&Bytes) -> Result<Inspection, ProcessError> + Send + Sync>;

struct FakeState {
    script: VecDeque<Result<Inspection, ProcessError>>,
    answerer: Option<Answerer>,
    documents: Vec<Bytes>,
}

/// A worker for tests: no Python, no network. It gives scripted answers in
/// order, or computes each from the document, and remembers every document it
/// was given. Clones share one script and one record.
#[derive(Clone)]
pub struct FakeWorker {
    state: Arc<Mutex<FakeState>>,
}

impl FakeWorker {
    /// Answers with these, one per call, in order. When they run out, every call
    /// is [`ProcessError::Unexpected`] with status 0, so a test that makes more
    /// calls than it expected fails loudly.
    pub fn scripted(answers: Vec<Result<Inspection, ProcessError>>) -> Self {
        FakeWorker::with_state(FakeState {
            script: answers.into(),
            answerer: None,
            documents: Vec::new(),
        })
    }

    /// Answers each call by running `answerer` on the document.
    pub fn answering(
        answerer: impl Fn(&Bytes) -> Result<Inspection, ProcessError> + Send + Sync + 'static,
    ) -> Self {
        FakeWorker::with_state(FakeState {
            script: VecDeque::new(),
            answerer: Some(Box::new(answerer)),
            documents: Vec::new(),
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

    /// How many calls have been made, answered or not.
    pub fn call_count(&self) -> usize {
        self.lock().documents.len()
    }

    /// Every document received, in order.
    pub fn documents(&self) -> Vec<Bytes> {
        self.lock().documents.clone()
    }
}

impl DocumentProcessor for FakeWorker {
    fn inspect(&self, document: Bytes) -> impl Future<Output = Result<Inspection, ProcessError>> + Send {
        let result = {
            let mut state = self.lock();
            let result = match &state.answerer {
                Some(answerer) => answerer(&document),
                None => state
                    .script
                    .pop_front()
                    .unwrap_or(Err(ProcessError::Unexpected { status: 0 })),
            };
            state.documents.push(document);
            result
        };
        std::future::ready(result)
    }
}

#[cfg(test)]
mod tests;
