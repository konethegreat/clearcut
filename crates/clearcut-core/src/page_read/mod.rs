// Written by Kone Tshivhinda & Claude
//! How a page will be read (ADR-0017, the first stage of a document's processing).
//!
//! The worker reports facts about each page (its text layer, its fonts, and, when OCR was
//! run, how much of the text layer OCR also read). [`crate::text_quality`] turns those into
//! a verdict on the text layer. This module turns the verdict into a **reading route**, the
//! one decision every later stage needs, and keeps the facts behind it in a [`PageRead`]
//! that is stored (one row per page) and holds no text:
//!
//! | Text layer | Route | Meaning for later stages |
//! |---|---|---|
//! | `Good` | `text_and_image` | the model gets the page picture and the text layer |
//! | `NeedsCheck` | `image_first` | the picture is the source; the text is only a labelled hint |
//! | `Unusable` (including no text at all) | `image_only` | the model reads the picture; OCR text, if it was run, is the hint |
//!
//! "Never trust one path" (P2-20): a page whose text and picture may disagree is never read
//! from the text alone. A text layer that was *rejected* (it exists but cannot be trusted) is
//! different from a page that never had one, and the record says which.
//!
//! The second half is the choice of **which pages OCR reads**, because OCR costs about a
//! second a page: page 1 always (a sentinel for the whole document), every page whose text
//! layer is not already `Good`, and, if page 1 disagrees with OCR, the rest of the
//! document, up to a cap. All of this is pure: the job code only fetches, calls and stores.

#[cfg(test)]
mod tests;

use crate::error::{Error, Result};
use crate::text_quality::{assess, FontFact, Reason, TextEvidence, TextVerdict, OCR_GOOD_FROM};
use serde::Serialize;

/// The version of the rules in this module, stored with every row so a later change to a
/// threshold or to the table above can be told apart from an earlier one.
pub const ROUTING_VERSION: &str = "routing-v1";

/// The most pages of one document that OCR reads, unless a setting says otherwise.
pub const DEFAULT_OCR_PAGE_CAP: u32 = 50;

/// The longest rules version a row may carry.
const MAX_VERSION_LEN: usize = 64;

// ------------------------------------------------------------------------------ names ----

/// How the model is to read a page. The names are stored in the database.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadRoute {
    /// The picture and the text layer.
    TextAndImage,
    /// The picture is the source; the text layer is only a hint.
    ImageFirst,
    /// The picture alone (and OCR text as a hint, if OCR was run).
    ImageOnly,
}

impl ReadRoute {
    pub const ALL: [ReadRoute; 3] = [
        ReadRoute::TextAndImage,
        ReadRoute::ImageFirst,
        ReadRoute::ImageOnly,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ReadRoute::TextAndImage => "text_and_image",
            ReadRoute::ImageFirst => "image_first",
            ReadRoute::ImageOnly => "image_only",
        }
    }

    /// Parses exactly one name: no trimming, no case folding.
    pub fn parse(text: &str) -> Result<ReadRoute> {
        ReadRoute::ALL
            .into_iter()
            .find(|route| route.as_str() == text)
            .ok_or(Error::InvalidPageRead("unknown reading route"))
    }
}

/// Where the text that goes with a page comes from. The names are stored in the database.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PageTextSource {
    /// The PDF's own text layer (trusted, or only a hint).
    TextLayer,
    /// OCR read the picture of the page.
    Ocr,
    /// There is no text for this page.
    None,
}

impl PageTextSource {
    pub const ALL: [PageTextSource; 3] = [
        PageTextSource::TextLayer,
        PageTextSource::Ocr,
        PageTextSource::None,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            PageTextSource::TextLayer => "text_layer",
            PageTextSource::Ocr => "ocr",
            PageTextSource::None => "none",
        }
    }

    /// Parses exactly one name: no trimming, no case folding.
    pub fn parse(text: &str) -> Result<PageTextSource> {
        PageTextSource::ALL
            .into_iter()
            .find(|source| source.as_str() == text)
            .ok_or(Error::InvalidPageRead("unknown text source"))
    }
}

