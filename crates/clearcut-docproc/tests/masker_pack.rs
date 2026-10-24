// Written by Kone Tshivhinda & Claude
//! The worker's masker and the context-pack builder, together.
//!
//! The builder asks its [`ImageMasker`] for the masked picture while it works out what to
//! send; [`WorkerMasker`] has already had the worker paint it. This is the path a page
//! takes, with a scripted worker standing in for the real one.

use bytes::Bytes;
use clearcut_core::redact::redact;
use clearcut_docproc::{
    FakeWorker, ImageFormat, PageContent, PageParts, PageSource, PageWord, ProcessError, RenderedPage,
    WordBox, WorkerMasker,
};
use clearcut_pack::{
    build_pack_with_nonce, DocumentContext, ImageState, NoMasker, PackError, PackSettings, PageImage,
    PageInput, PageText, RedactionPolicy, TextSource, UnsafeReason,
};
use clearcut_provider::{Block, CacheTtl, CallContext, Effort, ImageType, Purpose};
use std::num::NonZeroU32;
use uuid::Uuid;

const NONCE: &str = "0123456789abcdef0123456789abcdef";
const TEXT: &str = "Account No: 1234567890 and mail jane@example.co.za please";

fn n(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).unwrap()
}

fn settings(redaction: RedactionPolicy) -> PackSettings {
    PackSettings {
        model: "claude-haiku-5-5".into(),
        call: CallContext {
            tenant: Uuid::from_u128(1),
            job: Some(Uuid::from_u128(7)),
            document: Some(Uuid::from_u128(8)),
            page: Some(1),
            purpose: Purpose::Extraction,
        },
        effort: Effort::Medium,
        max_output_tokens: 4000,
        cache_ttl: CacheTtl::FiveMinutes,
        output_schema: serde_json::json!({
            "type": "object",
            "properties": { "invoice_number": { "type": "string" } },
            "required": ["invoice_number"]
        }),
        tenant_rules: None,
        redaction,
    }
}

fn image() -> PageImage {
    PageImage {
        media: ImageType::Png,
        bytes: b"\x89PNG\r\n\x1a\noriginal".to_vec(),
    }
}

fn input() -> PageInput {
    PageInput {
        page: n(1),
        of: n(1),
        image: Some(image()),
        text: Some(PageText {
            source: TextSource::TextLayer,
            content: TEXT.to_string(),
        }),
    }
}

/// The words of `TEXT`, found the way a worker would: runs between spaces, each placed
/// a little further along the line.
fn page() -> PageContent {
    let mut words = Vec::new();
    let mut offset = 0;
    for (index, token) in TEXT.split(' ').enumerate() {
        words.push(PageWord {
            start: offset,
            end: offset + token.len(),
            bounds: WordBox {
                x: u16::try_from(index * 90).unwrap(),
                y: 100,
                w: 80,
                h: 30,
            },
            confidence: None,
        });
        offset += token.len() + 1;
    }
    PageContent::from_parts(PageParts {
        page: n(1),
        source: PageSource::TextLayer,
        text: TEXT.to_string(),
        width_pt: 595.0,
        height_pt: 842.0,
        fonts: Vec::new(),
        words: Some(words),
        agreement: None,
    })
    .unwrap()
}

fn painted() -> RenderedPage {
    RenderedPage::new(
        ImageType::Png,
        Bytes::from_static(b"\x89PNG\r\n\x1a\npainted"),
        400,
        300,
    )
    .unwrap()
}

