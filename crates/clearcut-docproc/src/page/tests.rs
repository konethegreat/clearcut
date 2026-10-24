// Written by Kone Tshivhinda & Claude
use super::*;
use clearcut_core::text_quality::{FontEncoding, FontKind};

fn n(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).unwrap()
}

fn boxed(x: u16, y: u16, w: u16, h: u16) -> WordBox {
    WordBox { x, y, w, h }
}

fn word(start: usize, end: usize) -> PageWord {
    PageWord {
        start,
        end,
        bounds: boxed(10, 20, 100, 30),
        confidence: None,
    }
}

fn certain(start: usize, end: usize, confidence: u16) -> PageWord {
    PageWord {
        confidence: Some(confidence),
        ..word(start, end)
    }
}

fn font() -> FontFact {
    FontFact {
        kind: FontKind::Type1,
        has_to_unicode: false,
        encoding: FontEncoding::WinAnsi,
        embedded: false,
    }
}

/// A valid text-layer page: "Total 100", two words.
fn parts() -> PageParts {
    PageParts {
        page: n(1),
        source: PageSource::TextLayer,
        text: "Total 100".to_string(),
        width_pt: 595.0,
        height_pt: 842.0,
        fonts: vec![font()],
        words: Some(vec![word(0, 5), word(6, 9)]),
        agreement: None,
    }
}

fn ocr_parts() -> PageParts {
    PageParts {
        source: PageSource::Ocr,
        fonts: Vec::new(),
        words: Some(vec![certain(0, 5, 900), certain(6, 9, 850)]),
        ..parts()
    }
}

fn none_parts() -> PageParts {
    PageParts {
        source: PageSource::None,
        text: String::new(),
        fonts: Vec::new(),
        words: None,
        ..parts()
    }
}

fn rejected(parts: PageParts) -> InvalidPage {
    PageContent::from_parts(parts).unwrap_err()
}

// ---------- a good page ----------

#[test]
fn a_text_layer_page_is_kept_as_it_was_given() {
    let page = PageContent::from_parts(parts()).unwrap();
    assert_eq!(page.page(), n(1));
    assert_eq!(page.source(), PageSource::TextLayer);
    assert_eq!(page.text(), "Total 100");
    assert_eq!((page.width_pt(), page.height_pt()), (595.0, 842.0));
    assert_eq!(page.fonts(), [font()]);
    assert_eq!(page.words().unwrap().len(), 2);
    assert_eq!(page.agreement(), None);
}

#[test]
fn words_are_optional_on_a_text_page() {
    let page = PageContent::from_parts(PageParts {
        words: None,
        ..parts()
    })
    .unwrap();
    assert!(page.words().is_none());
}

#[test]
fn a_page_with_no_text_layer_is_empty_and_may_have_an_empty_list_of_words() {
    assert!(PageContent::from_parts(none_parts()).is_ok());
    let with_list = PageParts {
        words: Some(Vec::new()),
        ..none_parts()
    };
    assert!(PageContent::from_parts(with_list).is_ok());
}

#[test]
fn an_ocr_page_has_words_with_confidence_and_may_have_an_agreement() {
    let page = PageContent::from_parts(ocr_parts()).unwrap();
    assert_eq!(page.source(), PageSource::Ocr);
    assert_eq!(page.words().unwrap()[0].confidence, Some(900));
    for agreement in [None, Some(0), Some(730), Some(1000)] {
        let with = PageParts {
            agreement,
            ..ocr_parts()
        };
        assert_eq!(PageContent::from_parts(with).unwrap().agreement(), agreement);
    }
}

#[test]
fn an_ocr_page_with_nothing_found_is_empty() {
    let empty = PageParts {
        text: String::new(),
        words: Some(Vec::new()),
        ..ocr_parts()
    };
    assert!(PageContent::from_parts(empty).is_ok());
}

#[test]
fn the_debug_form_shows_sizes_and_never_the_text() {
    let page = PageContent::from_parts(PageParts {
        text: "SECRET 12345".to_string(),
        words: None,
        ..parts()
    })
    .unwrap();
    let shown = format!("{page:?}");
    assert!(!shown.contains("SECRET"));
    assert!(shown.contains("12 bytes"));
}

