// Written by Kone Tshivhinda & Claude
//! One period register, built from stored events only (export readiness, P4-08).
//!
//! Nothing here is stored or changes what a document says. Only current events count; an
//! undated document is never put in a month; amounts are added only within one currency;
//! credit notes stay apart from invoices; only `done` documents feed the ready figures and
//! `needs_review` documents feed the awaiting ones.
//!
//! A document's date is the invoice date on the most recent current event that carries one.
//! A document whose amounts disagree on currency gets a blank currency, a `mixed_currency`
//! flag, and is left out of cross-document sums.

use crate::document::DocumentStatus;
use crate::escalation::ReviewReason;
use crate::event::{Event, EventPayload};
use crate::export::Amount;
use crate::money::{Currency, Money};
use crate::tax::TaxCategory;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use time::{Date, Month, OffsetDateTime};

/// What every register file says at its foot.
pub const REGISTER_NOTICE: &str =
    "Made by ClearCut from what was read off your documents and checked by its own rules. \
     Figures that matter must be checked against the original. This register is not a tax return \
     or tax advice.";

/// The most documents one register may hold.
pub const MAX_DOCUMENTS: usize = 500;

/// The most line items one register may hold.
pub const MAX_LINES: usize = 50_000;

/// Which documents a register is made from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Selection {
    /// Every document whose invoice date falls from `from` to `to`, both included.
    Period { from: Date, to: Date },
    /// Exactly these documents, whatever their date.
    Documents { ids: Vec<String> },
}

impl Selection {
    /// How the selection reads in a heading.
    pub fn label(&self) -> String {
        match self {
            Selection::Period { from, to } => format!("{from} to {to}"),
            Selection::Documents { ids } => format!(
                "{} chosen document{}",
                ids.len(),
                if ids.len() == 1 { "" } else { "s" }
            ),
        }
    }
}

/// What a document is in the register.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A supplier invoice.
    Invoice,
    /// A credit note, kept apart because it reverses an invoice.
    CreditNote,
    /// Nothing said whether it is an invoice or a credit note.
    Unclassified,
}

impl Kind {
    /// The stored and API name.
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Invoice => "invoice",
            Kind::CreditNote => "credit_note",
            Kind::Unclassified => "unclassified",
        }
    }

    /// How it reads in a table.
    pub fn label(self) -> &'static str {
        match self {
            Kind::Invoice => "Invoice",
            Kind::CreditNote => "Credit note",
            Kind::Unclassified => "Unclassified",
        }
    }
}

/// A page that stopped for a person, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PageReview {
    pub page: u32,
    pub reason: ReviewReason,
}

/// What the store hands in for one document.
#[derive(Debug, Clone)]
pub struct RegisterFacts {
    pub id: String,
    pub filename: String,
    pub uploaded_at: OffsetDateTime,
    pub status: DocumentStatus,
    pub page_count: Option<u32>,
    pub pages: Vec<PageReview>,
    pub events: Vec<Event>,
}

/// One document's row in the register.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RegisterDocument {
    pub id: String,
    pub filename: String,
    pub uploaded_at: OffsetDateTime,
    pub status: DocumentStatus,
    pub kind: Kind,
    pub supplier: Option<String>,
    pub supplier_vat_number: Option<String>,
    pub invoice_number: Option<String>,
    pub invoice_date: Option<Date>,
    pub due_date: Option<Date>,
    pub terms: Option<String>,
    pub currency: Option<String>,
    pub net: Option<Amount>,
    pub vat: Option<Amount>,
    pub gross: Option<Amount>,
    pub line_count: usize,
    pub page_count: Option<u32>,
    pub review_reasons: Vec<String>,
    pub duplicate_of: Option<String>,
    pub read_by: Vec<String>,
    pub flags: Vec<String>,
}

/// One line item in the register.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RegisterLine {
    pub document_id: String,
    pub invoice_number: Option<String>,
    pub supplier: Option<String>,
    pub invoice_date: Option<Date>,
    pub number: u32,
    pub description: String,
    pub quantity_milli: i64,
    pub quantity: String,
    pub unit_price: Amount,
    pub line_total: Amount,
    pub tax: TaxCategory,
    pub page: u32,
}

