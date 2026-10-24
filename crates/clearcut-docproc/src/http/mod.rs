// Written by Kone Tshivhinda & Claude
//! The HTTP client for the Python document worker (ADR-0009, ADR-0016).
//!
//! What it will and will not do, each with a test:
//!
//! - **Plain `http://` only, no credentials in the address.** The worker lives on
//!   a private network next to this service; the bearer token travels in a header
//!   on that network. (If the worker is ever reached across an untrusted network,
//!   TLS is added here, deliberately, not by accident.)
//! - **Never follows a redirect and never uses a proxy from the environment**, so
//!   the bearer token cannot be handed to anyone but the configured worker.
//! - **Limits what it reads back.** An error answer is a few hundred bytes and at most
//!   64 KiB is read of one. A good answer has its own limit: 64 KiB for an inspection,
//!   32 MiB for the text of pages, 6 MiB for a picture. A worker cannot make this
//!   service buffer more than that.
//! - **Believes nothing it has not checked.** Pages and pictures are rebuilt through
//!   types that only hold possible values, and an answer that does not fit is a bad
//!   response.
//! - **Maps every outcome onto a [`ProcessError`]** without repeating anything
//!   the server said: only fixed reasons and numeric limits.
//! - **Never prints the token or the address.**

mod wire;

use crate::page::RenderedPage;
use crate::request::{ImageFormat, MaskBox, PageRange, RenderOptions, MAX_BOXES};
use crate::{DocumentKind, DocumentProblem, DocumentProcessor, Inspection, PageContent, ProcessError};
use base64::Engine as _;
use bytes::Bytes;
use clearcut_provider::ImageType;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use reqwest::redirect::Policy;
use reqwest::{Client, Url};
use serde::Deserialize;
use serde_json::{json, Value};
use std::fmt;
use std::num::NonZeroU32;
use std::time::Duration;

/// How long a call may take before the client gives up. The worker gives up on a
/// document after 30 s plus a 2 s grace, so the client waits longer than that.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(45);
/// The most of an error answer, or of an inspection, that is read.
pub const MAX_RESPONSE_BYTES: usize = 64 * 1024;
/// The most of the text of pages that is read (a page's text is at most a mebibyte and
/// a call reads at most fifty pages; the real answers are far smaller).
pub const MAX_TEXT_RESPONSE_BYTES: usize = 32 * 1024 * 1024;
/// The most of a picture that is read (the worker keeps its pictures under 5 MiB).
pub const MAX_IMAGE_RESPONSE_BYTES: usize = 6 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Why an [`HttpWorker`] could not be set up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    #[error("the worker address is not a valid URL")]
    BadUrl,
    #[error("the worker address must start with http://")]
    NotHttp,
    #[error("the worker address must not contain a user name or password")]
    HasCredentials,
    #[error("the worker address must not contain a query or fragment")]
    HasQuery,
    #[error("the worker token must be visible characters with no spaces")]
    BadToken,
    #[error("the timeout must be more than zero")]
    BadTimeout,
    #[error("the HTTP client could not be built")]
    Client,
}

/// Talks to the Python worker.
#[derive(Clone)]
pub struct HttpWorker {
    client: Client,
    base: Url,
    authorization: HeaderValue,
    timeout: Duration,
}

impl fmt::Debug for HttpWorker {
    /// The address and the token are not shown.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpWorker")
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

impl HttpWorker {
    /// `base_url` is where the worker listens (`http://worker:8090`, optionally
    /// with a path prefix); `token` is the shared secret (`CLEARCUT_WORKER_TOKEN`).
    pub fn new(base_url: &str, token: &str) -> Result<Self, ConfigError> {
        let base = Url::parse(base_url).map_err(|_| ConfigError::BadUrl)?;
        if base.cannot_be_a_base() {
            return Err(ConfigError::BadUrl);
        }
        if base.scheme() != "http" {
            return Err(ConfigError::NotHttp);
        }
        if !base.username().is_empty() || base.password().is_some() {
            return Err(ConfigError::HasCredentials);
        }
        if base.query().is_some() || base.fragment().is_some() {
            return Err(ConfigError::HasQuery);
        }
        if token.is_empty() || !token.bytes().all(|byte| (0x21..=0x7e).contains(&byte)) {
            return Err(ConfigError::BadToken);
        }

        let mut authorization =
            HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| ConfigError::BadToken)?;
        // Marked sensitive, so it is redacted if a request is ever debug-printed.
        authorization.set_sensitive(true);

        let client = Client::builder()
            .user_agent(concat!("clearcut-docproc/", env!("CARGO_PKG_VERSION")))
            .redirect(Policy::none())
            .no_proxy()
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .map_err(|_| ConfigError::Client)?;
        Ok(HttpWorker {
            client,
            base,
            authorization,
            timeout: DEFAULT_TIMEOUT,
        })
    }

