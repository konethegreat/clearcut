// Written by Kone Tshivhinda & Claude
//! The model provider (ADR-0004 #6, ADR-0005, ADR-0009).
//!
//! Everything that asks a language model a question goes through the
//! [`ModelProvider`] trait, so the backend can be swapped (Anthropic today, a
//! self-hosted model later if POPIA or a customer requires it) and so the whole
//! pipeline can be tested with a [`FakeProvider`], with no network and no cost.
//! The real Anthropic adapter is a separate task (P2-13); this crate is the
//! contract it must meet.
//!
//! Design rules, each with a test:
//!
//! - **A request is checked when it is built**, against the limits the API
//!   really has (image size and count, at most four cache breakpoints, an output
//!   cap), so a request that could never be sent fails here, in our code, with a
//!   clear reason, instead of as an opaque 400 from the provider after a
//!   round trip. A request that fails the provider's check anyway is
//!   [`ProviderError::Rejected`]: it means we have a bug.
//! - **Nothing prints invoice content.** The text and images in a request, and
//!   the data in a response, are invoice content (personal and financial
//!   information under POPIA). Their `Debug` output shows sizes only, so a
//!   stray `{:?}` in a log line cannot leak a document.
//! - **A call that was billed is counted even when it failed.** A truncated,
//!   refused or invalid answer still cost tokens, so those errors carry their
//!   [`Usage`]; budgets (P2-12) add them up.
//! - **Errors say what to do next.** [`ProviderError::failure_kind`] maps each
//!   one onto the job queue's failure kinds (ADR-0008): an outage does not use up
//!   attempts, a timeout is retried, a refusal goes to a person.
//!
//! The model's output is **untrusted data**: it is whatever the model produced
//! from a document that may contain instructions meant for the model. It is
//! validated by `clearcut-core` before anything relies on it.

use clearcut_core::jobs::FailureKind;
use serde_json::Value;
use std::collections::VecDeque;
use std::fmt;
use std::future::Future;
use std::iter::Sum;
use std::ops::{Add, AddAssign};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use uuid::Uuid;

// ------------------------------------------------------------------- limits ----

/// The most bytes in one image (the API's own limit is 5 MB per image).
pub const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;
/// The most images in one request. Per-page calls need one or a few; this is a
/// guard against a bug, well inside the API's limit.
pub const MAX_IMAGES: usize = 20;
/// The most cache breakpoints in one request (the API allows four).
pub const MAX_CACHE_BREAKPOINTS: usize = 4;
/// The most text and image bytes in one request (the API's limit is 32 MB).
pub const MAX_REQUEST_BYTES: usize = 25 * 1024 * 1024;
/// The largest output cap a request may ask for.
pub const MAX_OUTPUT_TOKENS: u32 = 64_000;
/// The longest model id.
const MAX_MODEL_ID_CHARS: usize = 128;

// -------------------------------------------------------------- request parts ----

/// How hard the model should think (ADR-0005 §3): `medium` for tiers 1 and 2,
/// `high` for tiers 3 and 4.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effort {
    Low,
    Medium,
    High,
}

impl Effort {
    /// The name the API uses.
    pub fn as_str(self) -> &'static str {
        match self {
            Effort::Low => "low",
            Effort::Medium => "medium",
            Effort::High => "high",
        }
    }
}

/// Whether the model may think before it answers. Always the adaptive kind:
/// the old fixed-budget mode is rejected by current models (ADR-0005 §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Thinking {
    Off,
    Adaptive,
}

/// The image formats we send. We render page images ourselves, so two are enough.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageType {
    Png,
    Jpeg,
}

impl ImageType {
    pub fn media_type(self) -> &'static str {
        match self {
            ImageType::Png => "image/png",
            ImageType::Jpeg => "image/jpeg",
        }
    }

    /// Exact match only: no case folding, no parameters.
    pub fn parse(text: &str) -> Option<ImageType> {
        match text {
            "image/png" => Some(ImageType::Png),
            "image/jpeg" => Some(ImageType::Jpeg),
            _ => None,
        }
    }
}

/// A piece of text. `cache` marks the end of a static prefix (instructions,
/// schema, SA VAT rules, examples) that the provider may cache between calls
/// (ADR-0005 §4). Its `Debug` output shows only the size.
#[derive(Clone, PartialEq, Eq)]
pub struct TextBlock {
    text: String,
    cache: bool,
}

