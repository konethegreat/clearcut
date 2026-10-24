// Written by Kone Tshivhinda & Claude
//! The HTTP client's four newer calls against a stub worker: what is sent, what is
//! believed of the answer, and what each failure becomes.

use super::tests::{script, serve, stub, worker, Script, Stub, TOKEN};
use super::*;
use crate::{
    DocumentProblem, DocumentProcessor, ImageFormat, MaskBox, PageContent, PageRange, PageSource,
    ProcessError, RenderOptions, MAX_IMAGE_RESPONSE_BYTES, MAX_TEXT_RESPONSE_BYTES,
};
use axum::http::Method;
use bytes::Bytes;
use clearcut_provider::ImageType;
use serde_json::{json, Value};
use std::num::NonZeroU32;
use std::time::Duration;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n fake picture bytes";
const JPEG: &[u8] = b"\xff\xd8\xff\xe0 fake picture bytes";

fn n(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).unwrap()
}

fn query_of(request: &super::tests::Seen) -> Vec<(String, String)> {
    let url = Url::parse(&format!("http://h{}", request.path_and_query)).unwrap();
    let mut pairs: Vec<(String, String)> = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    pairs.sort();
    pairs
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = list.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    out.sort();
    out
}

// ================================================================ text ====

fn word(start: u32, end: u32) -> Value {
    json!({"start": start, "end": end, "x": 10, "y": 20, "w": 100, "h": 30})
}

fn text_page(number: u32) -> Value {
    json!({
        "page": number,
        "source": "text_layer",
        "text": "Total 100",
        "width_pt": 595.0,
        "height_pt": 842.0,
        "fonts": [{"kind": "type1", "has_to_unicode": false, "encoding": "winansi", "embedded": false}],
        "words": [word(0, 5), word(6, 9)],
    })
}

fn text_body(pages: &[u32]) -> Vec<u8> {
    let list: Vec<Value> = pages.iter().map(|p| text_page(*p)).collect();
    serde_json::to_vec(&json!({"pages": list})).unwrap()
}

async fn texts(stub: &Stub, range: PageRange, words: bool) -> Result<Vec<PageContent>, ProcessError> {
    worker(stub)
        .page_texts(Bytes::from_static(b"%PDF-1.7 the document"), range, words)
        .await
}

fn range(first: u32, last: Option<u32>) -> PageRange {
    PageRange::new(n(first), last.map(n)).unwrap()
}

#[tokio::test]
async fn text_is_asked_for_by_page_range_and_words_with_the_document_as_the_body() {
    let s = stub(200, text_body(&[2, 3, 4])).await;
    texts(&s, range(2, Some(4)), true).await.unwrap();
    let [request] = &s.requests()[..] else {
        panic!("one request")
    };
    assert_eq!(request.method, Method::POST);
    assert_eq!(request.path, "/v1/text");
    assert_eq!(
        query_of(request),
        pairs(&[("from", "2"), ("to", "4"), ("words", "true")])
    );
    assert_eq!(request.body, b"%PDF-1.7 the document");
    assert_eq!(request.headers["authorization"], format!("Bearer {TOKEN}"));
    assert_eq!(request.headers["content-type"], "application/octet-stream");
}

#[tokio::test]
async fn an_open_ended_range_leaves_out_the_end_and_words_can_be_left_out() {
    let mut page = text_page(3);
    page.as_object_mut().unwrap().remove("words");
    let s = stub(200, serde_json::to_vec(&json!({"pages": [page]})).unwrap()).await;
    texts(&s, range(3, None), false).await.unwrap();
    assert_eq!(
        query_of(&s.requests()[0]),
        pairs(&[("from", "3"), ("words", "false")])
    );
}

#[tokio::test]
async fn a_good_answer_becomes_checked_pages_with_words_and_fonts() {
    let s = stub(200, text_body(&[2, 3])).await;
    let pages = texts(&s, range(2, Some(3)), true).await.unwrap();
    assert_eq!(pages.len(), 2);
    let page = &pages[0];
    assert_eq!(page.page(), n(2));
    assert_eq!(page.source(), PageSource::TextLayer);
    assert_eq!(page.text(), "Total 100");
    assert_eq!((page.width_pt(), page.height_pt()), (595.0, 842.0));
    assert_eq!(page.fonts().len(), 1);
    let words = page.words().unwrap();
    assert_eq!((words[0].start, words[0].end, words[0].bounds.w), (0, 5, 100));
    assert!(words[0].confidence.is_none());
    assert_eq!(pages[1].page(), n(3));
}

