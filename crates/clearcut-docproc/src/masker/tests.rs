// Written by Kone Tshivhinda & Claude
use super::*;
use crate::{FakeWorker, PageParts, PageSource, ProcessError, RenderedPage, WordBox};
use clearcut_core::redact::RedactionKind;
use clearcut_pack::TextSource;
use clearcut_provider::ImageType;
use std::num::NonZeroU32;

fn word(start: usize, end: usize, x: u16, y: u16, w: u16, h: u16) -> PageWord {
    PageWord {
        start,
        end,
        bounds: WordBox { x, y, w, h },
        confidence: None,
    }
}

fn span(start: usize, end: usize) -> Span {
    Span {
        kind: RedactionKind::BankAccount,
        start,
        end,
    }
}

fn mask_box(x: u16, y: u16, w: u16, h: u16) -> MaskBox {
    MaskBox::new(x, y, w, h).unwrap()
}

/// "Account No: 1234567890": three words.
fn words() -> Vec<PageWord> {
    vec![
        word(0, 7, 100, 200, 300, 40),
        word(8, 11, 420, 200, 100, 40),
        word(12, 22, 540, 200, 250, 40),
    ]
}

// ---------- which words, and where ----------

#[test]
fn the_margin_is_three_thousandths_on_every_side() {
    assert_eq!(MASK_MARGIN_PER_MILLE, 3);
}

#[test]
fn a_span_inside_a_word_boxes_the_whole_word_with_a_margin() {
    let boxes = boxes_for_spans(&words(), &[span(14, 17)]).unwrap();
    assert_eq!(boxes, [mask_box(537, 197, 256, 46)]);
}

#[test]
fn a_span_across_words_boxes_each_word_it_touches() {
    let boxes = boxes_for_spans(&words(), &[span(9, 15)]).unwrap();
    assert_eq!(boxes, [mask_box(417, 197, 106, 46), mask_box(537, 197, 256, 46)]);
}

#[test]
fn spans_are_half_open_so_a_span_that_only_touches_a_word_leaves_it_alone() {
    // The first word is bytes 0 to 7 and the second 8 to 11.
    assert_eq!(boxes_for_spans(&words(), &[span(0, 7)]).unwrap().len(), 1);
    assert_eq!(
        boxes_for_spans(&words(), &[span(6, 8)]).unwrap().len(),
        1,
        "touches word one only"
    );
    assert_eq!(
        boxes_for_spans(&words(), &[span(7, 9)]).unwrap().len(),
        1,
        "touches word two only"
    );
    assert_eq!(boxes_for_spans(&words(), &[span(6, 9)]).unwrap().len(), 2);
}

#[test]
fn several_spans_on_one_word_give_one_box() {
    let boxes = boxes_for_spans(&words(), &[span(12, 14), span(15, 20), span(21, 22)]).unwrap();
    assert_eq!(boxes.len(), 1);
}

#[test]
fn boxes_come_in_the_order_of_the_words() {
    let boxes = boxes_for_spans(&words(), &[span(14, 15), span(1, 2)]).unwrap();
    assert_eq!(boxes, [mask_box(97, 197, 306, 46), mask_box(537, 197, 256, 46)]);
}

#[test]
fn the_margin_stops_at_the_edge_of_the_picture() {
    let at_corners = [word(0, 3, 0, 0, 10, 10), word(4, 7, 990, 990, 10, 10)];
    let boxes = boxes_for_spans(&at_corners, &[span(0, 7)]).unwrap();
    assert_eq!(boxes, [mask_box(0, 0, 13, 13), mask_box(987, 987, 13, 13)]);
    let whole = [word(0, 3, 0, 0, 1000, 1000)];
    assert_eq!(
        boxes_for_spans(&whole, &[span(0, 3)]).unwrap(),
        [mask_box(0, 0, 1000, 1000)]
    );
}

#[test]
fn nothing_to_hide_is_nothing_to_paint_even_with_no_words() {
    assert_eq!(boxes_for_spans(&words(), &[]).unwrap(), Vec::new());
    assert_eq!(boxes_for_spans(&[], &[]).unwrap(), Vec::new());
}

