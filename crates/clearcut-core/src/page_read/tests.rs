// Written by Kone Tshivhinda & Claude
use super::*;
use crate::text_quality::{FontEncoding, FontFact, FontKind, Reason, TextVerdict};
use crate::Error;

const GOOD: &str = "TAX INVOICE\nInvoice No: INV-2024-001\nDate: 2024-03-01\n\
                    Description Quantity Price Amount\nTotal R1,150.00 VAT R150.00";
const BROKEN: &str = "Invoice \u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}";
/// No invoice words and many words without a vowel: weak text.
const WEAK: &str = "xqz brtn plmk vnrt zdfg hjkl";

fn type3() -> FontFact {
    FontFact {
        kind: FontKind::Type3,
        has_to_unicode: false,
        encoding: FontEncoding::None,
        embedded: true,
    }
}

fn plain() -> FontFact {
    FontFact {
        kind: FontKind::Type1,
        has_to_unicode: false,
        encoding: FontEncoding::WinAnsi,
        embedded: true,
    }
}

fn read(text: &str, fonts: &[FontFact], ocr: Option<OcrRun>) -> PageRead {
    PageRead::decide(&PageEvidence {
        page: 1,
        text,
        fonts,
        ocr,
    })
}

fn ocr(agreement: Option<u16>) -> Option<OcrRun> {
    Some(OcrRun { agreement })
}

// ---------- names that are stored ----------

#[test]
fn the_route_names_are_exactly_the_agreed_list() {
    // ADR-0017. The names are stored in the database, so changing one is a breaking change.
    assert_eq!(
        ReadRoute::ALL,
        [
            ReadRoute::TextAndImage,
            ReadRoute::ImageFirst,
            ReadRoute::ImageOnly
        ]
    );
    assert_eq!(ReadRoute::TextAndImage.as_str(), "text_and_image");
    assert_eq!(ReadRoute::ImageFirst.as_str(), "image_first");
    assert_eq!(ReadRoute::ImageOnly.as_str(), "image_only");
}

#[test]
fn the_text_source_names_are_exactly_the_agreed_list() {
    assert_eq!(
        PageTextSource::ALL,
        [
            PageTextSource::TextLayer,
            PageTextSource::Ocr,
            PageTextSource::None
        ]
    );
    assert_eq!(PageTextSource::TextLayer.as_str(), "text_layer");
    assert_eq!(PageTextSource::Ocr.as_str(), "ocr");
    assert_eq!(PageTextSource::None.as_str(), "none");
}

#[test]
fn names_are_parsed_exactly_or_refused() {
    for route in ReadRoute::ALL {
        assert_eq!(ReadRoute::parse(route.as_str()), Ok(route));
    }
    for source in PageTextSource::ALL {
        assert_eq!(PageTextSource::parse(source.as_str()), Ok(source));
    }
    for bad in ["", "Image_Only", " image_only", "image_only ", "image", "*"] {
        assert_eq!(
            ReadRoute::parse(bad),
            Err(Error::InvalidPageRead("unknown reading route")),
            "{bad:?}"
        );
    }
    for bad in ["", "OCR", " ocr", "ocr ", "text", "*"] {
        assert_eq!(
            PageTextSource::parse(bad),
            Err(Error::InvalidPageRead("unknown text source")),
            "{bad:?}"
        );
    }
}

#[test]
fn the_rules_version_and_the_default_cap_are_named() {
    assert_eq!(ROUTING_VERSION, "routing-v1");
    assert_eq!(DEFAULT_OCR_PAGE_CAP, 50);
}

// ---------- the routing rule (ADR-0017 decision 2) ----------

#[test]
fn each_verdict_has_one_route() {
    assert_eq!(route_for(TextVerdict::Good), ReadRoute::TextAndImage);
    assert_eq!(route_for(TextVerdict::NeedsCheck), ReadRoute::ImageFirst);
    assert_eq!(route_for(TextVerdict::Unusable), ReadRoute::ImageOnly);
}

#[test]
fn a_good_text_layer_is_read_with_the_picture_and_needs_no_ocr_to_say_so() {
    let r = read(GOOD, &[plain()], None);
    assert_eq!(r.verdict(), TextVerdict::Good);
    assert_eq!(r.route(), ReadRoute::TextAndImage);
    assert_eq!(r.text_source(), PageTextSource::TextLayer);
    assert_eq!(r.reasons(), &[] as &[Reason]);
    assert_eq!(r.ocr_agreement(), None);
    assert!(!r.text_layer_rejected());
    assert_eq!(r.page(), 1);
    assert_eq!(r.analysed_with(), ROUTING_VERSION);
}