#[test]
fn the_source_names_are_the_workers() {
    assert_eq!(PageSource::TextLayer.as_str(), "text_layer");
    assert_eq!(PageSource::None.as_str(), "none");
    assert_eq!(PageSource::Ocr.as_str(), "ocr");
    for (name, source) in [
        ("text_layer", PageSource::TextLayer),
        ("none", PageSource::None),
        ("ocr", PageSource::Ocr),
    ] {
        assert_eq!(PageSource::parse(name), Some(source));
    }
    for bad in ["", "OCR", "Text_Layer", "ocr ", "scan"] {
        assert_eq!(PageSource::parse(bad), None, "{bad:?}");
    }
}

// ---------- the source decides what else may be there ----------

#[test]
fn a_page_with_no_text_layer_may_not_have_text_or_fonts() {
    let with_text = PageParts {
        text: "x".to_string(),
        ..none_parts()
    };
    assert_eq!(rejected(with_text), InvalidPage::Source);
    let with_fonts = PageParts {
        fonts: vec![font()],
        ..none_parts()
    };
    assert_eq!(rejected(with_fonts), InvalidPage::Source);
    let with_words = PageParts {
        words: Some(vec![word(0, 1)]),
        ..none_parts()
    };
    assert!(PageContent::from_parts(with_words).is_err());
}

#[test]
fn a_text_layer_page_needs_text_that_is_not_blank() {
    for text in ["", "   ", "\n\t"] {
        let blank = PageParts {
            text: text.to_string(),
            words: None,
            ..parts()
        };
        assert_eq!(rejected(blank), InvalidPage::Source, "{text:?}");
    }
}

#[test]
fn only_an_ocr_page_has_confidence_or_an_agreement() {
    let confident = PageParts {
        words: Some(vec![certain(0, 5, 900), word(6, 9)]),
        ..parts()
    };
    assert_eq!(rejected(confident), InvalidPage::Words);
    let agreeing = PageParts {
        agreement: Some(500),
        ..parts()
    };
    assert_eq!(rejected(agreeing), InvalidPage::Agreement);
    let none_agreeing = PageParts {
        agreement: Some(0),
        ..none_parts()
    };
    assert_eq!(rejected(none_agreeing), InvalidPage::Agreement);
}

#[test]
fn an_ocr_page_must_have_words_all_with_confidence_and_no_fonts() {
    let no_list = PageParts {
        words: None,
        ..ocr_parts()
    };
    assert_eq!(rejected(no_list), InvalidPage::Source);
    let missing = PageParts {
        words: Some(vec![certain(0, 5, 900), word(6, 9)]),
        ..ocr_parts()
    };
    assert_eq!(rejected(missing), InvalidPage::Words);
    let fonts = PageParts {
        fonts: vec![font()],
        ..ocr_parts()
    };
    assert_eq!(rejected(fonts), InvalidPage::Source);
    let text_without_words = PageParts {
        words: Some(Vec::new()),
        ..ocr_parts()
    };
    assert!(PageContent::from_parts(text_without_words).is_err());
    let words_without_text = PageParts {
        text: String::new(),
        ..ocr_parts()
    };
    assert!(PageContent::from_parts(words_without_text).is_err());
}

#[test]
fn an_agreement_is_at_most_a_thousand() {
    let ok = PageParts {
        agreement: Some(1000),
        ..ocr_parts()
    };
    assert!(PageContent::from_parts(ok).is_ok());
    let over = PageParts {
        agreement: Some(1001),
        ..ocr_parts()
    };
    assert_eq!(rejected(over), InvalidPage::Agreement);
}

// ---------- text, size, fonts ----------

#[test]
fn control_characters_other_than_line_feed_and_tab_are_refused() {
    for bad in ["a\r\nb", "a\u{7}b", "a\u{0}b", "a\u{1b}b", "a\u{85}b", "a\u{7f}b"] {
        let page = PageParts {
            text: bad.to_string(),
            words: None,
            ..parts()
        };
        assert_eq!(rejected(page), InvalidPage::Text, "{bad:?}");
    }
    let fine = PageParts {
        text: "Total\t100\nVAT 15".to_string(),
        words: None,
        ..parts()
    };
    assert!(PageContent::from_parts(fine).is_ok());
}