#[tokio::test]
async fn a_page_with_no_text_layer_and_pages_without_words_are_understood() {
    let body = json!({"pages": [
        {"page": 1, "source": "none", "text": "", "width_pt": 200.0, "height_pt": 200.0, "fonts": []},
        {"page": 2, "source": "text_layer", "text": "Total", "width_pt": 200.0, "height_pt": 200.0,
         "fonts": []},
    ]});
    let s = stub(200, serde_json::to_vec(&body).unwrap()).await;
    let pages = texts(&s, range(1, Some(2)), false).await.unwrap();
    assert_eq!(pages[0].source(), PageSource::None);
    assert!(pages[0].words().is_none());
    assert!(pages[1].words().is_none());
}

#[tokio::test]
async fn fields_the_client_does_not_know_yet_are_ignored() {
    let mut page = text_page(1);
    page["language"] = json!("en");
    let body = json!({"pages": [page], "future": {"a": 1}});
    let s = stub(200, serde_json::to_vec(&body).unwrap()).await;
    assert_eq!(texts(&s, range(1, Some(1)), true).await.unwrap().len(), 1);
}

#[tokio::test]
async fn the_pages_must_be_exactly_the_ones_asked_for_in_order() {
    let wrong: Vec<(&str, Vec<u32>, PageRange)> = vec![
        ("none", vec![], range(1, Some(2))),
        ("too few", vec![2], range(2, Some(3))),
        ("too many", vec![2, 3, 4], range(2, Some(3))),
        ("a gap", vec![2, 4], range(2, Some(3))),
        ("the wrong start", vec![1, 2], range(2, Some(3))),
        ("backwards", vec![3, 2], range(2, Some(3))),
        ("repeated", vec![2, 2], range(2, Some(3))),
        ("open range, none", vec![], range(2, None)),
        ("open range, wrong start", vec![3], range(2, None)),
        ("open range, a gap", vec![2, 4], range(2, None)),
    ];
    for (name, pages, asked) in wrong {
        let s = stub(200, text_body(&pages)).await;
        let result = texts(&s, asked, true).await;
        assert!(
            matches!(result, Err(ProcessError::BadResponse(_))),
            "{name}: {result:?}"
        );
    }
    let many: Vec<u32> = (1..=51).collect();
    let s = stub(200, text_body(&many)).await;
    assert!(matches!(
        texts(&s, range(1, None), true).await,
        Err(ProcessError::BadResponse(_))
    ));
    let fifty: Vec<u32> = (1..=50).collect();
    let s = stub(200, text_body(&fifty)).await;
    assert_eq!(texts(&s, range(1, None), true).await.unwrap().len(), 50);
}

