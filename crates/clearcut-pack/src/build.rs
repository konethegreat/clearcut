// Written by Kone Tshivhinda & Claude
//! Assembling a pack (ADR-0014 §1 to §4).

use crate::input::{DocumentContext, FailureReport, PackSettings, PageImage, PageInput, RedactionPolicy};
use crate::manifest::{ImageState, PackManifest, RedactionReport, Section};
use crate::mask::ImageMasker;
use crate::prompt::{prompt_version, static_prefix};
use clearcut_core::redact;
use clearcut_core::untrusted::{self, SuspicionKind};
use clearcut_core::Currency;
use clearcut_provider::{Block, ModelRequest, RequestError, Thinking};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// More page text than this is refused, never cut: a silently shortened page is a wrong reading.
pub const MAX_PAGE_TEXT_BYTES: usize = 200_000;
/// Values from earlier pages are shown at most this long (in characters).
const MAX_CONTEXT_VALUE_CHARS: usize = 200;
/// The previous attempt's checks are shown at most this many.
const MAX_FAILURE_LINES: usize = 50;
const NONCE_LENGTH: usize = 32;

/// A built request, and the record of what is in it.
#[derive(Debug)]
pub struct Pack {
    pub request: ModelRequest,
    pub manifest: PackManifest,
}

/// Why a page could not be turned into a request. None of these carry text.
#[derive(Debug, thiserror::Error)]
pub enum PackError {
    #[error("the page has neither an image nor text")]
    NothingToRead,
    #[error("the page's text is larger than the {MAX_PAGE_TEXT_BYTES} bytes that are sent")]
    PageTextTooLarge,
    #[error("the nonce must be 32 lower-case hexadecimal digits")]
    BadNonce,
    #[error("the page contains the nonce chosen for this request")]
    NonceInPage,
    #[error("the page cannot be sent safely under the strict redaction policy: {0}")]
    CannotBeSentSafely(UnsafeReason),
    #[error("the request is not valid: {0}")]
    Request(#[from] RequestError),
}

/// Why the strict policy refused a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum UnsafeReason {
    #[error("personal data was found in the text and the image could not be masked")]
    UnmaskableMatches,
    #[error("the image has no text to check it against")]
    NothingToCheck,
}

/// A fresh random id for the data tags of one request: 32 lower-case hex digits
/// from the operating system's random source (a UUID version 4).
pub fn fresh_nonce() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// Builds the pack for one page, drawing its own nonce.
pub fn build_pack(
    settings: &PackSettings,
    page: &PageInput,
    context: &DocumentContext,
    failure: Option<&FailureReport>,
    masker: &dyn ImageMasker,
) -> Result<Pack, PackError> {
    // The page cannot know the nonce, so a clash is a freak chance; draw again.
    for _ in 0..4 {
        match build_pack_with_nonce(settings, page, context, failure, masker, &fresh_nonce()) {
            Err(PackError::NonceInPage) => continue,
            other => return other,
        }
    }
    Err(PackError::NonceInPage)
}