    /// Changes how long a call may take.
    pub fn with_timeout(mut self, timeout: Duration) -> Result<Self, ConfigError> {
        if timeout.is_zero() {
            return Err(ConfigError::BadTimeout);
        }
        self.timeout = timeout;
        Ok(self)
    }

    /// The address of one of the worker's routes, with no doubled or missing slash.
    fn url(&self, path: &str) -> Url {
        let mut url = self.base.clone();
        url.set_path(&format!("{}/{path}", self.base.path().trim_end_matches('/')));
        url
    }

    /// Sends one request and returns the good answer's headers and body, or the error the
    /// status stands for. `limit` is how much of a good answer may be read.
    async fn send(
        &self,
        url: Url,
        content_type: &'static str,
        body: Bytes,
        limit: usize,
    ) -> Result<(HeaderMap, Vec<u8>), ProcessError> {
        let response = self
            .client
            .post(url)
            .header(AUTHORIZATION, self.authorization.clone())
            .header(CONTENT_TYPE, content_type)
            .body(body)
            .timeout(self.timeout)
            .send()
            .await
            .map_err(classify)?;
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        // An error answer is short whatever the call; a good one has its own limit.
        let read_up_to = if status == 200 { limit } else { MAX_RESPONSE_BYTES };
        let (body, complete) = read_limited(response, read_up_to).await?;
        if status != 200 {
            return Err(map_status(status, &body));
        }
        if !complete {
            return Err(ProcessError::BadResponse("the answer was too large"));
        }
        Ok((headers, body))
    }
}

impl DocumentProcessor for HttpWorker {
    async fn inspect(&self, document: Bytes) -> Result<Inspection, ProcessError> {
        let (_, body) = self
            .send(
                self.url("v1/inspect"),
                "application/octet-stream",
                document,
                MAX_RESPONSE_BYTES,
            )
            .await?;
        parse_inspection(&body)
    }

    async fn page_texts(
        &self,
        document: Bytes,
        range: PageRange,
        words: bool,
    ) -> Result<Vec<PageContent>, ProcessError> {
        let mut url = self.url("v1/text");
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("from", &range.first().get().to_string());
            if let Some(last) = range.last() {
                query.append_pair("to", &last.get().to_string());
            }
            query.append_pair("words", if words { "true" } else { "false" });
        }
        let (_, body) = self
            .send(url, "application/octet-stream", document, MAX_TEXT_RESPONSE_BYTES)
            .await?;
        wire::parse_pages(&body, &wire::Expect::Text { range, words })
    }

    async fn ocr_page(
        &self,
        document: Bytes,
        page: NonZeroU32,
        agreement: bool,
    ) -> Result<PageContent, ProcessError> {
        let mut url = self.url("v1/ocr");
        url.query_pairs_mut()
            .append_pair("page", &page.get().to_string())
            .append_pair("agreement", if agreement { "true" } else { "false" });
        let (_, body) = self
            .send(url, "application/octet-stream", document, MAX_TEXT_RESPONSE_BYTES)
            .await?;
        wire::parse_pages(&body, &wire::Expect::Ocr { page, agreement })?
            .pop()
            .ok_or(ProcessError::BadResponse("not the pages that were asked for"))
    }

    async fn render_page(
        &self,
        document: Bytes,
        page: NonZeroU32,
        options: RenderOptions,
    ) -> Result<RenderedPage, ProcessError> {
        let mut url = self.url("v1/render");
        url.query_pairs_mut()
            .append_pair("page", &page.get().to_string())
            .append_pair("dpi", &options.dpi().to_string())
            .append_pair("format", options.format().as_str())
            .append_pair("max_edge", &options.max_edge().to_string());
        let (headers, body) = self
            .send(
                url,
                "application/octet-stream",
                document,
                MAX_IMAGE_RESPONSE_BYTES,
            )
            .await?;
        picture(&headers, body, options.format(), options.max_edge())
    }

    async fn mask_image(
        &self,
        image: Bytes,
        boxes: Vec<MaskBox>,
        format: ImageFormat,
        max_edge: u16,
    ) -> Result<RenderedPage, ProcessError> {
        if boxes.len() > MAX_BOXES {
            // The worker would refuse it as an invalid request; do not send it.
            return Err(ProcessError::Unexpected { status: 400 });
        }
        let mut url = self.url("v1/mask");
        url.query_pairs_mut()
            .append_pair("format", format.as_str())
            .append_pair("max_edge", &max_edge.to_string());
        let request = json!({
            "image": base64::engine::general_purpose::STANDARD.encode(&image),
            "boxes": boxes
                .iter()
                .map(|b| json!({"x": b.x(), "y": b.y(), "w": b.w(), "h": b.h()}))
                .collect::<Vec<Value>>(),
        });
        let body = serde_json::to_vec(&request)
            .map_err(|_| ProcessError::BadResponse("the request could not be built"))?;
        let (headers, body) = self
            .send(
                url,
                "application/json",
                Bytes::from(body),
                MAX_IMAGE_RESPONSE_BYTES,
            )
            .await?;
        picture(&headers, body, format, max_edge)
    }
}