#[tokio::test]
async fn an_answer_that_is_not_what_was_promised_is_a_bad_response_that_repeats_nothing() {
    let mut cases: Vec<(&str, Value)> = Vec::new();
    let with = |key: &str, value: Value| {
        let mut page = text_page(1);
        page[key] = value;
        json!({"pages": [page]})
    };
    cases.push(("source", with("source", json!("ocr"))));
    cases.push(("unknown source", with("source", json!("scan"))));
    cases.push(("text type", with("text", json!(5))));
    cases.push(("control character", with("text", json!("Total\u{7} 100"))));
    cases.push(("page type", with("page", json!("1"))));
    cases.push(("page fraction", with("page", json!(1.5))));
    cases.push(("negative page", with("page", json!(-1))));
    cases.push(("zero page", with("page", json!(0))));
    cases.push(("width zero", with("width_pt", json!(0))));
    cases.push(("width string", with("width_pt", json!("wide"))));
    cases.push(("fonts missing", {
        let mut page = text_page(1);
        page.as_object_mut().unwrap().remove("fonts");
        json!({"pages": [page]})
    }));
    cases.push((
        "font kind",
        with(
            "fonts",
            json!([{"kind": "type9", "has_to_unicode": false, "encoding": "none", "embedded": false}]),
        ),
    ));
    cases.push((
        "font extra",
        with(
            "fonts",
            json!([{"kind": "type1", "has_to_unicode": false, "encoding": "none", "embedded": false, "x": 1}]),
        ),
    ));
    cases.push(("word past the end", with("words", json!([word(0, 99)]))));
    cases.push(("word spans a space", with("words", json!([word(0, 9)]))));
    cases.push(("word backwards", with("words", json!([word(5, 3)]))));
    cases.push(("words overlap", with("words", json!([word(0, 5), word(4, 9)]))));
    cases.push((
        "word box outside",
        with(
            "words",
            json!([{"start": 0, "end": 5, "x": 999, "y": 0, "w": 5, "h": 5}]),
        ),
    ));
    cases.push((
        "word box flat",
        with(
            "words",
            json!([{"start": 0, "end": 5, "x": 0, "y": 0, "w": 0, "h": 5}]),
        ),
    ));
    cases.push((
        "word fraction",
        with(
            "words",
            json!([{"start": 0.5, "end": 5, "x": 0, "y": 0, "w": 1, "h": 5}]),
        ),
    ));
    cases.push((
        "word confidence on a text page",
        with(
            "words",
            json!([{"start": 0, "end": 5, "x": 0, "y": 0, "w": 1, "h": 5, "confidence": 5}]),
        ),
    ));
    cases.push(("words not a list", with("words", json!("none"))));
    cases.push(("agreement on a text page", with("agreement", json!(500))));
    cases.push(("pages missing", json!({"other": []})));
    cases.push(("pages not a list", json!({"pages": "none"})));
    cases.push(("an array", json!([1, 2])));
    for (name, body) in cases {
        let s = stub(200, serde_json::to_vec(&body).unwrap()).await;
        let error = texts(&s, range(1, Some(1)), true).await.unwrap_err();
        assert!(matches!(error, ProcessError::BadResponse(_)), "{name}: {error:?}");
        assert!(!error.to_string().contains("Total"), "{name}");
    }
    for raw in [&b"not json"[..], b"", b"{\"pages\":"] {
        let s = stub(200, raw).await;
        assert!(matches!(
            texts(&s, range(1, Some(1)), true).await,
            Err(ProcessError::BadResponse(_))
        ));
    }
}

#[tokio::test]
async fn a_long_text_answer_is_read_in_full_up_to_thirty_two_mebibytes() {
    assert_eq!(MAX_TEXT_RESPONSE_BYTES, 32 * 1024 * 1024);
    // A page of a mebibyte of text: far more than the 64 KiB a status answer may be.
    let mut page = text_page(1);
    page["text"] = json!("a".repeat(1024 * 1024));
    page["words"] = json!([word(0, 1000)]);
    let s = stub(200, serde_json::to_vec(&json!({"pages": [page]})).unwrap()).await;
    let pages = texts(&s, range(1, Some(1)), true).await.unwrap();
    assert_eq!(pages[0].text().len(), 1024 * 1024);

    let s = stub(200, vec![b' '; MAX_TEXT_RESPONSE_BYTES + 1]).await;
    assert!(matches!(
        texts(&s, range(1, Some(1)), true).await,
        Err(ProcessError::BadResponse(_))
    ));
}

#[tokio::test]
async fn the_ways_a_text_call_can_be_refused_are_named() {
    let cases: Vec<(u16, Value, ProcessError)> = vec![
        (
            422,
            json!({"error": "the page range is outside the document", "pages": 2}),
            ProcessError::Document(DocumentProblem::PageOutOfRange { pages: 2 }),
        ),
        (
            422,
            json!({"error": "a page holds more text than the worker will report"}),
            ProcessError::Document(DocumentProblem::PageTooLarge),
        ),
        (
            422,
            json!({"error": "the page range is outside the document"}),
            ProcessError::Document(DocumentProblem::Unreadable),
        ),
        (
            422,
            json!({"error": "the page range is outside the document", "pages": -1}),
            ProcessError::Document(DocumentProblem::Unreadable),
        ),
        (
            422,
            json!({"error": "the page range is outside the document", "pages": 5_000_000_000u64}),
            ProcessError::Document(DocumentProblem::Unreadable),
        ),
        (
            400,
            json!({"error": "invalid request"}),
            ProcessError::Unexpected { status: 400 },
        ),
        (401, json!({"error": "authentication failed"}), ProcessError::Auth),
        (
            422,
            json!({"error": "the document is encrypted"}),
            ProcessError::Document(DocumentProblem::Encrypted),
        ),
        (
            504,
            json!({"error": "the document took too long to inspect"}),
            ProcessError::Server { status: 504 },
        ),
    ];
    for (status, body, expected) in cases {
        let s = stub(status, serde_json::to_vec(&body).unwrap()).await;
        assert_eq!(
            texts(&s, range(1, Some(2)), true).await,
            Err(expected),
            "{status} {body}"
        );
    }
}