#[test]
fn a_page_of_text_is_at_most_a_mebibyte() {
    assert_eq!(MAX_PAGE_TEXT_BYTES, 1024 * 1024);
    let exact = PageParts {
        text: "a".repeat(MAX_PAGE_TEXT_BYTES),
        words: None,
        ..parts()
    };
    assert!(PageContent::from_parts(exact).is_ok());
    let over = PageParts {
        text: "a".repeat(MAX_PAGE_TEXT_BYTES + 1),
        words: None,
        ..parts()
    };
    assert_eq!(rejected(over), InvalidPage::Text);
}

#[test]
fn the_size_of_a_page_is_a_sensible_positive_number_of_points() {
    for bad in [
        0.0,
        -1.0,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        100_000.5,
        1e9,
    ] {
        let wide = PageParts {
            width_pt: bad,
            ..parts()
        };
        assert_eq!(rejected(wide), InvalidPage::Size, "{bad}");
        let tall = PageParts {
            height_pt: bad,
            ..parts()
        };
        assert_eq!(rejected(tall), InvalidPage::Size, "{bad}");
    }
    let edge = PageParts {
        width_pt: 100_000.0,
        height_pt: 0.001,
        ..parts()
    };
    assert!(PageContent::from_parts(edge).is_ok());
}

#[test]
fn at_most_sixty_four_fonts() {
    let ok = PageParts {
        fonts: vec![font(); 64],
        ..parts()
    };
    assert!(PageContent::from_parts(ok).is_ok());
    let over = PageParts {
        fonts: vec![font(); 65],
        ..parts()
    };
    assert_eq!(rejected(over), InvalidPage::Fonts);
}

// ---------- words ----------

#[test]
fn a_word_must_lie_inside_the_text_in_order_and_without_overlap() {
    let cases: Vec<(&str, Vec<PageWord>)> = vec![
        ("empty", vec![word(3, 3)]),
        ("backwards", vec![word(5, 3)]),
        ("past the end", vec![word(0, 10)]),
        ("overlapping", vec![word(0, 5), word(4, 9)]),
        ("out of order", vec![word(6, 9), word(0, 5)]),
        ("spans a space", vec![word(0, 9)]),
    ];
    for (name, words) in cases {
        let page = PageParts {
            words: Some(words),
            ..parts()
        };
        assert_eq!(rejected(page), InvalidPage::Words, "{name}");
    }
    let touching = PageParts {
        text: "abcdef".to_string(),
        words: Some(vec![word(0, 3), word(3, 6)]),
        ..parts()
    };
    assert!(PageContent::from_parts(touching).is_ok(), "words may touch");
}

#[test]
fn a_word_must_start_and_end_on_a_character() {
    // "é" is two bytes, so a word cannot end after the first of them.
    let page = |start, end| PageParts {
        text: "é au".to_string(),
        words: Some(vec![word(start, end)]),
        ..parts()
    };
    assert!(PageContent::from_parts(page(0, 2)).is_ok());
    assert_eq!(rejected(page(0, 1)), InvalidPage::Words);
    assert_eq!(rejected(page(1, 2)), InvalidPage::Words);
}

#[test]
fn a_box_must_have_a_size_and_lie_inside_the_page() {
    let bad_boxes = [
        boxed(0, 0, 0, 10),
        boxed(0, 0, 10, 0),
        boxed(995, 0, 10, 10),
        boxed(0, 995, 10, 10),
        boxed(1001, 0, 1, 1),
        boxed(0, 1001, 1, 1),
        boxed(0, 0, 1001, 1),
        boxed(u16::MAX, 0, u16::MAX, 1),
    ];
    for bounds in bad_boxes {
        let page = PageParts {
            words: Some(vec![PageWord { bounds, ..word(0, 5) }]),
            ..parts()
        };
        assert_eq!(rejected(page), InvalidPage::Words, "{bounds:?}");
    }
    let edge = PageParts {
        words: Some(vec![PageWord {
            bounds: boxed(999, 999, 1, 1),
            ..word(0, 5)
        }]),
        ..parts()
    };
    assert!(PageContent::from_parts(edge).is_ok());
    let whole = PageParts {
        words: Some(vec![PageWord {
            bounds: boxed(0, 0, 1000, 1000),
            ..word(0, 5)
        }]),
        ..parts()
    };
    assert!(PageContent::from_parts(whole).is_ok());
}