/// One side of a VAT group: ready documents or documents awaiting review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VatPart {
    pub documents: usize,
    pub taxable: Amount,
    pub vat: Amount,
}

/// VAT for one month, currency, kind, category and rate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VatGroup {
    pub month: String,
    pub currency: String,
    pub kind: Kind,
    pub category: TaxCategory,
    pub rate_bps: u32,
    pub ready: Option<VatPart>,
    pub awaiting: Option<VatPart>,
}

/// One side of a stated-totals group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StatedPart {
    pub documents: usize,
    pub net: Amount,
    pub vat: Amount,
    pub gross: Amount,
}

/// The totals printed on the documents, by month, currency and kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StatedTotals {
    pub month: String,
    pub currency: String,
    pub kind: Kind,
    pub ready: Option<StatedPart>,
    pub awaiting: Option<StatedPart>,
}

/// The VAT ClearCut calculated against the VAT the documents state. One row covers one month,
/// currency and kind, merging the ready and the awaiting documents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Reconciliation {
    pub month: String,
    pub currency: String,
    pub kind: Kind,
    pub calculated_vat: Amount,
    pub stated_vat: Amount,
    pub difference: Amount,
    pub documents_disagree: usize,
}

/// Something a person still has to check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToCheckRow {
    pub document_id: String,
    pub filename: String,
    pub reason: String,
    pub sentence: String,
    pub page: Option<u32>,
}

/// What the register contains and what it left out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Manifest {
    pub selection: Selection,
    pub only_ready: bool,
    pub documents: usize,
    pub done: usize,
    pub needs_review: usize,
    pub still_reading: usize,
    pub undated_in_tenant: usize,
    pub undated_in_selection: usize,
    pub lines: usize,
    pub automation_rate_bps: Option<u32>,
    pub finished: usize,
    pub awaiting_left_out: usize,
    pub software_version: String,
    pub notice: &'static str,
}

/// The whole register.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Register {
    pub notice: &'static str,
    pub version: String,
    pub selection: Selection,
    pub only_ready: bool,
    pub rows: Vec<RegisterDocument>,
    pub lines: Vec<RegisterLine>,
    pub vat_by_category: Vec<VatGroup>,
    pub stated_totals: Vec<StatedTotals>,
    pub reconciliation: Vec<Reconciliation>,
    pub to_check: Vec<ToCheckRow>,
    pub manifest: Manifest,
}

/// Why a register could not be built.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RegisterError {
    #[error("the selection names more than 500 documents")]
    TooManyDocuments,
    #[error("the selection holds more than 50 000 line items")]
    TooManyLines,
    #[error("the amounts are too large to add up")]
    Overflow,
}

/// The reading of one document the register works from.
struct Reading {
    document: RegisterDocument,
    current: Vec<Event>,
    pages: Vec<PageReview>,
}

/// Reads a `YYYY-MM-DD` date exactly, or `None`.
pub fn parse_date(text: &str) -> Option<Date> {
    let bytes = text.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    fn number(part: &[u8]) -> Option<u32> {
        part.iter().try_fold(0u32, |n, byte| {
            byte.is_ascii_digit().then(|| n * 10 + u32::from(byte - b'0'))
        })
    }
    let year = i32::try_from(number(&bytes[0..4])?).ok()?;
    let month = Month::try_from(u8::try_from(number(&bytes[5..7])?).ok()?).ok()?;
    let day = u8::try_from(number(&bytes[8..10])?).ok()?;
    Date::from_calendar_date(year, month, day).ok()
}

fn same_currency(amounts: &[crate::Money]) -> Option<String> {
    let first = amounts.first()?;
    amounts
        .iter()
        .all(|amount| amount.currency == first.currency)
        .then(|| first.currency.code().to_string())
}