#[test]
fn an_ocr_that_agrees_settles_a_doubtful_page_as_good() {
    let r = read(WEAK, &[type3()], ocr(Some(900)));
    assert_eq!(r.verdict(), TextVerdict::Good);
    assert_eq!(r.route(), ReadRoute::TextAndImage);
    assert_eq!(r.reasons(), &[Reason::OcrAgrees]);
    assert_eq!(r.ocr_agreement(), Some(900));
    assert_eq!(r.text_source(), PageTextSource::TextLayer);
}

#[test]
fn a_doubtful_page_that_nobody_checked_goes_picture_first_with_the_text_as_a_hint() {
    let r = read(WEAK, &[type3()], None);
    assert_eq!(r.verdict(), TextVerdict::NeedsCheck);
    assert_eq!(r.route(), ReadRoute::ImageFirst);
    assert_eq!(r.text_source(), PageTextSource::TextLayer);
    assert!(!r.text_layer_rejected());
}

#[test]
fn an_ocr_that_partly_agrees_also_goes_picture_first() {
    let r = read(GOOD, &[plain()], ocr(Some(700)));
    assert_eq!(r.verdict(), TextVerdict::NeedsCheck);
    assert_eq!(r.route(), ReadRoute::ImageFirst);
    assert_eq!(r.reasons(), &[Reason::OcrPartlyAgrees]);
    assert_eq!(r.ocr_agreement(), Some(700));
}

#[test]
fn the_ocr_agreement_edges_are_500_and_850() {
    for (agreement, verdict) in [
        (499, TextVerdict::Unusable),
        (500, TextVerdict::NeedsCheck),
        (849, TextVerdict::NeedsCheck),
        (850, TextVerdict::Good),
    ] {
        assert_eq!(
            read(GOOD, &[plain()], ocr(Some(agreement))).verdict(),
            verdict,
            "{agreement}"
        );
    }
}

#[test]
fn a_broken_text_layer_is_rejected_and_the_picture_is_the_only_source() {
    let r = read(BROKEN, &[plain()], None);
    assert_eq!(r.verdict(), TextVerdict::Unusable);
    assert_eq!(r.route(), ReadRoute::ImageOnly);
    assert_eq!(r.reasons(), &[Reason::ReplacementCharacters]);
    assert!(r.text_layer_rejected());
    // OCR was not run, so there is no text at all.
    assert_eq!(r.text_source(), PageTextSource::None);
}

#[test]
fn a_rejected_layer_whose_page_was_read_by_ocr_takes_its_text_from_ocr() {
    let r = read(BROKEN, &[plain()], ocr(None));
    assert_eq!(r.text_source(), PageTextSource::Ocr);
    assert!(r.text_layer_rejected());
    assert_eq!(r.ocr_agreement(), None);
}

#[test]
fn an_ocr_that_disagrees_rejects_the_layer() {
    let r = read(GOOD, &[plain()], ocr(Some(400)));
    assert_eq!(r.verdict(), TextVerdict::Unusable);
    assert_eq!(r.route(), ReadRoute::ImageOnly);
    assert_eq!(r.reasons(), &[Reason::OcrDisagrees]);
    assert!(r.text_layer_rejected());
    assert_eq!(r.text_source(), PageTextSource::Ocr);
}

#[test]
fn a_page_with_no_text_layer_is_nothing_rejected() {
    // A scan or a picture: there was no layer to reject.
    let r = read("", &[], None);
    assert_eq!(r.verdict(), TextVerdict::Unusable);
    assert_eq!(r.reasons(), &[Reason::NoText]);
    assert_eq!(r.route(), ReadRoute::ImageOnly);
    assert!(!r.text_layer_rejected());
    assert_eq!(r.text_source(), PageTextSource::None);

    let r = read("   \n ", &[], ocr(None));
    assert_eq!(r.text_source(), PageTextSource::Ocr);
    assert!(!r.text_layer_rejected());
}

#[test]
fn rejected_means_unusable_for_some_reason_other_than_having_no_text() {
    let row = |reasons: &'static [&'static str]| StoredPageRead {
        page: 1,
        text_source: "ocr",
        verdict: "unusable",
        reasons,
        ocr_agreement: Some(100),
        route: "image_only",
        analysed_with: "routing-v1",
    };
    assert!(PageRead::from_stored(row(&["no_text", "ocr_disagrees"]))
        .unwrap()
        .text_layer_rejected());
    assert!(PageRead::from_stored(row(&["replacement_characters"]))
        .unwrap()
        .text_layer_rejected());
    assert!(!PageRead::from_stored(row(&["no_text"]))
        .unwrap()
        .text_layer_rejected());
}