impl TextBlock {
    pub fn plain(text: impl Into<String>) -> Self {
        TextBlock {
            text: text.into(),
            cache: false,
        }
    }

    /// Text that ends a cacheable prefix. The provider caches everything up to
    /// and including this block.
    pub fn cached(text: impl Into<String>) -> Self {
        TextBlock {
            text: text.into(),
            cache: true,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_cache_breakpoint(&self) -> bool {
        self.cache
    }
}

impl fmt::Debug for TextBlock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TextBlock({} bytes, cache: {})", self.text.len(), self.cache)
    }
}

/// One piece of what the model is shown.
#[derive(Clone, PartialEq, Eq)]
pub enum Block {
    Text(TextBlock),
    Image { media: ImageType, bytes: Vec<u8> },
}

impl Block {
    pub fn text(text: impl Into<String>) -> Self {
        Block::Text(TextBlock::plain(text))
    }

    /// Text that ends a cacheable prefix.
    pub fn cached_text(text: impl Into<String>) -> Self {
        Block::Text(TextBlock::cached(text))
    }

    pub fn image(media: ImageType, bytes: Vec<u8>) -> Self {
        Block::Image { media, bytes }
    }

    fn byte_len(&self) -> usize {
        match self {
            Block::Text(text) => text.text.len(),
            Block::Image { bytes, .. } => bytes.len(),
        }
    }
}

impl fmt::Debug for Block {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Block::Text(text) => write!(f, "{text:?}"),
            Block::Image { media, bytes } => {
                write!(f, "Image({}, {} bytes)", media.media_type(), bytes.len())
            }
        }
    }
}

/// What a call is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Extraction,
    Narration,
}

/// Who and what a call is for. Used only for logging and budgets (ADR-0005 §5);
/// it is never sent to the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallContext {
    pub tenant: Uuid,
    pub job: Option<Uuid>,
    pub document: Option<Uuid>,
    pub page: Option<u32>,
    pub purpose: Purpose,
}

// -------------------------------------------------------------------- request ----

/// Why a request could not be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RequestError {
    #[error("the model id must be 1 to 128 letters, digits, '.', '_', ':' or '-'")]
    BadModel,
    #[error("a request needs something for the model to read")]
    NoContent,
    #[error("text must not be blank")]
    BlankText,
    #[error("a request needs an output schema")]
    NoSchema,
    #[error("the output schema must be a JSON object")]
    SchemaNotAnObject,
    #[error("the output token cap must be between 1 and 64000")]
    BadTokenCap,
    #[error("an image must have bytes")]
    EmptyImage,
    #[error("an image is larger than 5 MiB")]
    ImageTooLarge,
    #[error("a request carries at most 20 images")]
    TooManyImages,
    #[error("a request has at most 4 cache breakpoints")]
    TooManyCacheBreakpoints,
    #[error("the request is larger than 25 MiB")]
    TooLarge,
}

/// A checked request to a model. Build one with [`ModelRequest::builder`].
#[derive(Clone, PartialEq)]
pub struct ModelRequest {
    model: String,
    effort: Effort,
    thinking: Thinking,
    system: Vec<TextBlock>,
    content: Vec<Block>,
    output_schema: Value,
    max_output_tokens: u32,
    call: CallContext,
}

