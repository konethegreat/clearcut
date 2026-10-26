// Written by Kone Tshivhinda & Claude
//! A person's confirmation of a page that went to review.
//!
//! The person sees what the model read (or a blank form), changes what is wrong and confirms.
//! What they send is a [`PageReading`], the very same shape a model returns, and it goes
//! through the **same** conversion and the **same** checks. A person can fill in what the model
//! could not read (a tax category, say), but cannot get a page past the arithmetic: lines that
//! do not add up, an amount that is not an exact decimal, a missing currency are refused with
//! the fixed names that say where. Nothing is defaulted or repaired for them either.
//!
//! The result is recorded as [`Extractor::Human`] with a confidence of
//! [`ConfidenceBasis::PersonConfirmed`], never as the model's.

use crate::escalation::{assess, ConfidenceBasis, ReadingEvidence, TextCheck, Verdict};
use crate::event::{Confidence, DocumentId, Extractor};
use crate::extraction::{validate, Extraction, Finding};
use crate::reading::{ConversionContext, PageReading, ReadingProblem};
use serde::Serialize;
use std::num::NonZeroU32;

/// Why a page cannot be confirmed as it stands. Fixed names and places only, never text from
/// the document, so it can be shown and logged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Refusal {
    /// No currency is given: a page needs one.
    pub currency_missing: bool,
    /// What could not be read exactly, and where.
    pub problems: Vec<ReadingProblem>,
    /// The checks that failed (these carry amounts).
    pub findings: Vec<Finding>,
}

/// Checks a person's reading of one page and turns it into an [`Extraction`] made by `user`.
///
/// `whole_document` says the page is the whole document, so the printed totals can be compared
/// with its lines (on page 3 of 5 the totals are elsewhere and a mismatch means nothing yet).
/// The checks of the document as a whole run later, when its pages are put together.
pub fn check_confirmed(
    reading: &PageReading,
    document: DocumentId,
    page: NonZeroU32,
    user: &str,
    whole_document: bool,
) -> Result<Extraction, Refusal> {
    let context = ConversionContext {
        document,
        page,
        extractor: Extractor::Human {
            user: user.to_string(),
        },
        confidence: Confidence::new(ConfidenceBasis::PersonConfirmed.per_mille())
            .unwrap_or(Confidence::CERTAIN),
        currency_known: None,
    };
    let conversion = reading.into_extraction(&context).map_err(|_| Refusal {
        currency_missing: true,
        problems: Vec::new(),
        findings: Vec::new(),
    })?;
    let validation = validate(&conversion.extraction);
    let verdict = assess(&ReadingEvidence {
        problems: &conversion.problems,
        findings: &validation.findings,
        text_check: TextCheck::NotAvailable,
        whole_document,
    })
    .verdict;
    if verdict == Verdict::Accept {
        return Ok(conversion.extraction);
    }
    Err(Refusal {
        currency_missing: false,
        problems: conversion.problems,
        // A page with no lines may simply be a terms page: that is judged for the whole document.
        findings: validation
            .findings
            .into_iter()
            .filter(|finding| finding.is_blocking() && *finding != Finding::NoLines)
            .collect(),
    })
}

#[cfg(test)]
mod tests;