#[test]
fn confidence_is_at_most_a_thousand_and_may_be_zero() {
    for ok in [0, 1, 1000] {
        let page = PageParts {
            words: Some(vec![certain(0, 5, ok), certain(6, 9, 500)]),
            ..ocr_parts()
        };
        assert!(PageContent::from_parts(page).is_ok(), "{ok}");
    }
    let over = PageParts {
        words: Some(vec![certain(0, 5, 1001), certain(6, 9, 500)]),
        ..ocr_parts()
    };
    assert_eq!(rejected(over), InvalidPage::Words);
}

#[test]
fn at_most_twenty_thousand_words() {
    assert_eq!(MAX_WORDS_PER_PAGE, 20_000);
    let text = "a ".repeat(20_001);
    let words = |count: usize| -> Vec<PageWord> { (0..count).map(|i| word(i * 2, i * 2 + 1)).collect() };
    let ok = PageParts {
        text: text.clone(),
        words: Some(words(20_000)),
        ..parts()
    };
    assert!(PageContent::from_parts(ok).is_ok());
    let over = PageParts {
        text,
        words: Some(words(20_001)),
        ..parts()
    };
    assert_eq!(rejected(over), InvalidPage::Words);
}

#[test]
fn the_text_of_a_word_is_found_by_its_offsets() {
    let page = PageContent::from_parts(PageParts {
        text: "Café au".to_string(),
        words: Some(vec![word(0, 5), word(6, 8)]),
        ..parts()
    })
    .unwrap();
    let words = page.words().unwrap();
    assert_eq!(page.word_text(&words[0]), "Café");
    assert_eq!(page.word_text(&words[1]), "au");
}

// ---------- a drawn page ----------

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n and then the rest";
const JPEG: &[u8] = b"\xff\xd8\xff\xe0 and then the rest";

#[test]
fn a_rendered_page_is_a_picture_of_a_size_that_starts_like_one() {
    let png = RenderedPage::new(ImageType::Png, Bytes::from_static(PNG), 400, 300).unwrap();
    assert_eq!(
        (png.media(), png.width(), png.height()),
        (ImageType::Png, 400, 300)
    );
    assert_eq!(png.bytes(), PNG);
    assert!(RenderedPage::new(ImageType::Jpeg, Bytes::from_static(JPEG), 1, 1).is_ok());
}

#[test]
fn a_rendered_page_that_is_not_what_it_says_is_refused() {
    let png = || Bytes::from_static(PNG);
    let jpeg = || Bytes::from_static(JPEG);
    assert!(RenderedPage::new(ImageType::Jpeg, png(), 10, 10).is_err());
    assert!(RenderedPage::new(ImageType::Png, jpeg(), 10, 10).is_err());
    assert!(RenderedPage::new(ImageType::Png, Bytes::new(), 10, 10).is_err());
    assert!(RenderedPage::new(ImageType::Png, Bytes::from_static(b"\x89PNG"), 10, 10).is_err());
    assert!(RenderedPage::new(ImageType::Png, png(), 0, 10).is_err());
    assert!(RenderedPage::new(ImageType::Png, png(), 10, 0).is_err());
    assert!(RenderedPage::new(ImageType::Png, png(), MAX_IMAGE_EDGE + 1, 10).is_err());
    assert!(RenderedPage::new(ImageType::Png, png(), MAX_IMAGE_EDGE, MAX_IMAGE_EDGE).is_ok());
}

#[test]
fn the_debug_form_of_a_picture_shows_its_size_not_its_bytes() {
    let page = RenderedPage::new(ImageType::Png, Bytes::from_static(PNG), 4, 3).unwrap();
    let shown = format!("{page:?}");
    assert!(shown.contains("4") && shown.contains("3"));
    assert!(!shown.contains("PNG"));
}

#[test]
fn the_longest_side_of_a_picture_is_2576_pixels() {
    assert_eq!(MAX_IMAGE_EDGE, 2576);
    let png = || Bytes::from_static(PNG);
    assert!(RenderedPage::new(ImageType::Png, png(), 2576, 1).is_ok());
    assert!(RenderedPage::new(ImageType::Png, png(), 1, 2576).is_ok());
    assert!(RenderedPage::new(ImageType::Png, png(), 2577, 1).is_err());
    assert!(RenderedPage::new(ImageType::Png, png(), 1, 2577).is_err());
}

#[test]
fn a_word_that_does_not_belong_to_the_page_has_no_text() {
    let page = PageContent::from_parts(parts()).unwrap();
    assert_eq!(page.word_text(&word(0, 999)), "");
    assert_eq!(page.word_text(&word(3, 2)), "");
}