// ================================================================= ocr ====

fn ocr_page_json(number: u32) -> Value {
    json!({
        "page": number,
        "source": "ocr",
        "text": "Total 100",
        "width_pt": 595.0,
        "height_pt": 842.0,
        "fonts": [],
        "words": [
            {"start": 0, "end": 5, "x": 10, "y": 20, "w": 100, "h": 30, "confidence": 900},
            {"start": 6, "end": 9, "x": 120, "y": 20, "w": 100, "h": 30, "confidence": 850},
        ],
    })
}

async fn ocr(stub: &Stub, page: u32, agreement: bool) -> Result<PageContent, ProcessError> {
    worker(stub)
        .ocr_page(Bytes::from_static(b"%PDF-1.7 the document"), n(page), agreement)
        .await
}

fn ocr_body(page: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"pages": [page]})).unwrap()
}

#[tokio::test]
async fn ocr_is_asked_for_one_page_with_or_without_the_agreement() {
    let s = stub(200, ocr_body(ocr_page_json(3))).await;
    ocr(&s, 3, true).await.unwrap();
    let request = &s.requests()[0];
    assert_eq!(request.path, "/v1/ocr");
    assert_eq!(query_of(request), pairs(&[("page", "3"), ("agreement", "true")]));
    assert_eq!(request.body, b"%PDF-1.7 the document");
    ocr(&s, 3, false).await.unwrap();
    assert_eq!(
        query_of(&s.requests()[1]),
        pairs(&[("page", "3"), ("agreement", "false")])
    );
}

#[tokio::test]
async fn an_ocr_answer_has_words_with_confidence_and_perhaps_an_agreement() {
    let mut page = ocr_page_json(3);
    page["agreement"] = json!(730);
    let s = stub(200, ocr_body(page)).await;
    let read = ocr(&s, 3, true).await.unwrap();
    assert_eq!(read.source(), PageSource::Ocr);
    assert_eq!(read.agreement(), Some(730));
    assert_eq!(read.words().unwrap()[1].confidence, Some(850));
    assert!(read.fonts().is_empty());

    let s = stub(200, ocr_body(ocr_page_json(3))).await;
    assert_eq!(
        ocr(&s, 3, true).await.unwrap().agreement(),
        None,
        "left out when it could not be worked out"
    );
}

#[tokio::test]
async fn an_ocr_answer_that_is_not_what_was_promised_is_a_bad_response() {
    let with = |key: &str, value: Value| {
        let mut page = ocr_page_json(3);
        page[key] = value;
        ocr_body(page)
    };
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("wrong page", ocr_body(ocr_page_json(4))),
        ("text layer source", with("source", json!("text_layer"))),
        ("no words", {
            let mut page = ocr_page_json(3);
            page.as_object_mut().unwrap().remove("words");
            ocr_body(page)
        }),
        (
            "fonts present",
            with(
                "fonts",
                json!([{"kind": "type1", "has_to_unicode": false, "encoding": "none", "embedded": false}]),
            ),
        ),
        (
            "no confidence",
            with(
                "words",
                json!([{"start": 0, "end": 5, "x": 10, "y": 20, "w": 100, "h": 30}]),
            ),
        ),
        (
            "confidence over",
            with(
                "words",
                json!([{"start": 0, "end": 5, "x": 10, "y": 20, "w": 100, "h": 30, "confidence": 1001}]),
            ),
        ),
        ("agreement over", with("agreement", json!(1001))),
        ("agreement type", with("agreement", json!("high"))),
        (
            "two pages",
            serde_json::to_vec(&json!({"pages": [ocr_page_json(3), ocr_page_json(3)]})).unwrap(),
        ),
        ("no pages", serde_json::to_vec(&json!({"pages": []})).unwrap()),
    ];
    for (name, body) in cases {
        let s = stub(200, body).await;
        let result = ocr(&s, 3, true).await;
        assert!(
            matches!(result, Err(ProcessError::BadResponse(_))),
            "{name}: {result:?}"
        );
    }
}

#[tokio::test]
async fn an_agreement_nobody_asked_for_is_not_believed() {
    let mut page = ocr_page_json(3);
    page["agreement"] = json!(730);
    let s = stub(200, ocr_body(page)).await;
    assert!(matches!(
        ocr(&s, 3, false).await,
        Err(ProcessError::BadResponse(_))
    ));
}