/// The route that follows from the verdict on a page's text layer.
pub fn route_for(verdict: TextVerdict) -> ReadRoute {
    match verdict {
        TextVerdict::Good => ReadRoute::TextAndImage,
        TextVerdict::NeedsCheck => ReadRoute::ImageFirst,
        TextVerdict::Unusable => ReadRoute::ImageOnly,
    }
}

/// Where the page's text comes from, given the verdict and whether OCR was run. A usable
/// or doubtful text layer is still the text layer; a rejected one (or none) leaves OCR, or
/// nothing if OCR was not run.
fn text_source_for(verdict: TextVerdict, ocr_ran: bool) -> PageTextSource {
    match verdict {
        TextVerdict::Good | TextVerdict::NeedsCheck => PageTextSource::TextLayer,
        TextVerdict::Unusable if ocr_ran => PageTextSource::Ocr,
        TextVerdict::Unusable => PageTextSource::None,
    }
}

// ------------------------------------------------------------------------- the record ----

/// OCR was run on the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OcrRun {
    /// Per mille of the text layer's words that OCR also read; `None` when the page has no
    /// text layer to compare with.
    pub agreement: Option<u16>,
}

/// Everything the decision is made from, for one page.
#[derive(Debug, Clone, Copy)]
pub struct PageEvidence<'a> {
    /// The page, counting from 1.
    pub page: u32,
    /// The page's text layer (empty for a scan or a picture).
    pub text: &'a str,
    /// The fonts the page can draw with, from the worker.
    pub fonts: &'a [FontFact],
    /// What OCR found, if OCR was run on this page.
    pub ocr: Option<OcrRun>,
}

/// How one page will be read, and why. Holds facts about the page and none of its text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PageRead {
    page: u32,
    text_source: PageTextSource,
    verdict: TextVerdict,
    reasons: Vec<Reason>,
    ocr_agreement: Option<u16>,
    route: ReadRoute,
    analysed_with: String,
}

impl PageRead {
    /// Decides how to read a page from the evidence about it.
    pub fn decide(evidence: &PageEvidence<'_>) -> PageRead {
        let agreement = evidence.ocr.and_then(|run| run.agreement).map(|a| a.min(1000));
        let assessment = assess(&TextEvidence {
            text: evidence.text,
            fonts: evidence.fonts,
            ocr_agreement: agreement,
        });
        PageRead {
            page: evidence.page,
            text_source: text_source_for(assessment.verdict, evidence.ocr.is_some()),
            verdict: assessment.verdict,
            reasons: assessment.reasons,
            ocr_agreement: agreement,
            route: route_for(assessment.verdict),
            analysed_with: ROUTING_VERSION.to_string(),
        }
    }

    /// Reads a stored row back, refusing one that does not follow from the rules: a row
    /// the store hands over is checked like any other input.
    pub fn from_stored(row: StoredPageRead<'_>) -> Result<PageRead> {
        let verdict = TextVerdict::parse(row.verdict).ok_or(Error::InvalidPageRead("unknown verdict"))?;
        let text_source = PageTextSource::parse(row.text_source)?;
        let route = ReadRoute::parse(row.route)?;
        let reasons = row
            .reasons
            .iter()
            .map(|name| Reason::parse(name).ok_or(Error::InvalidPageRead("unknown reason")))
            .collect::<Result<Vec<_>>>()?;
        if row.page == 0 {
            return Err(Error::InvalidPageRead("page numbers start at 1"));
        }
        if route != route_for(verdict) {
            return Err(Error::InvalidPageRead(
                "the route does not follow from the verdict",
            ));
        }
        let source_fits = match verdict {
            TextVerdict::Good | TextVerdict::NeedsCheck => text_source == PageTextSource::TextLayer,
            TextVerdict::Unusable => text_source != PageTextSource::TextLayer,
        };
        if !source_fits {
            return Err(Error::InvalidPageRead(
                "the text source does not follow from the verdict",
            ));
        }
        if row.ocr_agreement.is_some_and(|a| a > 1000) {
            return Err(Error::InvalidPageRead("an agreement is at most 1000"));
        }
        if row.analysed_with.is_empty() || row.analysed_with.len() > MAX_VERSION_LEN {
            return Err(Error::InvalidPageRead("the rules version is empty or too long"));
        }
        if reasons.iter().enumerate().any(|(i, r)| reasons[..i].contains(r)) {
            return Err(Error::InvalidPageRead("a reason is listed twice"));
        }
        Ok(PageRead {
            page: row.page,
            text_source,
            verdict,
            reasons,
            ocr_agreement: row.ocr_agreement,
            route,
            analysed_with: row.analysed_with.to_string(),
        })
    }