fn read(facts: &RegisterFacts) -> Reading {
    let current: Vec<Event> = crate::event::view::current(&facts.events)
        .into_iter()
        .cloned()
        .collect();
    let invoice_date = current.iter().rev().find_map(Event::date);
    let supplier = current.iter().rev().find_map(|event| match event.payload() {
        EventPayload::SupplierIdentified { name, vat_number } => Some((name.clone(), vat_number.clone())),
        _ => None,
    });
    let invoice_number = current.iter().rev().find_map(|event| match event.payload() {
        EventPayload::InvoiceNumberStated { number } => Some(number.clone()),
        _ => None,
    });
    let totals = current.iter().rev().find_map(|event| match event.payload() {
        EventPayload::TotalsStated { net, vat, gross } => Some((*net, *vat, *gross)),
        _ => None,
    });
    let due = current.iter().rev().find_map(|event| match event.payload() {
        EventPayload::DueDateStated { due, terms } => Some((*due, terms.clone())),
        _ => None,
    });
    let duplicate_of = current.iter().rev().find_map(|event| match event.payload() {
        EventPayload::PossibleDuplicate {
            matching_document, ..
        } => Some(matching_document.as_str().to_string()),
        _ => None,
    });
    let credit_note = current
        .iter()
        .any(|event| matches!(event.payload(), EventPayload::CreditNoteReferenced { .. }));
    let kind = if credit_note {
        Kind::CreditNote
    } else if invoice_number.is_some() || totals.is_some() {
        Kind::Invoice
    } else {
        Kind::Unclassified
    };

    let amounts: Vec<crate::Money> = current
        .iter()
        .flat_map(|event| event.payload().money_fields())
        .collect();
    let currency = same_currency(&amounts);
    let mut flags = Vec::new();
    if !amounts.is_empty() && currency.is_none() {
        flags.push("mixed_currency".to_string());
    }

    let (supplier, supplier_vat_number) = match supplier {
        Some((name, vat)) => (Some(name), vat),
        None => (None, None),
    };
    let (due_date, terms) = match due {
        Some((due, terms)) => (Some(due), terms),
        None => (None, None),
    };
    let (net, vat, gross) = match totals {
        Some((net, vat, gross)) => (
            Some(crate::export::amount(net)),
            Some(crate::export::amount(vat)),
            Some(crate::export::amount(gross)),
        ),
        None => (None, None, None),
    };
    let mut review_reasons = Vec::new();
    for page in &facts.pages {
        let name = page.reason.as_str().to_string();
        if !review_reasons.contains(&name) {
            review_reasons.push(name);
        }
    }
    let mut read_by: Vec<String> = Vec::new();
    for event in &current {
        let name = crate::export::who(event.extractor());
        if !read_by.contains(&name) {
            read_by.push(name);
        }
    }
    let line_count = current
        .iter()
        .filter(|event| matches!(event.payload(), EventPayload::LineItemBilled { .. }))
        .count();

    Reading {
        document: RegisterDocument {
            id: facts.id.clone(),
            filename: facts.filename.clone(),
            uploaded_at: facts.uploaded_at,
            status: facts.status,
            kind,
            supplier,
            supplier_vat_number,
            invoice_number,
            invoice_date,
            due_date,
            terms,
            currency,
            net,
            vat,
            gross,
            line_count,
            page_count: facts.page_count,
            review_reasons,
            duplicate_of,
            read_by,
            flags,
        },
        current,
        pages: facts.pages.clone(),
    }
}

// ---------------------------------------------------------------- line rows ----

/// The line items of every finished document, in reading order, numbered from one per document.
/// A document still being read has no line rows, because its extraction is only partial.
fn lines(readings: &[Reading]) -> Vec<RegisterLine> {
    let mut out = Vec::new();
    for reading in readings {
        let document = &reading.document;
        if side(document.status).is_none() {
            continue;
        }
        let mut number = 0u32;
        for event in &reading.current {
            let EventPayload::LineItemBilled {
                description,
                quantity_milli,
                unit_price,
                line_total,
                tax_category,
            } = event.payload()
            else {
                continue;
            };
            number += 1;
            out.push(RegisterLine {
                document_id: document.id.clone(),
                invoice_number: document.invoice_number.clone(),
                supplier: document.supplier.clone(),
                invoice_date: document.invoice_date,
                number,
                description: description.clone(),
                quantity_milli: *quantity_milli,
                quantity: crate::narration::format::quantity_milli(*quantity_milli),
                unit_price: crate::export::amount(*unit_price),
                line_total: crate::export::amount(*line_total),
                tax: *tax_category,
                page: event.citation().page.get(),
            });
        }
    }
    out
}