#[tokio::test]
async fn ocr_that_is_not_set_up_is_not_available_and_any_other_503_is_a_server_failure() {
    let s = stub(
        503,
        serde_json::to_vec(&json!({"error": "OCR is not available"})).unwrap(),
    )
    .await;
    assert_eq!(ocr(&s, 1, false).await, Err(ProcessError::NotAvailable));
    for body in [
        json!({"error": "something else"}),
        json!({}),
        json!("OCR is not available"),
    ] {
        let s = stub(503, serde_json::to_vec(&body).unwrap()).await;
        assert_eq!(
            ocr(&s, 1, false).await,
            Err(ProcessError::Server { status: 503 }),
            "{body}"
        );
    }
    let s = stub(
        500,
        serde_json::to_vec(&json!({"error": "OCR is not available"})).unwrap(),
    )
    .await;
    assert_eq!(ocr(&s, 1, false).await, Err(ProcessError::Server { status: 500 }));
}

// ============================================================== render ====

fn picture_script(content_type: &str, width: &str, height: &str, body: &[u8]) -> Script {
    let mut s = script(200, body);
    s.headers = vec![
        ("content-type".into(), content_type.into()),
        ("x-image-width".into(), width.into()),
        ("x-image-height".into(), height.into()),
    ];
    s
}

async fn render(stub: &Stub, options: RenderOptions) -> Result<crate::RenderedPage, ProcessError> {
    worker(stub)
        .render_page(Bytes::from_static(b"%PDF-1.7 the document"), n(2), options)
        .await
}

#[tokio::test]
async fn a_page_is_drawn_with_the_options_in_the_query_and_comes_back_as_a_picture() {
    let s = serve(picture_script("image/jpeg", "1000", "500", JPEG)).await;
    let options = RenderOptions::new(200, ImageFormat::Jpeg, 1500).unwrap();
    let page = render(&s, options).await.unwrap();
    assert_eq!(
        (page.media(), page.width(), page.height(), page.bytes().as_ref()),
        (ImageType::Jpeg, 1000, 500, JPEG)
    );
    let request = &s.requests()[0];
    assert_eq!(request.path, "/v1/render");
    assert_eq!(
        query_of(request),
        pairs(&[
            ("page", "2"),
            ("dpi", "200"),
            ("format", "jpeg"),
            ("max_edge", "1500")
        ])
    );
    assert_eq!(request.body, b"%PDF-1.7 the document");
}

#[tokio::test]
async fn the_default_options_are_150_dpi_png_at_2000() {
    let s = serve(picture_script("image/png", "400", "300", PNG)).await;
    render(&s, RenderOptions::default()).await.unwrap();
    assert_eq!(
        query_of(&s.requests()[0]),
        pairs(&[
            ("page", "2"),
            ("dpi", "150"),
            ("format", "png"),
            ("max_edge", "2000")
        ])
    );
}

#[tokio::test]
async fn a_png_asked_for_may_come_back_as_a_jpeg_but_not_the_other_way_round() {
    let s = serve(picture_script("image/jpeg", "400", "300", JPEG)).await;
    let page = render(&s, RenderOptions::default()).await.unwrap();
    assert_eq!(page.media(), ImageType::Jpeg, "a big PNG is sent as a JPEG");
    let s = serve(picture_script("image/png", "400", "300", PNG)).await;
    let jpeg_please = RenderOptions::new(150, ImageFormat::Jpeg, 2000).unwrap();
    assert!(matches!(
        render(&s, jpeg_please).await,
        Err(ProcessError::BadResponse(_))
    ));
}