/// Builds the pack for one page with a given nonce (tests use fixed ones).
pub fn build_pack_with_nonce(
    settings: &PackSettings,
    page: &PageInput,
    context: &DocumentContext,
    failure: Option<&FailureReport>,
    masker: &dyn ImageMasker,
    nonce: &str,
) -> Result<Pack, PackError> {
    if nonce.len() != NONCE_LENGTH
        || !nonce
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(PackError::BadNonce);
    }
    if page.image.is_none() && page.text.is_none() {
        return Err(PackError::NothingToRead);
    }
    if page
        .text
        .as_ref()
        .is_some_and(|t| t.content.len() > MAX_PAGE_TEXT_BYTES)
    {
        return Err(PackError::PageTextTooLarge);
    }
    let untrusted_strings: Vec<&str> = page
        .text
        .iter()
        .map(|t| t.content.as_str())
        .chain(context.invoice_number.as_deref())
        .chain(context.supplier_name.as_deref())
        .collect();
    if untrusted_strings.iter().any(|s| s.contains(nonce)) {
        return Err(PackError::NonceInPage);
    }

    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut count = |spans: &[redact::Span]| {
        for span in spans {
            *counts.entry(span.kind.as_str()).or_insert(0) += 1;
        }
    };

    // ---- what the model is told about the page's text, redacted
    let page_text = page.text.as_ref().map(|text| {
        let redacted = redact::redact(&text.content);
        count(&redacted.spans);
        (text, redacted)
    });

    // ---- the image
    let (image_state, image) = match (&page.image, &page_text) {
        (None, _) => (ImageState::NotApplicable, None),
        (Some(image), Some((_, redacted))) if redacted.spans.is_empty() => {
            (ImageState::NotNeeded, Some(image.clone()))
        }
        (Some(image), Some((text, redacted))) => match masker.mask(image, text, &redacted.spans) {
            Ok(masked) => (ImageState::Masked, Some(masked)),
            Err(_) => (ImageState::Unmasked, Some(image.clone())),
        },
        (Some(image), None) => (ImageState::Unchecked, Some(image.clone())),
    };
    if settings.redaction == RedactionPolicy::Strict {
        match image_state {
            ImageState::Unmasked => {
                return Err(PackError::CannotBeSentSafely(UnsafeReason::UnmaskableMatches))
            }
            ImageState::Unchecked => return Err(PackError::CannotBeSentSafely(UnsafeReason::NothingToCheck)),
            _ => {}
        }
    }

    // ---- injection flags (on the original text, before redaction)
    let mut suspicions: Vec<SuspicionKind> = untrusted_strings
        .iter()
        .flat_map(|s| untrusted::suspicious_kinds(s))
        .collect();
    suspicions.sort();
    suspicions.dedup();

    // ---- the cached system blocks
    let rules = static_prefix();
    let tenant_rules = settings.tenant_rules.as_deref().and_then(|text| {
        let redacted = redact::redact(&text.replace("\r\n", "\n"));
        count(&redacted.spans);
        let trimmed = redacted.text.trim().to_string();
        (!trimmed.is_empty())
            .then(|| format!("Conventions confirmed by the reviewers of this account:\n{trimmed}"))
    });

    // ---- the per-page blocks
    let context_block = document_context_block(page, context, nonce, &mut count);
    let text_block = match &page_text {
        Some((text, redacted)) => format!(
            "<page_text id=\"{nonce}\" source=\"{}\">\n{}\n</page_text id=\"{nonce}\">",
            text.source.as_str(),
            redacted.text
        ),
        None => format!(
            "<page_text id=\"{nonce}\" source=\"none\">\n(no text is available for this page; read the image)\n</page_text id=\"{nonce}\">"
        ),
    };
    let failure_block = failure.map(failure_block);
    let instruction = format!(
        "Read page {} of {} now and return only the JSON.",
        page.page, page.of
    );

    // ---- the request
    let mut builder = ModelRequest::builder(&settings.model, settings.call.clone())
        .effort(settings.effort)
        .thinking(Thinking::Adaptive)
        .cache_ttl(settings.cache_ttl)
        .output_schema(settings.output_schema.clone())
        .max_output_tokens(settings.max_output_tokens)
        .cached_system(rules.clone());
    if let Some(tenant) = &tenant_rules {
        builder = builder.cached_system(tenant.clone());
    }
    let mut sections = vec![Section {
        kind: "rules",
        bytes: rules.len(),
        cached: true,
    }];
    if let Some(tenant) = &tenant_rules {
        sections.push(Section {
            kind: "tenant_rules",
            bytes: tenant.len(),
            cached: true,
        });
    }
    if let Some(PageImage { media, bytes }) = &image {
        builder = builder.content(Block::image(*media, bytes.clone()));
        sections.push(Section {
            kind: "page_image",
            bytes: bytes.len(),
            cached: false,
        });
    }
    let mut add_text = |kind: &'static str, text: &str, builder: clearcut_provider::RequestBuilder| {
        sections.push(Section {
            kind,
            bytes: text.len(),
            cached: false,
        });
        builder.content(Block::text(text))
    };
    builder = add_text("document_context", &context_block, builder);
    builder = add_text("page_text", &text_block, builder);
    if let Some(block) = &failure_block {
        builder = add_text("previous_attempt", block, builder);
    }
    builder = add_text("instruction", &instruction, builder);
    let request = builder.build()?;

    // ---- the manifest
    let mut prefix_hash = Sha256::new();
    let mut prefix_bytes = 0;
    for block in request.system() {
        prefix_hash.update((block.text().len() as u64).to_le_bytes());
        prefix_hash.update(block.text().as_bytes());
        prefix_bytes += block.text().len();
    }
    let schema_json = serde_json::to_string(&settings.output_schema).unwrap_or_default();
    let manifest = PackManifest {
        prompt_version: prompt_version(),
        prefix_sha256: hex(&prefix_hash.finalize()),
        prefix_bytes,
        schema_sha256: hex(&Sha256::digest(schema_json.as_bytes())),
        model: settings.model.clone(),
        effort: settings.effort.as_str(),
        cache_ttl: settings.cache_ttl.as_str(),
        page: page.page.get(),
        of: page.of.get(),
        text_source: page.text.as_ref().map_or("none", |t| t.source.as_str()),
        sections,
        redaction: RedactionReport {
            policy: settings.redaction.as_str(),
            counts,
            image: image_state,
        },
        injection_suspected: suspicions.iter().map(|kind| kind.as_str()).collect(),
        escalated_from: failure.map(|f| f.previous_model.clone()),
    };
    Ok(Pack { request, manifest })
}