    pub fn page(&self) -> u32 {
        self.page
    }

    pub fn text_source(&self) -> PageTextSource {
        self.text_source
    }

    pub fn verdict(&self) -> TextVerdict {
        self.verdict
    }

    pub fn reasons(&self) -> &[Reason] {
        &self.reasons
    }

    pub fn ocr_agreement(&self) -> Option<u16> {
        self.ocr_agreement
    }

    pub fn route(&self) -> ReadRoute {
        self.route
    }

    pub fn analysed_with(&self) -> &str {
        &self.analysed_with
    }

    /// The page had a text layer and it was thrown out (replacement characters, or OCR does
    /// not agree with it): recorded as `text_layer_rejected`. A page that never had a text
    /// layer (a scan, a picture) has nothing to reject.
    pub fn text_layer_rejected(&self) -> bool {
        self.verdict == TextVerdict::Unusable && self.reasons.iter().any(|reason| *reason != Reason::NoText)
    }
}

/// One stored row, as plain values, for [`PageRead::from_stored`].
#[derive(Debug, Clone, Copy)]
pub struct StoredPageRead<'a> {
    pub page: u32,
    pub text_source: &'a str,
    pub verdict: &'a str,
    pub reasons: &'a [&'a str],
    pub ocr_agreement: Option<u16>,
    pub route: &'a str,
    pub analysed_with: &'a str,
}

// ---------------------------------------------------------------------- which pages OCR ----

/// The pages OCR reads first: page 1 (always, as the sentinel) and every page whose text
/// layer is not already `Good`, in page order, at most `cap` of them. `verdicts[i]` is the
/// verdict on page `i + 1` from the text alone (a page with no text is `Unusable`).
pub fn first_ocr_round(verdicts: &[TextVerdict], cap: u32) -> Vec<u32> {
    (1u32..)
        .zip(verdicts)
        .filter(|(page, verdict)| *page == 1 || **verdict != TextVerdict::Good)
        .map(|(page, _)| page)
        .take(usize::try_from(cap).unwrap_or(usize::MAX))
        .collect()
}

/// The pages OCR reads next. If page 1's text layer disagrees with OCR (an agreement below
/// the `Good` threshold), the producer of this document cannot be taken at its word, so the
/// pages not yet read are read too, in page order, until `cap` pages in all have been read.
/// If page 1 agrees, or has no text layer to compare with, there is nothing more to do.
pub fn second_ocr_round(
    verdicts: &[TextVerdict],
    first_round: &[u32],
    page_one_agreement: Option<u16>,
    cap: u32,
) -> Vec<u32> {
    if !page_one_agreement.is_some_and(|agreement| agreement < OCR_GOOD_FROM) {
        return Vec::new();
    }
    let room = usize::try_from(cap)
        .unwrap_or(usize::MAX)
        .saturating_sub(first_round.len());
    (1u32..)
        .zip(verdicts)
        .map(|(page, _)| page)
        .filter(|page| !first_round.contains(page))
        .take(room)
        .collect()
}