#[test]
fn a_span_that_covers_no_word_cannot_be_painted_and_says_so() {
    // Byte 11 is the space between the second and third words.
    assert_eq!(
        boxes_for_spans(&words(), &[span(11, 12)]),
        Err(MaskPlanError::UnlocatedSpan)
    );
    assert_eq!(
        boxes_for_spans(&words(), &[span(30, 40)]),
        Err(MaskPlanError::UnlocatedSpan)
    );
    assert_eq!(
        boxes_for_spans(&[], &[span(0, 3)]),
        Err(MaskPlanError::UnlocatedSpan)
    );
    // One located span does not excuse another.
    assert_eq!(
        boxes_for_spans(&words(), &[span(0, 3), span(30, 40)]),
        Err(MaskPlanError::UnlocatedSpan)
    );
}

#[test]
fn a_span_with_no_length_is_not_a_span_to_trust() {
    assert_eq!(
        boxes_for_spans(&words(), &[span(3, 3)]),
        Err(MaskPlanError::UnlocatedSpan)
    );
    assert_eq!(
        boxes_for_spans(&words(), &[span(5, 2)]),
        Err(MaskPlanError::UnlocatedSpan)
    );
}

#[test]
fn at_most_five_hundred_boxes() {
    let many = |count: usize| -> (Vec<PageWord>, Vec<Span>) {
        let words: Vec<PageWord> = (0..count)
            .map(|i| word(i * 2, i * 2 + 1, 10, 10, 20, 20))
            .collect();
        let spans: Vec<Span> = (0..count).map(|i| span(i * 2, i * 2 + 1)).collect();
        (words, spans)
    };
    let (words, spans) = many(500);
    assert_eq!(boxes_for_spans(&words, &spans).unwrap().len(), 500);
    let (words, spans) = many(501);
    assert_eq!(boxes_for_spans(&words, &spans), Err(MaskPlanError::TooManyBoxes));
}

// ---------- the masker that asks the worker ----------

const NUMBER_TEXT: &str = "Account No: 1234567890";

fn n(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).unwrap()
}

fn page_content(source: PageSource, text: &str, words: Option<Vec<PageWord>>) -> PageContent {
    let words = match source {
        PageSource::Ocr => words.map(|list| {
            list.into_iter()
                .map(|w| PageWord {
                    confidence: Some(900),
                    ..w
                })
                .collect()
        }),
        _ => words,
    };
    PageContent::from_parts(PageParts {
        page: n(1),
        source,
        text: text.to_string(),
        width_pt: 595.0,
        height_pt: 842.0,
        fonts: Vec::new(),
        words,
        agreement: None,
    })
    .unwrap()
}

fn text_layer() -> PageContent {
    page_content(PageSource::TextLayer, NUMBER_TEXT, Some(words()))
}

fn image() -> PageImage {
    PageImage {
        media: ImageType::Png,
        bytes: vec![1, 2, 3, 4],
    }
}

fn page_text() -> PageText {
    PageText {
        source: TextSource::TextLayer,
        content: NUMBER_TEXT.to_string(),
    }
}

fn painted(media: ImageType) -> RenderedPage {
    let bytes: &'static [u8] = match media {
        ImageType::Png => b"\x89PNG\r\n\x1a\nMASKED",
        ImageType::Jpeg => b"\xff\xd8\xff\xe0MASKED",
    };
    RenderedPage::new(media, Bytes::from_static(bytes), 400, 300).unwrap()
}

fn number_span() -> Vec<Span> {
    vec![span(12, 22)]
}