#[tokio::test]
async fn a_picture_that_is_not_what_the_headers_say_is_a_bad_response() {
    let cases: Vec<(&str, Script)> = vec![
        (
            "wrong content type",
            picture_script("image/gif", "400", "300", PNG),
        ),
        ("no content type", picture_script("", "400", "300", PNG)),
        (
            "content type with a parameter",
            picture_script("image/png; x=1", "400", "300", PNG),
        ),
        (
            "bytes of another kind",
            picture_script("image/png", "400", "300", JPEG),
        ),
        ("empty", picture_script("image/png", "400", "300", b"")),
        ("width zero", picture_script("image/png", "0", "300", PNG)),
        ("height zero", picture_script("image/png", "400", "0", PNG)),
        (
            "width not a number",
            picture_script("image/png", "wide", "300", PNG),
        ),
        (
            "width with a sign",
            picture_script("image/png", "+400", "300", PNG),
        ),
        (
            "width with a space inside",
            picture_script("image/png", "4 00", "300", PNG),
        ),
        ("width fraction", picture_script("image/png", "4.5", "300", PNG)),
        ("width missing", picture_script("image/png", "", "300", PNG)),
        (
            "over the longest side asked",
            picture_script("image/png", "2001", "300", PNG),
        ),
        (
            "over the longest side asked, tall",
            picture_script("image/png", "300", "2001", PNG),
        ),
        (
            "absurd size",
            picture_script("image/png", "99999999999999", "300", PNG),
        ),
    ];
    for (name, script) in cases {
        let s = serve(script).await;
        let result = render(&s, RenderOptions::default()).await;
        assert!(
            matches!(result, Err(ProcessError::BadResponse(_))),
            "{name}: {result:?}"
        );
    }
}

#[tokio::test]
async fn the_longest_side_asked_for_is_allowed() {
    let s = serve(picture_script("image/png", "2000", "2000", PNG)).await;
    assert!(render(&s, RenderOptions::default()).await.is_ok());
}

#[tokio::test]
async fn a_picture_is_read_up_to_six_mebibytes_and_not_a_byte_more() {
    assert_eq!(MAX_IMAGE_RESPONSE_BYTES, 6 * 1024 * 1024);
    let mut exact = PNG.to_vec();
    exact.resize(MAX_IMAGE_RESPONSE_BYTES, 0);
    let s = serve(picture_script("image/png", "400", "300", &exact)).await;
    assert!(render(&s, RenderOptions::default()).await.is_ok());
    exact.push(0);
    let s = serve(picture_script("image/png", "400", "300", &exact)).await;
    assert!(matches!(
        render(&s, RenderOptions::default()).await,
        Err(ProcessError::BadResponse(_))
    ));
}

#[tokio::test]
async fn the_ways_a_render_can_be_refused_are_named() {
    let cases: Vec<(u16, Value, ProcessError)> = vec![
        (
            422,
            json!({"error": "the page is too large to render"}),
            ProcessError::Document(DocumentProblem::PageTooBig),
        ),
        (
            422,
            json!({"error": "the image is too large"}),
            ProcessError::Document(DocumentProblem::ImageTooLarge),
        ),
        (
            422,
            json!({"error": "the page range is outside the document", "pages": 1}),
            ProcessError::Document(DocumentProblem::PageOutOfRange { pages: 1 }),
        ),
        (
            415,
            json!({"error": "unsupported file type"}),
            ProcessError::Document(DocumentProblem::UnsupportedType),
        ),
    ];
    for (status, body, expected) in cases {
        let s = stub(status, serde_json::to_vec(&body).unwrap()).await;
        assert_eq!(
            render(&s, RenderOptions::default()).await,
            Err(expected),
            "{body}"
        );
    }
}

// =============================================================== mask ====

fn box_of(x: u16, y: u16, w: u16, h: u16) -> MaskBox {
    MaskBox::new(x, y, w, h).unwrap()
}

async fn mask(stub: &Stub, boxes: Vec<MaskBox>) -> Result<crate::RenderedPage, ProcessError> {
    worker(stub)
        .mask_image(Bytes::from_static(PNG), boxes, ImageFormat::Png, 2000)
        .await
}

#[tokio::test]
async fn a_mask_request_is_json_with_the_picture_in_base64_and_the_boxes() {
    let s = serve(picture_script("image/png", "400", "300", PNG)).await;
    let page = mask(&s, vec![box_of(10, 20, 300, 40), box_of(0, 0, 1000, 1000)])
        .await
        .unwrap();
    assert_eq!(
        (page.media(), page.width(), page.height()),
        (ImageType::Png, 400, 300)
    );
    let request = &s.requests()[0];
    assert_eq!(request.method, Method::POST);
    assert_eq!(request.path, "/v1/mask");
    assert_eq!(
        query_of(request),
        pairs(&[("format", "png"), ("max_edge", "2000")])
    );
    assert_eq!(request.headers["content-type"], "application/json");
    assert_eq!(request.headers["authorization"], format!("Bearer {TOKEN}"));
    let sent: Value = serde_json::from_slice(&request.body).unwrap();
    assert_eq!(
        sent,
        json!({
            "image": base64::engine::general_purpose::STANDARD.encode(PNG),
            "boxes": [{"x": 10, "y": 20, "w": 300, "h": 40}, {"x": 0, "y": 0, "w": 1000, "h": 1000}],
        })
    );
}