// --------------------------------------------------------------- the sums ----

/// Which side of a group a document is on: `true` for ready figures, `false` for awaiting ones.
fn side(status: DocumentStatus) -> Option<bool> {
    match status {
        DocumentStatus::Done => Some(true),
        DocumentStatus::NeedsReview => Some(false),
        _ => None,
    }
}

/// `YYYY-MM` of the invoice date, which is the month every sum is grouped by.
fn month_of(date: Date) -> String {
    format!("{:04}-{:02}", date.year(), u8::from(date.month()))
}

/// The month a document may be summed into, or `None` when it has no date or no single currency.
fn month_for_sums(document: &RegisterDocument) -> Option<String> {
    document.currency.as_ref()?;
    Some(month_of(document.invoice_date?))
}

/// Adds a part amount to a running total, refusing to overflow.
fn added(total: &mut i64, amount: Money) -> Result<(), RegisterError> {
    *total = total.checked_add(amount.minor).ok_or(RegisterError::Overflow)?;
    Ok(())
}

/// The last `TotalsStated` a document still says.
fn stated_of(reading: &Reading) -> Option<(Money, Money, Money)> {
    reading
        .current
        .iter()
        .rev()
        .find_map(|event| match event.payload() {
            EventPayload::TotalsStated { net, vat, gross } => Some((*net, *vat, *gross)),
            _ => None,
        })
}

#[derive(Default)]
struct VatPartAcc {
    documents: usize,
    taxable: i64,
    vat: i64,
}

impl VatPartAcc {
    fn part(self, currency: Currency) -> VatPart {
        VatPart {
            documents: self.documents,
            taxable: crate::export::amount(Money::new(self.taxable, currency)),
            vat: crate::export::amount(Money::new(self.vat, currency)),
        }
    }
}

struct VatAcc {
    month: String,
    currency: Currency,
    kind: Kind,
    category: TaxCategory,
    rate_bps: u32,
    ready: Option<VatPartAcc>,
    awaiting: Option<VatPartAcc>,
}