#[tokio::test]
async fn the_worker_is_asked_once_for_the_words_that_hold_personal_data() {
    let fake = FakeWorker::scripted(vec![]).with_masks(vec![Ok(painted(ImageType::Png))]);
    let masker = WorkerMasker::prepare(&fake, &image(), &text_layer(), &page_text(), &number_span())
        .await
        .unwrap();
    let calls = fake.mask_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].image, Bytes::from(vec![1, 2, 3, 4]));
    assert_eq!(calls[0].boxes, [mask_box(537, 197, 256, 46)]);
    assert_eq!(calls[0].format, ImageFormat::Png);
    assert_eq!(calls[0].max_edge, crate::MAX_EDGE);
    assert_eq!(fake.call_count(), 1);

    let masked = masker.mask(&image(), &page_text(), &number_span()).unwrap();
    assert_eq!(masked.media, ImageType::Png);
    assert_eq!(masked.bytes, b"\x89PNG\r\n\x1a\nMASKED");
}

#[tokio::test]
async fn a_jpeg_is_painted_as_a_jpeg() {
    let fake = FakeWorker::scripted(vec![]).with_masks(vec![Ok(painted(ImageType::Jpeg))]);
    let jpeg = PageImage {
        media: ImageType::Jpeg,
        bytes: vec![9, 9],
    };
    let masker = WorkerMasker::prepare(&fake, &jpeg, &text_layer(), &page_text(), &number_span())
        .await
        .unwrap();
    assert_eq!(fake.mask_calls()[0].format, ImageFormat::Jpeg);
    let masked = masker.mask(&jpeg, &page_text(), &number_span()).unwrap();
    assert_eq!(masked.media, ImageType::Jpeg);
}

#[tokio::test]
async fn an_ocr_page_is_masked_through_its_own_words() {
    let fake = FakeWorker::scripted(vec![]).with_masks(vec![Ok(painted(ImageType::Png))]);
    let page = page_content(PageSource::Ocr, NUMBER_TEXT, Some(words()));
    let text = PageText {
        source: TextSource::Ocr,
        content: NUMBER_TEXT.to_string(),
    };
    let masker = WorkerMasker::prepare(&fake, &image(), &page, &text, &number_span())
        .await
        .unwrap();
    assert!(masker.mask(&image(), &text, &number_span()).is_ok());
}

#[tokio::test]
async fn with_nothing_to_hide_the_worker_is_not_asked_and_the_picture_comes_back_as_it_was() {
    let fake = FakeWorker::scripted(vec![]);
    let masker = WorkerMasker::prepare(&fake, &image(), &text_layer(), &page_text(), &[])
        .await
        .unwrap();
    assert_eq!(fake.call_count(), 0);
    assert_eq!(masker.mask(&image(), &page_text(), &[]).unwrap(), image());
}

#[tokio::test]
async fn it_hands_the_picture_over_only_for_the_very_page_it_was_prepared_for() {
    let fake = FakeWorker::scripted(vec![]).with_masks(vec![Ok(painted(ImageType::Png))]);
    let masker = WorkerMasker::prepare(&fake, &image(), &text_layer(), &page_text(), &number_span())
        .await
        .unwrap();
    let other_image = PageImage {
        bytes: vec![1, 2, 3, 5],
        ..image()
    };
    let other_media = PageImage {
        media: ImageType::Jpeg,
        ..image()
    };
    let other_text = PageText {
        content: "Account No: 1234567891".to_string(),
        ..page_text()
    };
    let other_source = PageText {
        source: TextSource::Ocr,
        ..page_text()
    };
    let wrong: Vec<(&str, PageImage, PageText, Vec<Span>)> = vec![
        ("another picture", other_image, page_text(), number_span()),
        ("another media type", other_media, page_text(), number_span()),
        ("another text", image(), other_text, number_span()),
        ("another source", image(), other_source, number_span()),
        ("no spans", image(), page_text(), vec![]),
        ("more spans", image(), page_text(), vec![span(12, 22), span(0, 7)]),
        ("another span", image(), page_text(), vec![span(12, 21)]),
        (
            "another kind",
            image(),
            page_text(),
            vec![Span {
                kind: RedactionKind::Email,
                ..span(12, 22)
            }],
        ),
    ];
    for (name, picture, text, spans) in wrong {
        assert_eq!(
            masker.mask(&picture, &text, &spans),
            Err(MaskError::Failed),
            "{name}"
        );
    }
    assert!(masker.mask(&image(), &page_text(), &number_span()).is_ok());
}