#[tokio::test]
async fn the_format_and_the_longest_side_are_passed_on() {
    let s = serve(picture_script("image/jpeg", "400", "300", JPEG)).await;
    worker(&s)
        .mask_image(Bytes::from_static(PNG), Vec::new(), ImageFormat::Jpeg, 1500)
        .await
        .unwrap();
    assert_eq!(
        query_of(&s.requests()[0]),
        pairs(&[("format", "jpeg"), ("max_edge", "1500")])
    );
}

#[tokio::test]
async fn up_to_five_hundred_boxes_are_sent_and_more_are_not() {
    let s = serve(picture_script("image/png", "400", "300", PNG)).await;
    assert!(mask(&s, vec![box_of(0, 0, 10, 10); 500]).await.is_ok());
    let refused = mask(&s, vec![box_of(0, 0, 10, 10); 501]).await;
    assert_eq!(refused, Err(ProcessError::Unexpected { status: 400 }));
    assert_eq!(s.requests().len(), 1, "the second request was never sent");
}

#[tokio::test]
async fn a_masked_picture_is_checked_like_a_drawn_one() {
    let cases: Vec<(&str, Script)> = vec![
        (
            "wrong content type",
            picture_script("text/html", "400", "300", PNG),
        ),
        (
            "bytes of another kind",
            picture_script("image/jpeg", "400", "300", PNG),
        ),
        (
            "over the longest side",
            picture_script("image/png", "2001", "300", PNG),
        ),
        ("no size", picture_script("image/png", "", "", PNG)),
    ];
    for (name, script) in cases {
        let s = serve(script).await;
        let result = mask(&s, vec![box_of(0, 0, 10, 10)]).await;
        assert!(
            matches!(result, Err(ProcessError::BadResponse(_))),
            "{name}: {result:?}"
        );
    }
}

#[tokio::test]
async fn the_ways_a_mask_can_be_refused_are_named() {
    let cases: Vec<(u16, Value, ProcessError)> = vec![
        (
            415,
            json!({"error": "unsupported file type"}),
            ProcessError::Document(DocumentProblem::UnsupportedType),
        ),
        (
            413,
            json!({"error": "file too large", "max_bytes": 100}),
            ProcessError::Document(DocumentProblem::TooLarge { max_bytes: 100 }),
        ),
        (
            422,
            json!({"error": "the image is too large"}),
            ProcessError::Document(DocumentProblem::ImageTooLarge),
        ),
        (
            422,
            json!({"error": "the document could not be read"}),
            ProcessError::Document(DocumentProblem::Unreadable),
        ),
        (
            400,
            json!({"error": "invalid request"}),
            ProcessError::Unexpected { status: 400 },
        ),
    ];
    for (status, body, expected) in cases {
        let s = stub(status, serde_json::to_vec(&body).unwrap()).await;
        assert_eq!(
            mask(&s, vec![box_of(0, 0, 10, 10)]).await,
            Err(expected),
            "{body}"
        );
    }
}

// ======================================================= what they share ====

#[tokio::test]
async fn a_redirect_is_not_followed_by_any_of_the_new_calls() {
    let elsewhere = stub(200, text_body(&[1])).await;
    let mut redirect = script(302, "");
    redirect.headers = vec![("location".into(), format!("{}/v1/text", elsewhere.url))];
    let s = serve(redirect).await;
    assert!(matches!(
        texts(&s, range(1, Some(1)), true).await,
        Err(ProcessError::Unexpected { status: 302 })
    ));
    assert!(matches!(
        ocr(&s, 1, false).await,
        Err(ProcessError::Unexpected { status: 302 })
    ));
    assert!(matches!(
        render(&s, RenderOptions::default()).await,
        Err(ProcessError::Unexpected { status: 302 })
    ));
    assert!(matches!(
        mask(&s, Vec::new()).await,
        Err(ProcessError::Unexpected { status: 302 })
    ));
    assert!(elsewhere.requests().is_empty());
}

#[tokio::test]
async fn a_slow_worker_is_a_timeout_for_the_new_calls_too() {
    let mut slow = script(200, text_body(&[1]));
    slow.delay = Duration::from_millis(500);
    let s = serve(slow).await;
    let impatient = worker(&s).with_timeout(Duration::from_millis(100)).unwrap();
    let result = impatient
        .page_texts(Bytes::from_static(b"x"), range(1, Some(1)), true)
        .await;
    assert_eq!(result, Err(ProcessError::Timeout));
}