/// VAT ClearCut calculated, added within one month, currency, kind, category and rate.
///
/// The currency comes from the VAT event itself, and the document is skipped when it has no
/// single currency, so two currencies are never added together.
fn vat_by_category(readings: &[Reading]) -> Result<Vec<VatGroup>, RegisterError> {
    let mut groups: BTreeMap<(String, &'static str, Kind, TaxCategory, u32), VatAcc> = BTreeMap::new();
    for reading in readings {
        let document = &reading.document;
        let Some(ready) = side(document.status) else {
            continue;
        };
        let Some(month) = month_for_sums(document) else {
            continue;
        };
        let mut own: BTreeMap<(TaxCategory, u32), (Currency, i64, i64)> = BTreeMap::new();
        for event in &reading.current {
            let EventPayload::VatCalculated {
                category,
                taxable,
                rate_bps,
                vat,
            } = event.payload()
            else {
                continue;
            };
            if taxable.currency != vat.currency {
                continue;
            }
            let entry = own
                .entry((*category, *rate_bps))
                .or_insert((taxable.currency, 0, 0));
            added(&mut entry.1, *taxable)?;
            added(&mut entry.2, *vat)?;
        }
        for ((category, rate_bps), (currency, taxable, vat)) in own {
            let key = (month.clone(), currency.code(), document.kind, category, rate_bps);
            let acc = groups.entry(key).or_insert_with(|| VatAcc {
                month: month.clone(),
                currency,
                kind: document.kind,
                category,
                rate_bps,
                ready: None,
                awaiting: None,
            });
            let part = if ready { &mut acc.ready } else { &mut acc.awaiting };
            let part = part.get_or_insert_with(VatPartAcc::default);
            part.documents += 1;
            added(&mut part.taxable, Money::new(taxable, currency))?;
            added(&mut part.vat, Money::new(vat, currency))?;
        }
    }
    let mut out = Vec::new();
    for acc in groups.into_values() {
        let VatAcc {
            month,
            currency,
            kind,
            category,
            rate_bps,
            ready,
            awaiting,
        } = acc;
        out.push(VatGroup {
            month,
            currency: currency.code().to_string(),
            kind,
            category,
            rate_bps,
            ready: ready.map(|part| part.part(currency)),
            awaiting: awaiting.map(|part| part.part(currency)),
        });
    }
    Ok(out)
}

struct StatedPartAcc {
    documents: usize,
    net: i64,
    vat: i64,
    gross: i64,
}

impl StatedPartAcc {
    fn part(self, currency: Currency) -> StatedPart {
        StatedPart {
            documents: self.documents,
            net: crate::export::amount(Money::new(self.net, currency)),
            vat: crate::export::amount(Money::new(self.vat, currency)),
            gross: crate::export::amount(Money::new(self.gross, currency)),
        }
    }
}

struct StatedAcc {
    month: String,
    currency: Currency,
    kind: Kind,
    ready: Option<StatedPartAcc>,
    awaiting: Option<StatedPartAcc>,
}

/// The totals printed on the documents, added within one month, currency and kind.
fn stated_totals(readings: &[Reading]) -> Result<Vec<StatedTotals>, RegisterError> {
    let mut groups: BTreeMap<(String, &'static str, Kind), StatedAcc> = BTreeMap::new();
    for reading in readings {
        let document = &reading.document;
        let Some(ready) = side(document.status) else {
            continue;
        };
        let Some(month) = month_for_sums(document) else {
            continue;
        };
        let Some((net, vat, gross)) = stated_of(reading) else {
            continue;
        };
        if net.currency != vat.currency || vat.currency != gross.currency {
            continue;
        }
        let key = (month.clone(), net.currency.code(), document.kind);
        let acc = groups.entry(key).or_insert_with(|| StatedAcc {
            month: month.clone(),
            currency: net.currency,
            kind: document.kind,
            ready: None,
            awaiting: None,
        });
        let part = if ready { &mut acc.ready } else { &mut acc.awaiting };
        let part = part.get_or_insert_with(|| StatedPartAcc {
            documents: 0,
            net: 0,
            vat: 0,
            gross: 0,
        });
        part.documents += 1;
        added(&mut part.net, net)?;
        added(&mut part.vat, vat)?;
        added(&mut part.gross, gross)?;
    }
    let mut out = Vec::new();
    for acc in groups.into_values() {
        let StatedAcc {
            month,
            currency,
            kind,
            ready,
            awaiting,
        } = acc;
        out.push(StatedTotals {
            month,
            currency: currency.code().to_string(),
            kind,
            ready: ready.map(|part| part.part(currency)),
            awaiting: awaiting.map(|part| part.part(currency)),
        });
    }
    Ok(out)
}

struct ReconAcc {
    month: String,
    currency: Currency,
    kind: Kind,
    calculated: i64,
    stated: i64,
    disagree: usize,
}

/// The VAT ClearCut calculated against the VAT the document states, per month, currency and kind.
///
/// Only a document that states VAT and also has a calculated VAT is reconciled. A document with
/// only one of the two is left out rather than counted as agreeing.
fn reconciliation(readings: &[Reading]) -> Result<Vec<Reconciliation>, RegisterError> {
    let mut groups: BTreeMap<(String, &'static str, Kind), ReconAcc> = BTreeMap::new();
    for reading in readings {
        let document = &reading.document;
        if side(document.status).is_none() {
            continue;
        }
        let Some(month) = month_for_sums(document) else {
            continue;
        };
        let Some((_, stated, _)) = stated_of(reading) else {
            continue;
        };
        let mut calculated: Option<Money> = None;
        for event in &reading.current {
            let EventPayload::VatCalculated { taxable, vat, .. } = event.payload() else {
                continue;
            };
            if taxable.currency != vat.currency {
                continue;
            }
            calculated = Some(match calculated {
                None => *vat,
                Some(total) => total.checked_add(*vat).map_err(|_| RegisterError::Overflow)?,
            });
        }
        let Some(calculated) = calculated else {
            continue;
        };
        if calculated.currency != stated.currency {
            continue;
        }
        let key = (month.clone(), stated.currency.code(), document.kind);
        let acc = groups.entry(key).or_insert_with(|| ReconAcc {
            month: month.clone(),
            currency: stated.currency,
            kind: document.kind,
            calculated: 0,
            stated: 0,
            disagree: 0,
        });
        added(&mut acc.calculated, calculated)?;
        added(&mut acc.stated, stated)?;
        if calculated != stated {
            acc.disagree += 1;
        }
    }
    let mut out = Vec::new();
    for acc in groups.into_values() {
        let calculated = Money::new(acc.calculated, acc.currency);
        let stated = Money::new(acc.stated, acc.currency);
        let difference = calculated
            .checked_sub(stated)
            .map_err(|_| RegisterError::Overflow)?;
        out.push(Reconciliation {
            month: acc.month,
            currency: acc.currency.code().to_string(),
            kind: acc.kind,
            calculated_vat: crate::export::amount(calculated),
            stated_vat: crate::export::amount(stated),
            difference: crate::export::amount(difference),
            documents_disagree: acc.disagree,
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------- the build ----

/// What the store hands in: a selection, whether to keep only ready documents, how many
/// documents in the tenant have no invoice date, and the documents themselves.
#[derive(Debug, Clone)]
pub struct RegisterInput {
    pub selection: Selection,
    pub only_ready: bool,
    pub undated_in_tenant: usize,
    pub facts: Vec<RegisterFacts>,
}

/// The order rows appear in: dated first, oldest first, then by supplier, number and id.
fn order(document: &RegisterDocument) -> (bool, Option<Date>, String, String, String) {
    (
        document.invoice_date.is_none(),
        document.invoice_date,
        document.supplier.clone().unwrap_or_default(),
        document.invoice_number.clone().unwrap_or_default(),
        document.id.clone(),
    )
}

/// Why a page stopped for a person, in a sentence.
fn why(reason: ReviewReason) -> &'static str {
    match reason {
        ReviewReason::TopRungFailed => "Every model tried could not read this page with enough certainty.",
        ReviewReason::TooLong => "This page is longer than one model call can read.",
        ReviewReason::Refused => "The model refused to read this page.",
        ReviewReason::Budget => "This document's spending limit was reached before the page was read.",
        ReviewReason::CallCap => "The limit on calls for this document was reached before the page was read.",
        ReviewReason::CannotBeSentSafely => "This page could not be sent to a model safely.",
        ReviewReason::RequestRejected => "A model request for this page was rejected.",
        ReviewReason::NoRungAvailable => "No model was available to read this page.",
        ReviewReason::AmountsIncludeVat => {
            "The amounts on this page include VAT, and it was not clear how much."
        }
        ReviewReason::TaxNotShown => "This page does not show VAT, so none was added.",
    }
}

fn check(document: &RegisterDocument, reason: &str, sentence: String, page: Option<u32>) -> ToCheckRow {
    ToCheckRow {
        document_id: document.id.clone(),
        filename: document.filename.clone(),
        reason: reason.to_string(),
        sentence,
        page,
    }
}

/// Everything a person still has to look at, in row order.
fn to_check(readings: &[Reading]) -> Vec<ToCheckRow> {
    let mut out = Vec::new();
    for reading in readings {
        let document = &reading.document;
        let mut own = Vec::new();
        match document.status {
            DocumentStatus::Received | DocumentStatus::Processing => own.push(check(
                document,
                "still_reading",
                "This document is still being read, so its figures are not in the register yet.".to_string(),
                None,
            )),
            _ => {}
        }
        if document.invoice_date.is_none() {
            own.push(check(
                document,
                "no_invoice_date",
                "This document has no invoice date, so it is not in a month.".to_string(),
                None,
            ));
        }
        if document.flags.iter().any(|flag| flag == "mixed_currency") {
            own.push(check(
                document,
                "mixed_currency",
                "The amounts on this document are not all in one currency, so they are not added."
                    .to_string(),
                None,
            ));
        }
        if let Some(duplicate) = &document.duplicate_of {
            own.push(check(
                document,
                "possible_duplicate",
                format!("This document looks like document {duplicate}."),
                None,
            ));
        }
        for page in &reading.pages {
            own.push(check(
                document,
                page.reason.as_str(),
                why(page.reason).to_string(),
                Some(page.page),
            ));
        }
        // A document awaiting review with no page reason still needs a person, so it says so.
        if document.status == DocumentStatus::NeedsReview && own.is_empty() {
            own.push(check(
                document,
                "needs_review",
                "This document needs a person to check it.".to_string(),
                None,
            ));
        }
        own.sort_by(|a, b| a.page.cmp(&b.page).then_with(|| a.reason.cmp(&b.reason)));
        out.append(&mut own);
    }
    out
}

/// The share of finished documents that needed nobody, in basis points.
fn automation_rate_bps(done: usize, finished: usize) -> Option<u32> {
    if finished == 0 {
        return None;
    }
    let rate = (done as u128 * 10_000 + finished as u128 / 2) / finished as u128;
    u32::try_from(rate).ok()
}

/// Builds the register from what the store handed in.
///
/// The register is derived every time; nothing here is stored. Rows follow the selection in date
/// order, undated documents last. With `only_ready`, documents awaiting review are left out of the
/// rows and the sums and counted in `manifest.awaiting_left_out`. A period selection is applied
/// again to each document's authoritative invoice date, so a candidate the store's SQL let through
/// cannot appear in the register.
pub fn build(input: RegisterInput) -> Result<Register, RegisterError> {
    let RegisterInput {
        selection,
        only_ready,
        undated_in_tenant,
        facts,
    } = input;

    if facts.len() > MAX_DOCUMENTS {
        return Err(RegisterError::TooManyDocuments);
    }

    let mut readings: Vec<Reading> = facts.iter().map(read).collect();
    if let Selection::Period { from, to } = &selection {
        readings.retain(|reading| {
            reading
                .document
                .invoice_date
                .is_some_and(|date| date >= *from && date <= *to)
        });
    }
    readings.sort_by_key(|reading| order(&reading.document));

    let done = readings
        .iter()
        .filter(|reading| reading.document.status == DocumentStatus::Done)
        .count();
    let needs_review = readings
        .iter()
        .filter(|reading| reading.document.status == DocumentStatus::NeedsReview)
        .count();
    let still_reading = readings.len() - done - needs_review;
    let undated_in_selection = readings
        .iter()
        .filter(|reading| reading.document.invoice_date.is_none())
        .count();
    let awaiting_left_out = if only_ready { needs_review } else { 0 };
    let documents = readings.len();

    let mut included = readings;
    if only_ready {
        included.retain(|reading| reading.document.status != DocumentStatus::NeedsReview);
    }

    let total_lines = included
        .iter()
        .try_fold(0usize, |total, reading| {
            let own = reading
                .current
                .iter()
                .filter(|event| matches!(event.payload(), EventPayload::LineItemBilled { .. }))
                .count();
            total.checked_add(own)
        })
        .ok_or(RegisterError::TooManyLines)?;
    if total_lines > MAX_LINES {
        return Err(RegisterError::TooManyLines);
    }

    let rows: Vec<RegisterDocument> = included.iter().map(|reading| reading.document.clone()).collect();
    let line_rows = lines(&included);
    let vat = vat_by_category(&included)?;
    let totals = stated_totals(&included)?;
    let reconciled = reconciliation(&included)?;
    let checks = to_check(&included);

    let manifest = Manifest {
        selection: selection.clone(),
        only_ready,
        documents,
        done,
        needs_review,
        still_reading,
        undated_in_tenant,
        undated_in_selection,
        lines: line_rows.len(),
        automation_rate_bps: automation_rate_bps(done, done + needs_review),
        finished: done + needs_review,
        awaiting_left_out,
        software_version: env!("CARGO_PKG_VERSION").to_string(),
        notice: REGISTER_NOTICE,
    };

    Ok(Register {
        notice: REGISTER_NOTICE,
        version: env!("CARGO_PKG_VERSION").to_string(),
        selection,
        only_ready,
        rows,
        lines: line_rows,
        vat_by_category: vat,
        stated_totals: totals,
        reconciliation: reconciled,
        to_check: checks,
        manifest,
    })
}

#[cfg(test)]
mod tests;