#[tokio::test]
async fn the_picture_in_the_pack_is_the_one_the_worker_painted() {
    let spans = redact(TEXT).spans;
    assert_eq!(spans.len(), 2, "an account number and an e-mail address");
    let worker = FakeWorker::scripted(vec![]).with_masks(vec![Ok(painted())]);
    let page_input = input();
    let (image, text) = (
        page_input.image.as_ref().unwrap(),
        page_input.text.as_ref().unwrap(),
    );
    let masker = WorkerMasker::prepare(&worker, image, &page(), text, &spans)
        .await
        .unwrap();

    let pack = build_pack_with_nonce(
        &settings(RedactionPolicy::BestEffort),
        &page_input,
        &DocumentContext::default(),
        None,
        &masker,
        NONCE,
    )
    .unwrap();
    assert_eq!(pack.manifest.redaction.image, ImageState::Masked);
    let sent: Vec<&Vec<u8>> = pack
        .request
        .content()
        .iter()
        .filter_map(|block| match block {
            Block::Image { bytes, .. } => Some(bytes),
            _ => None,
        })
        .collect();
    assert_eq!(sent, [&b"\x89PNG\r\n\x1a\npainted".to_vec()]);

    // The worker was asked to paint exactly the words that hold the two values, once.
    let calls = worker.mask_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].format, ImageFormat::Png);
    assert_eq!(calls[0].boxes.len(), 2);
}

#[tokio::test]
async fn a_masker_prepared_for_other_spans_is_never_used_and_the_page_is_unmasked_not_leaked() {
    let worker = FakeWorker::scripted(vec![]).with_masks(vec![Ok(painted())]);
    let page_input = input();
    let (image, text) = (
        page_input.image.as_ref().unwrap(),
        page_input.text.as_ref().unwrap(),
    );
    // Prepared for the account number only; the builder will find the e-mail address too.
    let only_the_number = vec![redact(TEXT).spans[0]];
    let masker = WorkerMasker::prepare(&worker, image, &page(), text, &only_the_number)
        .await
        .unwrap();

    let best_effort = build_pack_with_nonce(
        &settings(RedactionPolicy::BestEffort),
        &page_input,
        &DocumentContext::default(),
        None,
        &masker,
        NONCE,
    )
    .unwrap();
    assert_eq!(best_effort.manifest.redaction.image, ImageState::Unmasked);

    let strict = build_pack_with_nonce(
        &settings(RedactionPolicy::Strict),
        &page_input,
        &DocumentContext::default(),
        None,
        &masker,
        NONCE,
    );
    assert!(matches!(
        strict,
        Err(PackError::CannotBeSentSafely(UnsafeReason::UnmaskableMatches))
    ));
}

#[tokio::test]
async fn when_the_worker_cannot_paint_strict_sends_nothing() {
    let worker = FakeWorker::scripted(vec![]).with_masks(vec![Err(ProcessError::Timeout)]);
    let page_input = input();
    let (image, text) = (
        page_input.image.as_ref().unwrap(),
        page_input.text.as_ref().unwrap(),
    );
    let spans = redact(TEXT).spans;
    assert!(WorkerMasker::prepare(&worker, image, &page(), text, &spans)
        .await
        .is_err());

    // With no masker at all the builder behaves the same way.
    let strict = build_pack_with_nonce(
        &settings(RedactionPolicy::Strict),
        &page_input,
        &DocumentContext::default(),
        None,
        &NoMasker,
        NONCE,
    );
    assert!(matches!(strict, Err(PackError::CannotBeSentSafely(_))));
}

#[tokio::test]
async fn the_spans_the_caller_computes_are_the_ones_the_builder_computes() {
    // The masker is only handed over when the spans agree exactly, so this is what makes
    // it work at all: the redaction is a pure function of the text.
    assert_eq!(redact(TEXT).spans, redact(TEXT).spans);
    let worker = FakeWorker::scripted(vec![]).with_masks(vec![Ok(painted())]);
    let page_input = input();
    let (image, text) = (
        page_input.image.as_ref().unwrap(),
        page_input.text.as_ref().unwrap(),
    );
    let masker = WorkerMasker::prepare(&worker, image, &page(), text, &redact(TEXT).spans)
        .await
        .unwrap();
    assert!(clearcut_pack::ImageMasker::mask(&masker, image, text, &redact(TEXT).spans).is_ok());
}