#[test]
fn an_agreement_above_a_thousand_is_stored_as_a_thousand() {
    let r = read(GOOD, &[plain()], ocr(Some(5000)));
    assert_eq!(r.ocr_agreement(), Some(1000));
    assert_eq!(r.verdict(), TextVerdict::Good);
}

#[test]
fn the_page_number_is_kept() {
    let r = PageRead::decide(&PageEvidence {
        page: 17,
        text: GOOD,
        fonts: &[],
        ocr: None,
    });
    assert_eq!(r.page(), 17);
}

#[test]
fn no_part_of_the_text_is_in_the_record() {
    let r = read(BROKEN, &[plain()], ocr(Some(10)));
    let shown = format!("{r:?}");
    assert!(!shown.contains("Invoice"), "{shown}");
}

// ---------- reading it back from storage ----------

#[test]
fn what_is_decided_can_be_stored_and_read_back_unchanged() {
    let cases = [
        read(GOOD, &[plain()], None),
        read(GOOD, &[plain()], ocr(Some(900))),
        read(WEAK, &[type3()], None),
        read(GOOD, &[plain()], ocr(Some(700))),
        read(BROKEN, &[plain()], None),
        read(BROKEN, &[plain()], ocr(Some(10))),
        read(GOOD, &[plain()], ocr(Some(400))),
        read("", &[], None),
        read("", &[], ocr(None)),
    ];
    for r in &cases {
        let names: Vec<&str> = r.reasons().iter().map(|reason| reason.as_str()).collect();
        let back = PageRead::from_stored(StoredPageRead {
            page: r.page(),
            text_source: r.text_source().as_str(),
            verdict: r.verdict().as_str(),
            reasons: &names,
            ocr_agreement: r.ocr_agreement(),
            route: r.route().as_str(),
            analysed_with: r.analysed_with(),
        })
        .unwrap();
        assert_eq!(&back, r);
    }
}

fn good_row() -> StoredPageRead<'static> {
    StoredPageRead {
        page: 3,
        text_source: "text_layer",
        verdict: "good",
        reasons: &[],
        ocr_agreement: None,
        route: "text_and_image",
        analysed_with: "routing-v1",
    }
}

#[test]
fn a_row_that_disagrees_with_the_rule_is_refused() {
    assert!(PageRead::from_stored(good_row()).is_ok());
    let refused = |row: StoredPageRead<'_>, why: &'static str| {
        assert_eq!(PageRead::from_stored(row), Err(Error::InvalidPageRead(why)));
    };
    refused(
        StoredPageRead {
            page: 0,
            ..good_row()
        },
        "page numbers start at 1",
    );
    refused(
        StoredPageRead {
            route: "image_only",
            ..good_row()
        },
        "the route does not follow from the verdict",
    );
    refused(
        StoredPageRead {
            verdict: "unusable",
            ..good_row()
        },
        "the route does not follow from the verdict",
    );
    refused(
        StoredPageRead {
            text_source: "ocr",
            ..good_row()
        },
        "the text source does not follow from the verdict",
    );
    refused(
        StoredPageRead {
            verdict: "unusable",
            route: "image_only",
            text_source: "text_layer",
            ..good_row()
        },
        "the text source does not follow from the verdict",
    );
    refused(
        StoredPageRead {
            ocr_agreement: Some(1001),
            ..good_row()
        },
        "an agreement is at most 1000",
    );
    refused(
        StoredPageRead {
            analysed_with: "",
            ..good_row()
        },
        "the rules version is empty or too long",
    );
    refused(
        StoredPageRead {
            analysed_with: &"v".repeat(65),
            ..good_row()
        },
        "the rules version is empty or too long",
    );
    refused(
        StoredPageRead {
            reasons: &["ocr_agrees", "ocr_agrees"],
            ..good_row()
        },
        "a reason is listed twice",
    );
}

#[test]
fn the_longest_rules_version_is_64_characters() {
    let row = StoredPageRead {
        analysed_with: &"v".repeat(64),
        ..good_row()
    };
    assert!(PageRead::from_stored(row).is_ok());
}

#[test]
fn an_agreement_of_exactly_a_thousand_is_fine() {
    let row = StoredPageRead {
        ocr_agreement: Some(1000),
        ..good_row()
    };
    assert!(PageRead::from_stored(row).is_ok());
}