fn currency_code(currency: Currency) -> &'static str {
    match currency {
        Currency::Zar => "ZAR",
        Currency::Usd => "USD",
        Currency::Eur => "EUR",
        Currency::Gbp => "GBP",
    }
}

/// A value from an earlier page, made safe to put on one line: redacted, every
/// run of whitespace a single space, and cut at 200 characters.
fn context_value(raw: &str, count: &mut impl FnMut(&[redact::Span])) -> String {
    let redacted = redact::redact(raw);
    count(&redacted.spans);
    let one_line: Vec<&str> = redacted.text.split_whitespace().collect();
    one_line.join(" ").chars().take(MAX_CONTEXT_VALUE_CHARS).collect()
}

fn document_context_block(
    page: &PageInput,
    context: &DocumentContext,
    nonce: &str,
    count: &mut impl FnMut(&[redact::Span]),
) -> String {
    let mut lines = vec![format!("page: {} of {}", page.page, page.of)];
    if let Some(currency) = context.currency {
        lines.push(format!(
            "currency established on earlier pages: {}",
            currency_code(currency)
        ));
    }
    if let Some(number) = &context.invoice_number {
        lines.push(format!(
            "invoice number read on earlier pages: {}",
            context_value(number, count)
        ));
    }
    if let Some(name) = &context.supplier_name {
        lines.push(format!(
            "supplier name read on earlier pages: {}",
            context_value(name, count)
        ));
    }
    if context.totals_seen {
        lines.push("totals already read on earlier pages: yes".to_string());
    }
    if context.lines_read > 0 {
        lines.push(format!(
            "line items read on earlier pages: {}",
            context.lines_read
        ));
    }
    format!(
        "<document_context id=\"{nonce}\">\n{}\n</document_context id=\"{nonce}\">",
        lines.join("\n")
    )
}

/// The previous attempt's failed checks, one JSON object per line, in ClearCut's
/// own fixed vocabulary.
fn failure_block(failure: &FailureReport) -> String {
    let model: String = failure
        .previous_model
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-'))
        .collect();
    let json = |value: &str| serde_json::to_string(value).unwrap_or_default();
    let problems = failure.problems.iter().map(|p| {
        let kind = serde_json::to_string(&p.kind).unwrap_or_default();
        format!(
            "{{\"type\":\"problem\",\"path\":{},\"kind\":{kind}}}",
            json(&p.path)
        )
    });
    let findings = failure
        .findings
        .iter()
        .filter_map(|f| serde_json::to_string(f).ok());
    let lines: Vec<String> = problems.chain(findings).take(MAX_FAILURE_LINES).collect();
    format!(
        "<previous_attempt model=\"{model}\">\nThese checks failed on the previous reading of this page. They were produced by ClearCut, not by the document.\n{}\n</previous_attempt>",
        lines.join("\n")
    )
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