impl ModelRequest {
    /// Starts a request for `model` (an id such as `claude-haiku-5-5`; which
    /// model to use for which tier is the router's decision, P2-14).
    pub fn builder(model: impl Into<String>, call: CallContext) -> RequestBuilder {
        RequestBuilder {
            model: model.into(),
            call,
            effort: Effort::Medium,
            thinking: Thinking::Adaptive,
            system: Vec::new(),
            content: Vec::new(),
            output_schema: None,
            max_output_tokens: None,
        }
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn effort(&self) -> Effort {
        self.effort
    }

    pub fn thinking(&self) -> Thinking {
        self.thinking
    }

    /// The instructions, in order. Always text.
    pub fn system(&self) -> &[TextBlock] {
        &self.system
    }

    /// What the model is shown, in order.
    pub fn content(&self) -> &[Block] {
        &self.content
    }

    /// The JSON Schema the answer must follow.
    pub fn output_schema(&self) -> &Value {
        &self.output_schema
    }

    pub fn max_output_tokens(&self) -> u32 {
        self.max_output_tokens
    }

    pub fn call(&self) -> &CallContext {
        &self.call
    }

    fn system_bytes(&self) -> usize {
        self.system.iter().map(|block| block.text.len()).sum()
    }

    fn content_bytes(&self) -> usize {
        self.content.iter().map(Block::byte_len).sum()
    }
}

impl fmt::Debug for ModelRequest {
    /// Sizes only: the text and images are invoice content.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let images = self
            .content
            .iter()
            .filter(|block| matches!(block, Block::Image { .. }))
            .count();
        f.debug_struct("ModelRequest")
            .field("model", &self.model)
            .field("effort", &self.effort)
            .field("thinking", &self.thinking)
            .field("max_output_tokens", &self.max_output_tokens)
            .field(
                "system",
                &format_args!("{} blocks, {} bytes", self.system.len(), self.system_bytes()),
            )
            .field(
                "content",
                &format_args!(
                    "{} blocks ({} images), {} bytes",
                    self.content.len(),
                    images,
                    self.content_bytes()
                ),
            )
            .field("call", &self.call)
            .finish()
    }
}

/// Assembles a [`ModelRequest`]. Nothing is checked until [`build`](Self::build).
///
/// (A *builder* is a common Rust pattern: each method takes `self` and returns
/// it, so calls chain, and the final `build()` checks everything at once.)
#[derive(Clone)]
pub struct RequestBuilder {
    model: String,
    call: CallContext,
    effort: Effort,
    thinking: Thinking,
    system: Vec<TextBlock>,
    content: Vec<Block>,
    output_schema: Option<Value>,
    max_output_tokens: Option<u32>,
}

impl RequestBuilder {
    pub fn effort(mut self, effort: Effort) -> Self {
        self.effort = effort;
        self
    }

    pub fn thinking(mut self, thinking: Thinking) -> Self {
        self.thinking = thinking;
        self
    }

    /// Adds instructions.
    pub fn system(mut self, text: impl Into<String>) -> Self {
        self.system.push(TextBlock::plain(text));
        self
    }

    /// Adds instructions that end a cacheable prefix.
    pub fn cached_system(mut self, text: impl Into<String>) -> Self {
        self.system.push(TextBlock::cached(text));
        self
    }

    /// Adds something for the model to read: text or an image.
    pub fn content(mut self, block: Block) -> Self {
        self.content.push(block);
        self
    }

    pub fn output_schema(mut self, schema: Value) -> Self {
        self.output_schema = Some(schema);
        self
    }

    pub fn max_output_tokens(mut self, tokens: u32) -> Self {
        self.max_output_tokens = Some(tokens);
        self
    }

    /// Checks everything and returns the request, or the first problem found.
    pub fn build(self) -> Result<ModelRequest, RequestError> {
        let model_ok = !self.model.is_empty()
            && self.model.chars().count() <= MAX_MODEL_ID_CHARS
            && self
                .model
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-'));
        if !model_ok {
            return Err(RequestError::BadModel);
        }
        if self.content.is_empty() {
            return Err(RequestError::NoContent);
        }
        let blank = |text: &str| text.trim().is_empty();
        if self.system.iter().any(|block| blank(&block.text))
            || self
                .content
                .iter()
                .any(|block| matches!(block, Block::Text(text) if blank(&text.text)))
        {
            return Err(RequestError::BlankText);
        }
        let output_schema = self.output_schema.ok_or(RequestError::NoSchema)?;
        if !output_schema.is_object() {
            return Err(RequestError::SchemaNotAnObject);
        }
        let max_output_tokens = match self.max_output_tokens {
            Some(tokens) if (1..=MAX_OUTPUT_TOKENS).contains(&tokens) => tokens,
            _ => return Err(RequestError::BadTokenCap),
        };

        let mut images = 0;
        for block in &self.content {
            if let Block::Image { bytes, .. } = block {
                if bytes.is_empty() {
                    return Err(RequestError::EmptyImage);
                }
                if bytes.len() > MAX_IMAGE_BYTES {
                    return Err(RequestError::ImageTooLarge);
                }
                images += 1;
            }
        }
        if images > MAX_IMAGES {
            return Err(RequestError::TooManyImages);
        }

        let breakpoints = self.system.iter().filter(|block| block.cache).count()
            + self
                .content
                .iter()
                .filter(|block| matches!(block, Block::Text(text) if text.cache))
                .count();
        if breakpoints > MAX_CACHE_BREAKPOINTS {
            return Err(RequestError::TooManyCacheBreakpoints);
        }

        // Text counts in bytes, not characters, and the (small) schema is not counted.
        let total: usize = self.system.iter().map(|block| block.text.len()).sum::<usize>()
            + self.content.iter().map(Block::byte_len).sum::<usize>();
        if total > MAX_REQUEST_BYTES {
            return Err(RequestError::TooLarge);
        }

        Ok(ModelRequest {
            model: self.model,
            effort: self.effort,
            thinking: self.thinking,
            system: self.system,
            content: self.content,
            output_schema,
            max_output_tokens,
            call: self.call,
        })
    }
}