/// A failure to send or to read: said in terms of what the caller should do.
fn classify(error: reqwest::Error) -> ProcessError {
    if error.is_timeout() {
        ProcessError::Timeout
    } else if error.is_connect() {
        ProcessError::Unavailable
    } else {
        ProcessError::Interrupted
    }
}

/// Reads at most `limit` bytes of the body. The flag says whether that was all of it.
async fn read_limited(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<(Vec<u8>, bool), ProcessError> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(classify)? {
        let room = limit - body.len();
        if chunk.len() > room {
            body.extend_from_slice(&chunk[..room]);
            return Ok((body, false));
        }
        body.extend_from_slice(&chunk);
    }
    Ok((body, true))
}

/// The worker's success answer to an inspection. Fields we do not know yet are ignored.
#[derive(Deserialize)]
struct Wire {
    media_type: String,
    pages: u64,
    pages_with_text: u64,
    has_text_layer: bool,
}

fn parse_inspection(body: &[u8]) -> Result<Inspection, ProcessError> {
    let wire: Wire =
        serde_json::from_slice(body).map_err(|_| ProcessError::BadResponse("not the expected shape"))?;
    let kind =
        DocumentKind::parse(&wire.media_type).ok_or(ProcessError::BadResponse("an unknown document type"))?;
    let count =
        |value: u64| u32::try_from(value).map_err(|_| ProcessError::BadResponse("a count out of range"));
    let inspection = Inspection::new(kind, count(wire.pages)?, count(wire.pages_with_text)?)
        .map_err(|_| ProcessError::BadResponse("inconsistent page counts"))?;
    if inspection.has_text_layer() != wire.has_text_layer {
        return Err(ProcessError::BadResponse("inconsistent text layer"));
    }
    Ok(inspection)
}

/// A picture answer: its kind from `Content-Type`, its size from the two size headers, and
/// all of it held to what was asked. A PNG asked for may come back as a JPEG (a big one is
/// sent that way); a JPEG asked for may not come back as a PNG.
fn picture(
    headers: &HeaderMap,
    body: Vec<u8>,
    asked: ImageFormat,
    max_edge: u16,
) -> Result<RenderedPage, ProcessError> {
    const BAD: ProcessError = ProcessError::BadResponse("the picture was not as promised");
    let text = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
    let media = match text("content-type") {
        Some("image/png") => ImageType::Png,
        Some("image/jpeg") => ImageType::Jpeg,
        _ => return Err(BAD),
    };
    if asked == ImageFormat::Jpeg && media == ImageType::Png {
        return Err(BAD);
    }
    let dimension = |name: &str| {
        let value = text(name)?;
        let plain = !value.is_empty() && value.len() <= 9 && value.bytes().all(|b| b.is_ascii_digit());
        plain.then(|| value.parse::<u32>().ok()).flatten()
    };
    let (Some(width), Some(height)) = (dimension("x-image-width"), dimension("x-image-height")) else {
        return Err(BAD);
    };
    if width > u32::from(max_edge) || height > u32::from(max_edge) {
        return Err(BAD);
    }
    RenderedPage::new(media, Bytes::from(body), width, height).map_err(|_| BAD)
}

/// An error answer, understood by its status first and its message second. The
/// message is only ever compared with known texts, never repeated.
fn map_status(status: u16, body: &[u8]) -> ProcessError {
    let json: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let message = json.get("error").and_then(Value::as_str).unwrap_or("");
    let limit = |name: &str| json.get(name).and_then(Value::as_u64);
    match status {
        400 if message == "the file is empty" => ProcessError::Document(DocumentProblem::Empty),
        401 | 403 => ProcessError::Auth,
        413 => ProcessError::Document(DocumentProblem::TooLarge {
            max_bytes: limit("max_bytes").unwrap_or(0),
        }),
        415 => ProcessError::Document(DocumentProblem::UnsupportedType),
        422 => ProcessError::Document(match message {
            "the document is encrypted" => DocumentProblem::Encrypted,
            "too many pages" => match limit("max_pages").and_then(|n| u32::try_from(n).ok()) {
                Some(max_pages) if max_pages > 0 => DocumentProblem::TooManyPages { max_pages },
                _ => DocumentProblem::Unreadable,
            },
            "the page range is outside the document" => {
                match limit("pages").and_then(|n| u32::try_from(n).ok()) {
                    Some(pages) => DocumentProblem::PageOutOfRange { pages },
                    None => DocumentProblem::Unreadable,
                }
            }
            "a page holds more text than the worker will report" => DocumentProblem::PageTooLarge,
            "the page is too large to render" => DocumentProblem::PageTooBig,
            "the image is too large" => DocumentProblem::ImageTooLarge,
            _ => DocumentProblem::Unreadable,
        }),
        503 if message == "OCR is not available" => ProcessError::NotAvailable,
        500..=599 => ProcessError::Server { status },
        _ => ProcessError::Unexpected { status },
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod page_tests;