#[tokio::test]
async fn a_worker_that_is_not_there_is_unavailable_for_the_new_calls_too() {
    let s = stub(200, "").await;
    let gone = HttpWorker::new("http://127.0.0.1:1", TOKEN).unwrap();
    drop(s);
    let result = gone.ocr_page(Bytes::from_static(b"x"), n(1), false).await;
    assert_eq!(result, Err(ProcessError::Unavailable));
}

#[tokio::test]
async fn an_error_never_repeats_what_the_server_said_for_the_new_calls() {
    let s = stub(422, "SECRET text from a document").await;
    let errors = [
        texts(&s, range(1, Some(1)), true).await.unwrap_err(),
        ocr(&s, 1, false).await.unwrap_err(),
        render(&s, RenderOptions::default()).await.unwrap_err(),
        mask(&s, Vec::new()).await.unwrap_err(),
    ];
    for error in errors {
        assert_eq!(error, ProcessError::Document(DocumentProblem::Unreadable));
        assert!(!format!("{error} {error:?}").contains("SECRET"));
    }
}

#[tokio::test]
async fn a_picture_is_a_one_page_document_when_inspected() {
    for (media, kind) in [
        ("image/png", crate::DocumentKind::Png),
        ("image/jpeg", crate::DocumentKind::Jpeg),
    ] {
        let body =
            format!(r#"{{"media_type":"{media}","pages":1,"pages_with_text":0,"has_text_layer":false}}"#);
        let s = stub(200, body).await;
        let found = worker(&s).inspect(Bytes::from_static(b"x")).await.unwrap();
        assert_eq!(
            (found.kind(), found.pages(), found.has_text_layer()),
            (kind, 1, false)
        );
    }
}

#[tokio::test]
async fn words_asked_for_but_not_given_are_refused() {
    let mut page = text_page(1);
    page.as_object_mut().unwrap().remove("words");
    let s = stub(200, serde_json::to_vec(&json!({"pages": [page]})).unwrap()).await;
    assert!(matches!(
        texts(&s, range(1, Some(1)), true).await,
        Err(ProcessError::BadResponse(_))
    ));
}

#[tokio::test]
async fn every_part_of_a_word_box_is_read_to_the_right_place() {
    let body = json!({"pages": [{
        "page": 1, "source": "text_layer", "text": "Total 100",
        "width_pt": 595.0, "height_pt": 842.0, "fonts": [],
        "words": [{"start": 0, "end": 5, "x": 11, "y": 22, "w": 333, "h": 44}],
    }]});
    let s = stub(200, serde_json::to_vec(&body).unwrap()).await;
    let pages = texts(&s, range(1, Some(1)), true).await.unwrap();
    let word = pages[0].words().unwrap()[0];
    assert_eq!((word.start, word.end), (0, 5));
    let b = word.bounds;
    assert_eq!((b.x, b.y, b.w, b.h), (11, 22, 333, 44));
}

#[tokio::test]
async fn the_confidence_of_an_ocr_word_is_read_to_the_right_place() {
    let s = stub(200, ocr_body(ocr_page_json(3))).await;
    let words = ocr(&s, 3, false).await.unwrap().words().unwrap().to_vec();
    assert_eq!([words[0].confidence, words[1].confidence], [Some(900), Some(850)]);
    assert_eq!(words[1].bounds.x, 120);
}

#[tokio::test]
async fn a_page_read_by_ocr_is_not_an_answer_to_a_text_call_and_the_other_way_round() {
    // Both are well-formed pages; each is the wrong kind for the call.
    let mut ocr_shaped = ocr_page_json(1);
    ocr_shaped["words"] = ocr_page_json(1)["words"].clone();
    let s = stub(200, serde_json::to_vec(&json!({"pages": [ocr_shaped]})).unwrap()).await;
    let result = texts(&s, range(1, Some(1)), true).await;
    assert!(matches!(result, Err(ProcessError::BadResponse(_))), "{result:?}");

    let mut layer = text_page(3);
    layer["fonts"] = json!([]);
    let s = stub(200, serde_json::to_vec(&json!({"pages": [layer]})).unwrap()).await;
    let result = ocr(&s, 3, false).await;
    assert!(matches!(result, Err(ProcessError::BadResponse(_))), "{result:?}");
}