// ------------------------------------------------------------------- response ----

/// Tokens used by one call, as the provider reports them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Input read from the cache (billed at a fraction of the normal price).
    pub cache_read_tokens: u64,
    /// Input written to the cache (billed at a premium).
    pub cache_write_tokens: u64,
}

impl Usage {
    /// All four counts added up. Saturates rather than wrapping around.
    pub fn total(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.output_tokens)
            .saturating_add(self.cache_read_tokens)
            .saturating_add(self.cache_write_tokens)
    }
}

impl Add for Usage {
    type Output = Usage;

    fn add(self, other: Usage) -> Usage {
        Usage {
            input_tokens: self.input_tokens.saturating_add(other.input_tokens),
            output_tokens: self.output_tokens.saturating_add(other.output_tokens),
            cache_read_tokens: self.cache_read_tokens.saturating_add(other.cache_read_tokens),
            cache_write_tokens: self.cache_write_tokens.saturating_add(other.cache_write_tokens),
        }
    }
}

impl AddAssign for Usage {
    fn add_assign(&mut self, other: Usage) {
        *self = *self + other;
    }
}

impl Sum for Usage {
    fn sum<I: Iterator<Item = Usage>>(iter: I) -> Usage {
        iter.fold(Usage::default(), |total, usage| total + usage)
    }
}

/// A model's answer. `output` is **untrusted**: validate it before relying on it.
#[derive(Clone, PartialEq)]
pub struct ModelResponse {
    /// The structured answer, as the model produced it.
    pub output: Value,
    pub usage: Usage,
    /// The model that actually answered.
    pub model: String,
    /// The provider's id for the call, for support and audit.
    pub request_id: Option<String>,
}

impl fmt::Debug for ModelResponse {
    /// The answer is invoice data, so only its size is shown.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModelResponse")
            .field("model", &self.model)
            .field("usage", &self.usage)
            .field("request_id", &self.request_id)
            .field(
                "output",
                &format_args!("<{} bytes>", self.output.to_string().len()),
            )
            .finish()
    }
}

// --------------------------------------------------------------------- errors ----

/// Why a call did not produce a usable answer. None of the messages contain
/// invoice content: a `reason` states which rule failed, never what the
/// document said.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    /// Overloaded, rate-limited, or unreachable. Not the document's fault.
    #[error("the model provider is unavailable")]
    Unavailable { retry_after: Option<Duration> },
    #[error("the model provider timed out")]
    Timeout,
    /// The answer hit the output cap and was cut off. Billed.
    #[error("the model's output was cut off at the token limit")]
    Truncated { usage: Usage },
    /// The model declined to answer. Billed.
    #[error("the model declined to answer")]
    Refused { usage: Usage },
    /// The answer did not follow the schema, or was not JSON. Billed.
    #[error("the model's output was not valid: {reason}")]
    InvalidOutput { usage: Usage, reason: String },
    /// The provider rejected our request: a bug on our side.
    #[error("the model provider rejected the request: {0}")]
    Rejected(String),
    /// Our credentials were refused (a missing, revoked or wrong key).
    #[error("the model provider refused our credentials")]
    Auth,
}