#[test]
fn unknown_names_in_a_row_are_refused() {
    assert_eq!(
        PageRead::from_stored(StoredPageRead {
            verdict: "great",
            ..good_row()
        }),
        Err(Error::InvalidPageRead("unknown verdict"))
    );
    assert_eq!(
        PageRead::from_stored(StoredPageRead {
            reasons: &["because"],
            ..good_row()
        }),
        Err(Error::InvalidPageRead("unknown reason"))
    );
    assert_eq!(
        PageRead::from_stored(StoredPageRead {
            route: "somehow",
            ..good_row()
        }),
        Err(Error::InvalidPageRead("unknown reading route"))
    );
    assert_eq!(
        PageRead::from_stored(StoredPageRead {
            text_source: "magic",
            ..good_row()
        }),
        Err(Error::InvalidPageRead("unknown text source"))
    );
}

// ---------- which pages OCR reads (ADR-0017 decision 3) ----------

use TextVerdict::{Good, NeedsCheck, Unusable};

#[test]
fn the_first_round_is_page_one_and_every_page_that_is_not_good() {
    assert_eq!(first_ocr_round(&[Good, Good, Good], 50), vec![1]);
    assert_eq!(
        first_ocr_round(&[Good, NeedsCheck, Good, Unusable], 50),
        vec![1, 2, 4]
    );
    assert_eq!(first_ocr_round(&[Unusable, Unusable], 50), vec![1, 2]);
    assert_eq!(first_ocr_round(&[NeedsCheck], 50), vec![1]);
}

#[test]
fn no_pages_no_ocr() {
    assert_eq!(first_ocr_round(&[], 50), Vec::<u32>::new());
    assert_eq!(second_ocr_round(&[], &[], Some(0), 50), Vec::<u32>::new());
}

#[test]
fn the_first_round_stops_at_the_cap_and_keeps_page_one() {
    let all_bad = vec![Unusable; 80];
    let round = first_ocr_round(&all_bad, 50);
    assert_eq!(round.len(), 50);
    assert_eq!(round[0], 1);
    assert_eq!(round[49], 50);
    assert_eq!(first_ocr_round(&all_bad, 1), vec![1]);
    assert_eq!(first_ocr_round(&all_bad, 0), Vec::<u32>::new());
}

#[test]
fn a_page_one_that_disagrees_sends_the_rest_of_the_document_to_ocr() {
    let verdicts = [NeedsCheck, Good, Good, Good];
    let first = first_ocr_round(&verdicts, 50);
    assert_eq!(first, vec![1]);
    assert_eq!(second_ocr_round(&verdicts, &first, Some(700), 50), vec![2, 3, 4]);
}

#[test]
fn the_second_round_skips_pages_already_read() {
    let verdicts = [Good, Unusable, Good, NeedsCheck, Good];
    let first = first_ocr_round(&verdicts, 50);
    assert_eq!(first, vec![1, 2, 4]);
    assert_eq!(second_ocr_round(&verdicts, &first, Some(0), 50), vec![3, 5]);
}

#[test]
fn a_page_one_that_agrees_or_was_not_compared_adds_nothing() {
    let verdicts = [Good, Good, Good];
    let first = first_ocr_round(&verdicts, 50);
    assert_eq!(
        second_ocr_round(&verdicts, &first, Some(850), 50),
        Vec::<u32>::new()
    );
    assert_eq!(
        second_ocr_round(&verdicts, &first, Some(1000), 50),
        Vec::<u32>::new()
    );
    // No agreement: page one has no text layer to compare with, so there is no sentinel result.
    assert_eq!(second_ocr_round(&verdicts, &first, None, 50), Vec::<u32>::new());
    assert_eq!(second_ocr_round(&verdicts, &first, Some(849), 50), vec![2, 3]);
}

#[test]
fn the_second_round_stops_where_the_cap_is_reached() {
    let verdicts = vec![Good; 120];
    let first = first_ocr_round(&verdicts, 50);
    let second = second_ocr_round(&verdicts, &first, Some(0), 50);
    assert_eq!(second.len(), 49);
    assert_eq!(second[0], 2);
    assert_eq!(second[48], 50);
    // The two rounds together never go past the cap.
    let second = second_ocr_round(&verdicts, &first, Some(0), 10);
    assert_eq!(second, (2..=10).collect::<Vec<u32>>());
    // A first round that already used the cap leaves no room.
    let bad = vec![Unusable; 60];
    let first = first_ocr_round(&bad, 50);
    assert_eq!(second_ocr_round(&bad, &first, Some(0), 50), Vec::<u32>::new());
}