#[tokio::test]
async fn it_cannot_be_prepared_when_the_words_do_not_belong_to_the_text() {
    let cases: Vec<(&str, PageContent, PageText)> = vec![
        (
            "words never asked for",
            page_content(PageSource::TextLayer, NUMBER_TEXT, None),
            page_text(),
        ),
        (
            "another text",
            text_layer(),
            PageText {
                content: "Account No: 9999999999".to_string(),
                ..page_text()
            },
        ),
        (
            "a text layer read as OCR",
            text_layer(),
            PageText {
                source: TextSource::Ocr,
                ..page_text()
            },
        ),
        (
            "OCR read as a text layer",
            page_content(PageSource::Ocr, NUMBER_TEXT, Some(words())),
            page_text(),
        ),
    ];
    for (name, page, text) in cases {
        let fake = FakeWorker::scripted(vec![]).with_masks(vec![Ok(painted(ImageType::Png))]);
        let result = WorkerMasker::prepare(&fake, &image(), &page, &text, &number_span()).await;
        assert!(matches!(result, Err(MaskError::Failed)), "{name}");
        assert_eq!(fake.call_count(), 0, "{name}: the worker is not asked");
    }
}

#[tokio::test]
async fn it_cannot_be_prepared_when_a_span_cannot_be_located_or_there_are_too_many() {
    let fake = FakeWorker::scripted(vec![]).with_masks(vec![Ok(painted(ImageType::Png))]);
    let result = WorkerMasker::prepare(&fake, &image(), &text_layer(), &page_text(), &[span(11, 12)]).await;
    assert!(matches!(result, Err(MaskError::Failed)));
    assert_eq!(fake.call_count(), 0);

    let text: String = (0..501).map(|_| "a ").collect();
    let many_words: Vec<PageWord> = (0..501).map(|i| word(i * 2, i * 2 + 1, 10, 10, 20, 20)).collect();
    let spans: Vec<Span> = (0..501).map(|i| span(i * 2, i * 2 + 1)).collect();
    let page = page_content(PageSource::TextLayer, &text, Some(many_words));
    let long = PageText {
        source: TextSource::TextLayer,
        content: text,
    };
    let result = WorkerMasker::prepare(&fake, &image(), &page, &long, &spans).await;
    assert!(matches!(result, Err(MaskError::Failed)));
    assert_eq!(fake.call_count(), 0);
}

#[tokio::test]
async fn whatever_goes_wrong_at_the_worker_is_a_failure_to_mask_never_a_picture() {
    for error in [
        ProcessError::Unavailable,
        ProcessError::Timeout,
        ProcessError::NotAvailable,
        ProcessError::BadResponse("x"),
        ProcessError::Document(crate::DocumentProblem::Unreadable),
    ] {
        let fake = FakeWorker::scripted(vec![]).with_masks(vec![Err(error)]);
        let result =
            WorkerMasker::prepare(&fake, &image(), &text_layer(), &page_text(), &number_span()).await;
        assert!(matches!(result, Err(MaskError::Failed)), "{error:?}");
    }
}

#[tokio::test]
async fn the_kind_of_picture_that_comes_back_is_the_one_that_is_kept() {
    // A PNG that was too big may be sent back as a JPEG; the masker keeps what it was given.
    let fake = FakeWorker::scripted(vec![]).with_masks(vec![Ok(painted(ImageType::Jpeg))]);
    let masker = WorkerMasker::prepare(&fake, &image(), &text_layer(), &page_text(), &number_span())
        .await
        .unwrap();
    let masked = masker.mask(&image(), &page_text(), &number_span()).unwrap();
    assert_eq!(masked.media, ImageType::Jpeg);
    assert_eq!(masked.bytes, b"\xff\xd8\xff\xe0MASKED");
}
