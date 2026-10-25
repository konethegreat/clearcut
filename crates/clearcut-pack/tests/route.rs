// Written by Kone Tshivhinda & Claude
//! Which text goes with a page, and how it is labelled, from the page's reading route
//! (ADR-0017 and ADR-0018 section 4).

use clearcut_core::page_read::{PageTextSource, ReadRoute};
use clearcut_pack::{PageText, TextSource};

fn text_for(route: ReadRoute, source: PageTextSource, content: &str) -> Option<PageText> {
    PageText::for_route(route, source, content.to_string())
}

fn labelled(text: Option<PageText>) -> Option<(TextSource, String)> {
    text.map(|t| (t.source, t.content))
}

#[test]
fn a_trusted_text_layer_is_sent_as_the_text_layer() {
    assert_eq!(
        labelled(text_for(
            ReadRoute::TextAndImage,
            PageTextSource::TextLayer,
            "Invoice 1"
        )),
        Some((TextSource::TextLayer, "Invoice 1".to_string()))
    );
}

#[test]
fn a_doubtful_text_layer_is_sent_as_unverified() {
    assert_eq!(
        labelled(text_for(
            ReadRoute::ImageFirst,
            PageTextSource::TextLayer,
            "Invoice 1"
        )),
        Some((TextSource::UnverifiedTextLayer, "Invoice 1".to_string()))
    );
}

#[test]
fn ocr_text_of_a_picture_page_is_sent_as_ocr() {
    assert_eq!(
        labelled(text_for(ReadRoute::ImageOnly, PageTextSource::Ocr, "Invoice 1")),
        Some((TextSource::Ocr, "Invoice 1".to_string()))
    );
}

#[test]
fn a_page_with_no_text_sends_none() {
    for route in ReadRoute::ALL {
        assert_eq!(
            labelled(text_for(route, PageTextSource::None, "ignored")),
            None,
            "{route:?}"
        );
    }
}

#[test]
fn text_that_is_only_white_space_is_no_text() {
    for content in ["", "   ", "\n\t \n"] {
        for (route, source) in [
            (ReadRoute::TextAndImage, PageTextSource::TextLayer),
            (ReadRoute::ImageFirst, PageTextSource::TextLayer),
            (ReadRoute::ImageOnly, PageTextSource::Ocr),
        ] {
            assert_eq!(
                labelled(text_for(route, source, content)),
                None,
                "{content:?} {route:?}"
            );
        }
    }
}

#[test]
fn a_combination_a_reading_can_never_produce_sends_no_text() {
    // A rejected text layer is never sent, and OCR text only goes with a picture-only page.
    for (route, source) in [
        (ReadRoute::ImageOnly, PageTextSource::TextLayer),
        (ReadRoute::TextAndImage, PageTextSource::Ocr),
        (ReadRoute::ImageFirst, PageTextSource::Ocr),
    ] {
        assert_eq!(
            labelled(text_for(route, source, "Invoice 1")),
            None,
            "{route:?} {source:?}"
        );
    }
}

#[test]
fn the_text_is_passed_on_exactly() {
    let content = "  Invoice\n  1  ";
    assert_eq!(
        labelled(text_for(
            ReadRoute::TextAndImage,
            PageTextSource::TextLayer,
            content
        )),
        Some((TextSource::TextLayer, content.to_string()))
    );
}