impl ProviderError {
    /// What the job queue should do about it (ADR-0008, ADR-0009):
    /// an outage or a bad key is the platform's problem, not the document's, so
    /// it does not use up attempts; a timeout, a cut-off answer or a malformed
    /// one is worth retrying (the router also escalates the tier, P2-14); a
    /// refusal, or a request we got wrong, needs a person.
    pub fn failure_kind(&self) -> FailureKind {
        match self {
            ProviderError::Unavailable { .. } | ProviderError::Auth => FailureKind::ProviderUnavailable,
            ProviderError::Timeout
            | ProviderError::Truncated { .. }
            | ProviderError::InvalidOutput { .. } => FailureKind::Transient,
            ProviderError::Refused { .. } | ProviderError::Rejected(_) => FailureKind::NeedsReview,
        }
    }

    /// The tokens this failed call was billed for, if it was billed.
    pub fn usage(&self) -> Option<Usage> {
        match self {
            ProviderError::Truncated { usage }
            | ProviderError::Refused { usage }
            | ProviderError::InvalidOutput { usage, .. } => Some(*usage),
            _ => None,
        }
    }

    /// How long the provider asked us to wait, when it said.
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            ProviderError::Unavailable { retry_after } => *retry_after,
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------- trait ----

/// A language-model backend. The pipeline is written against this trait, so it
/// can run against the Anthropic adapter, a self-hosted model, or a
/// [`FakeProvider`].
///
/// (`impl Future` in the return position lets implementations write
/// `async fn`-style code without boxing; the same style as `ObjectStore`.)
pub trait ModelProvider: Send + Sync + 'static {
    fn complete(
        &self,
        request: ModelRequest,
    ) -> impl Future<Output = Result<ModelResponse, ProviderError>> + Send;
}

// ----------------------------------------------------------------------- fake ----

type Answerer = Box<dyn Fn(&ModelRequest) -> Result<ModelResponse, ProviderError> + Send + Sync>;

struct FakeState {
    script: VecDeque<Result<ModelResponse, ProviderError>>,
    answerer: Option<Answerer>,
    requests: Vec<ModelRequest>,
}

/// A provider for tests: no network, no cost. It gives scripted answers in
/// order, or computes each from the request, and remembers every request it was
/// asked. Clones share one script and one record.
#[derive(Clone)]
pub struct FakeProvider {
    state: Arc<Mutex<FakeState>>,
}

impl FakeProvider {
    /// Answers with these, one per call, in order. When they run out, every call
    /// is [`ProviderError::Rejected`], so a test that makes more calls than it
    /// expected fails loudly.
    pub fn scripted(answers: Vec<Result<ModelResponse, ProviderError>>) -> Self {
        FakeProvider::with_state(FakeState {
            script: answers.into(),
            answerer: None,
            requests: Vec::new(),
        })
    }

    /// Answers each call by running `answerer` on the request.
    pub fn answering(
        answerer: impl Fn(&ModelRequest) -> Result<ModelResponse, ProviderError> + Send + Sync + 'static,
    ) -> Self {
        FakeProvider::with_state(FakeState {
            script: VecDeque::new(),
            answerer: Some(Box::new(answerer)),
            requests: Vec::new(),
        })
    }

    fn with_state(state: FakeState) -> Self {
        FakeProvider {
            state: Arc::new(Mutex::new(state)),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, FakeState> {
        // A test that panicked while holding the lock must not hide the rest.
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// How many calls have been made, answered or not.
    pub fn call_count(&self) -> usize {
        self.lock().requests.len()
    }

    /// Every request received, in order.
    pub fn requests(&self) -> Vec<ModelRequest> {
        self.lock().requests.clone()
    }
}

impl ModelProvider for FakeProvider {
    fn complete(
        &self,
        request: ModelRequest,
    ) -> impl Future<Output = Result<ModelResponse, ProviderError>> + Send {
        let result = {
            let mut state = self.lock();
            let result = match &state.answerer {
                Some(answerer) => answerer(&request),
                None => state.script.pop_front().unwrap_or_else(|| {
                    Err(ProviderError::Rejected(
                        "the fake provider has no more scripted answers".into(),
                    ))
                }),
            };
            state.requests.push(request);
            result
        };
        std::future::ready(result)
    }
}

#[cfg(test)]
mod tests;
