# Export and Workflow Readiness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give a bookkeeper or finance clerk a period register they can download from ClearCut in six accounting-ready shapes, and let them upload up to 20 invoices in one go, without a model call being needed to see or export what is already read.

**Architecture:** A new pure module `clearcut_core::register` turns stored events into one `Register` value: rows, lines, VAT by category, stated totals, reconciliation, the To check list and a manifest. The store loads a selection in three queries; the export crate lays the same value out as Excel, a zipped CSV bundle, JSON, PDF, HTML, Markdown and plain text; the server exposes one `POST /v1/register`; the web app adds a Register screen and bulk intake. Nothing new is stored: the register is derived from the append-only events.

**Tech Stack:** Rust (workspace crates `clearcut-core`, `clearcut-store`, `clearcut-export`, `clearcut-server`), Postgres, `rust_xlsxwriter`, `printpdf`, `zip`, vanilla JavaScript served by `clearcut-server`.

**Spec:** `docs/superpowers/specs/2026-10-30-export-readiness-design.md`

## Global Constraints

- South African VAT only. `TaxCategory`, `Money` (integer cents) and `ZA_VAT` are the only money and tax types.
- Do not add a model call to any path in this plan. Tests never spend money.
- Text that came off an uploaded document is data: escape HTML, write spreadsheet cells as strings, neutralise CSV formula leaders, never build SQL with it.
- Amounts are summed only within one currency. Credit notes are grouped separately from invoices. Undated documents are never put in a month.
- New migrations are additive only; never edit an applied migration. The next file is `0012_event_date.sql`.
- Every new public function and type gets a short doc comment. No em dashes in user-facing copy.
- Tests run against Postgres with `DATABASE_URL=postgres://clearcut@127.0.0.1:54329/clearcut`. Pure tests need no database.
- One branch and one small PR per slice. Every PR runs the full gate below; the feature's docs and worklog entry are Task 10's PR.

**Gate for every task that changes Rust:**

```bash
cargo fmt --all
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
DATABASE_URL=postgres://clearcut@127.0.0.1:54329/clearcut cargo test --workspace
```

### Slices and pull requests

The spec's six slices map onto the tasks like this. Each slice is one branch and one small PR, squash-merged once CI is green; start the next branch from the updated `main`.

| Slice | Tasks | Branch |
|---|---|---|
| 1 Core model | 1, 2, 3 | `register-core` |
| 2 Store selection | 4 | `register-store` |
| 3 Layouts | 5, 6 | `register-layouts` |
| 4 Endpoint and limits | 7 | `register-endpoint` |
| 5 Register screen | 8 | `register-screen` |
| 6 Bulk intake | 9 | `register-bulk` |
| Docs and delivery | 10 | `docs/export-readiness` |

Tasks 1 to 3 are one branch and one PR; tasks 5 and 6 are one branch and one PR. Task 9 does not depend on the register code, but it edits the same `app.js` as Task 8, so merge it after the screen. The docs and the worklog entry are written once, in Task 10, because they describe the feature as a whole; every earlier PR carries its code and its tests.

Before a slice's first task, create its branch from the updated `main`:

```bash
git switch main
git pull
git switch -c register-core
```

After its last task, push the branch and open the PR with the gate results in the body, wait for CI, then squash-merge it and delete the branch. The commit steps inside the tasks are the commits on that branch. The next slice starts from the updated `main`.

## File Structure

| File | Responsibility |
|---|---|
| `crates/clearcut-core/src/register/mod.rs` | The register value, its rules, and `build` |
| `crates/clearcut-core/src/register/tests.rs` | Pure tests for every rule |
| `crates/clearcut-store/migrations/0012_event_date.sql` | Generated `event_date` column and index |
| `crates/clearcut-store/src/register.rs` | Selection query: documents, current events, review pages |
| `crates/clearcut-store/src/register/tests.rs` | Store tests for the selection |
| `crates/clearcut-export/src/register/mod.rs` | `RegisterFormat`, `render_register` |
| `crates/clearcut-export/src/register/text.rs` | JSON, HTML, Markdown, plain text |
| `crates/clearcut-export/src/register/xlsx.rs` | Six-sheet Excel workbook |
| `crates/clearcut-export/src/register/csv.rs` | Zipped CSV bundle |
| `crates/clearcut-export/src/register/pdf.rs` | PDF register |
| `crates/clearcut-export/src/register/tests.rs` | Layout tests |
| `crates/clearcut-server/src/register.rs` | `POST /v1/register` |
| `crates/clearcut-server/src/register/tests.rs` | Endpoint tests |
| `crates/clearcut-server/src/web/app.js` | Register screen and bulk intake |
| `crates/clearcut-server/src/web/index.html` | Register nav link |
| `crates/clearcut-server/src/web/app.css` | Register controls |

---

### Task 1: The register value and its reading rules

**Files:**
- Create: `crates/clearcut-core/src/register/mod.rs`
- Create: `crates/clearcut-core/src/register/tests.rs`
- Modify: `crates/clearcut-core/src/lib.rs` (module list)
- Modify: `crates/clearcut-core/src/export/mod.rs` (make `amount` and `who` crate-visible)

**Interfaces:**
- Produces: `Selection`, `Kind`, `PageReview`, `RegisterFacts`, `RegisterDocument`, `RegisterLine`, `VatPart`, `VatGroup`, `StatedPart`, `StatedTotals`, `Reconciliation`, `ToCheckRow`, `Manifest`, `Register`, `RegisterError`, `parse_date`, `REGISTER_NOTICE`, `MAX_DOCUMENTS`, `MAX_LINES`, `Selection::label`, `Kind::as_str`, `Kind::label`.
- Produces (private to the module, used by Tasks 2 and 3): `struct Reading { document: RegisterDocument, current: Vec<Event>, pages: Vec<PageReview> }`.

- [ ] **Step 1: Write the failing tests for the value and date parsing**

Create `crates/clearcut-core/src/register/tests.rs`:

```rust
// Written by Kone Tshivhinda & Claude
//! Pure tests: no database, no clock, no model.

use super::*;
use crate::event::{
    Citation, Confidence, DocumentId, Event, EventId, EventParts, EventPayload, Extractor,
};
use crate::Money;
use std::num::NonZeroU32;
use time::macros::date as d;

fn event(doc: &str, id: &str, date: Option<time::Date>, payload: EventPayload, supersedes: Option<&Event>) -> Event {
    Event::new(EventParts {
        id: EventId::new(format!("{doc}:{id}")),
        date,
        payload,
        citation: Citation {
            document: DocumentId::new(doc),
            page: NonZeroU32::MIN,
            locator: None,
        },
        extractor: Extractor::System,
        confidence: Confidence::CERTAIN,
        supersedes: supersedes.map(|earlier| earlier.id().clone()),
    })
    .unwrap()
}

fn zar(minor: i64) -> Money {
    Money::zar(minor)
}

fn facts(id: &str, status: DocumentStatus, events: Vec<Event>) -> RegisterFacts {
    RegisterFacts {
        id: id.into(),
        filename: format!("{id}.pdf"),
        uploaded_at: time::OffsetDateTime::UNIX_EPOCH,
        status,
        page_count: Some(1),
        pages: Vec::new(),
        events,
    }
}

#[test]
fn a_date_is_read_strictly() {
    assert_eq!(parse_date("2026-09-01"), Some(d!(2026 - 09 - 01)));
    assert_eq!(parse_date("2024-02-29"), Some(d!(2024 - 02 - 29)));
    for bad in [
        "2026-9-1",
        "2026-02-30",
        "2026/09/01",
        "",
        "2026-09-01T00:00",
        "٢٠٢٦-٠٩-٠١",
        "2026-09-0x",
    ] {
        assert_eq!(parse_date(bad), None, "{bad}");
    }
}

#[test]
fn a_selection_has_one_clear_json_shape() {
    let period = Selection::Period { from: d!(2026 - 09 - 01), to: d!(2026 - 09 - 30) };
    assert_eq!(
        serde_json::to_value(&period).unwrap(),
        serde_json::json!({ "type": "period", "from": "2026-09-01", "to": "2026-09-30" })
    );
    let chosen = Selection::Documents { ids: vec!["doc-a".into()] };
    assert_eq!(
        serde_json::to_value(&chosen).unwrap(),
        serde_json::json!({ "type": "documents", "ids": ["doc-a"] })
    );
}

#[test]
fn the_latest_current_fact_wins() {
    let old = event(
        "doc",
        "supplier-1",
        Some(d!(2026 - 09 - 05)),
        EventPayload::SupplierIdentified { name: "Old Name".into(), vat_number: None },
        None,
    );
    let corrected = event(
        "doc",
        "supplier-2",
        Some(d!(2026 - 09 - 05)),
        EventPayload::SupplierIdentified { name: "Right Name".into(), vat_number: Some("4123456789".into()) },
        Some(&old),
    );
    let totals = event(
        "doc",
        "totals",
        Some(d!(2026 - 09 - 05)),
        EventPayload::TotalsStated { net: zar(100_000), vat: zar(15_000), gross: zar(115_000) },
        None,
    );
    let reading = read(&facts("doc", DocumentStatus::Done, vec![old, corrected, totals]));
    assert_eq!(reading.document.supplier.as_deref(), Some("Right Name"));
    assert_eq!(reading.document.supplier_vat_number.as_deref(), Some("4123456789"));
    assert_eq!(reading.document.invoice_date, Some(d!(2026 - 09 - 05)));
    assert_eq!(reading.document.kind, Kind::Invoice);
    assert_eq!(reading.document.currency.as_deref(), Some("ZAR"));
}

#[test]
fn a_credit_note_is_not_an_invoice() {
    let events = vec![
        event(
            "doc",
            "number",
            Some(d!(2026 - 09 - 06)),
            EventPayload::InvoiceNumberStated { number: "CN-1".into() },
            None,
        ),
        event(
            "doc",
            "credit",
            Some(d!(2026 - 09 - 06)),
            EventPayload::CreditNoteReferenced { original_invoice_number: "INV-1".into(), amount: zar(50_000) },
            None,
        ),
    ];
    assert_eq!(read(&facts("doc", DocumentStatus::Done, events)).document.kind, Kind::CreditNote);
}

#[test]
fn mixed_currencies_leave_the_currency_blank() {
    let events = vec![event(
        "doc",
        "totals",
        Some(d!(2026 - 09 - 07)),
        EventPayload::TotalsStated {
            net: zar(100_000),
            vat: zar(15_000),
            gross: Money::new(115_000, crate::Currency::Usd),
        },
        None,
    )];
    let reading = read(&facts("doc", DocumentStatus::Done, events));
    assert_eq!(reading.document.currency, None);
    assert!(reading.document.flags.contains(&"mixed_currency".to_string()));
}

#[test]
fn no_totals_leaves_the_amounts_blank_and_reads_currency_from_the_lines() {
    let events = vec![event(
        "doc",
        "line-1",
        Some(d!(2026 - 09 - 08)),
        EventPayload::LineItemBilled {
            description: "Consulting".into(),
            quantity_milli: 1_000,
            unit_price: zar(100_000),
            line_total: zar(100_000),
            tax_category: crate::TaxCategory::Standard,
        },
        None,
    )];
    let reading = read(&facts("doc", DocumentStatus::Done, events));
    assert_eq!(reading.document.currency.as_deref(), Some("ZAR"));
    assert_eq!(reading.document.net, None);
    assert_eq!(reading.document.gross, None);
    assert_eq!(reading.document.line_count, 1);
}

#[test]
fn review_pages_duplicates_and_readers_are_carried() {
    let events = vec![event(
        "doc",
        "duplicate",
        Some(d!(2026 - 09 - 09)),
        EventPayload::PossibleDuplicate {
            matching_document: DocumentId::new("earlier"),
            reason: crate::event::DuplicateReason::SupplierNumberDateTotal,
        },
        None,
    )];
    let mut source = facts("doc", DocumentStatus::NeedsReview, events);
    source.pages = vec![PageReview { page: 2, reason: ReviewReason::AmountsIncludeVat }];
    let reading = read(&source);
    assert_eq!(reading.document.duplicate_of.as_deref(), Some("earlier"));
    assert_eq!(reading.document.review_reasons, vec!["amounts_include_vat"]);
    assert_eq!(reading.document.read_by, vec!["ClearCut"]);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p clearcut-core register`
Expected: FAIL, the module `register` does not exist.

- [ ] **Step 3: Write the types, parsing and reading rule**

Add `pub mod register;` to `crates/clearcut-core/src/lib.rs` in alphabetical order (after `pub mod reading;`).

In `crates/clearcut-core/src/export/mod.rs`, change `fn amount` to `pub(crate) fn amount` and `fn who` to `pub(crate) fn who`.

Create `crates/clearcut-core/src/register/mod.rs`:

```rust
// Written by Kone Tshivhinda & Claude
//! One period register, built from stored events only (export readiness, P4-08).
//!
//! The register is not stored and does not change what a document says. It lays out the current
//! events of a selection of documents as rows, lines, VAT by category, stated totals, a
//! reconciliation, the things a person still has to check, and a manifest of what was included.
//!
//! Rules that are easy to get wrong, and are therefore tested:
//! - Only current events count, so a person's correction wins over the model's first reading.
//! - The period is the invoice date on the most recent current event that carries a date.
//! - An undated document is never put in a month. In an explicit selection it is included with a
//!   blank date and listed in To check.
//! - Amounts are added only within one currency. A document whose amounts disagree on currency
//!   has a blank currency and a `mixed_currency` flag; it is left out of cross-document sums.
//! - Only `done` documents feed the ready figures. `needs_review` documents feed the awaiting
//!   figures. Credit notes are grouped apart from invoices and are never netted against them.

use crate::document::DocumentStatus;
use crate::escalation::ReviewReason;
use crate::event::{Event, EventPayload};
use crate::export::Amount;
use crate::tax::TaxCategory;
use serde::{Deserialize, Serialize};
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
            Selection::Documents { ids } => format!("{} chosen document{}", ids.len(), if ids.len() == 1 { "" } else { "s" }),
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

/// The VAT ClearCut calculated against the VAT the documents state, for one month.
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
        EventPayload::SupplierIdentified { name, vat_number } => {
            Some((name.clone(), vat_number.clone()))
        }
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
        EventPayload::PossibleDuplicate { matching_document, .. } => {
            Some(matching_document.as_str().to_string())
        }
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

    let line_amounts: Vec<crate::Money> = current
        .iter()
        .filter_map(|event| match event.payload() {
            EventPayload::LineItemBilled { unit_price, line_total, .. } => {
                Some(vec![*unit_price, *line_total])
            }
            _ => None,
        })
        .flatten()
        .collect();
    let amounts = match totals {
        Some((net, vat, gross)) => vec![net, vat, gross],
        None => line_amounts,
    };
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
        Some((net, vat, gross)) => (Some(crate::export::amount(net)), Some(crate::export::amount(vat)), Some(crate::export::amount(gross))),
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

#[cfg(test)]
mod tests;
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p clearcut-core register`
Expected: PASS, 6 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/clearcut-core/src/register crates/clearcut-core/src/lib.rs crates/clearcut-core/src/export/mod.rs
git commit -m "feat(core): read a register row from a document's current events"
```

---

### Task 2: Line rows, VAT groups, stated totals and reconciliation

**Files:**
- Modify: `crates/clearcut-core/src/register/mod.rs` (row helpers, grouping)
- Modify: `crates/clearcut-core/src/register/tests.rs` (tests for every grouping rule)

**Interfaces:**
- Consumes: `Reading { document, current, pages }`, `read(&RegisterFacts)`, `RegisterDocument`, `RegisterLine`, `VatGroup`, `VatPart`, `StatedTotals`, `StatedPart`, `Reconciliation`, `RegisterError`, `Kind`, from Task 1.
- Produces (private to the module, used by Task 3's `build`):
  - `fn lines(readings: &[Reading]) -> Vec<RegisterLine>`
  - `fn vat_by_category(readings: &[Reading]) -> Result<Vec<VatGroup>, RegisterError>`
  - `fn stated_totals(readings: &[Reading]) -> Result<Vec<StatedTotals>, RegisterError>`
  - `fn reconciliation(readings: &[Reading]) -> Result<Vec<Reconciliation>, RegisterError>`

- [ ] **Step 1: Write the failing tests for the rows and the sums**

Add to the bottom of `crates/clearcut-core/src/register/tests.rs`, after the Task 1 tests. Also add `use crate::TaxCategory;` under the existing `use crate::Money;` line.

```rust
fn line_on(doc: &str, id: &str, page: u32, description: &str, quantity_milli: i64, unit_minor: i64) -> Event {
    Event::new(EventParts {
        id: EventId::new(format!("{doc}:{id}")),
        date: Some(d!(2026 - 09 - 10)),
        payload: EventPayload::LineItemBilled {
            description: description.into(),
            quantity_milli,
            unit_price: zar(unit_minor),
            line_total: zar(unit_minor * quantity_milli / 1_000),
            tax_category: TaxCategory::Standard,
        },
        citation: Citation {
            document: DocumentId::new(doc),
            page: NonZeroU32::new(page).unwrap(),
            locator: None,
        },
        extractor: Extractor::System,
        confidence: Confidence::CERTAIN,
        supersedes: None,
    })
    .unwrap()
}

fn vat_on(doc: &str, id: &str, date: Option<time::Date>, category: TaxCategory, taxable: i64, vat: i64) -> Event {
    event(
        doc,
        id,
        date,
        EventPayload::VatCalculated {
            category,
            taxable: zar(taxable),
            rate_bps: category.rate_bps_in(crate::tax::ZA_VAT),
            vat: zar(vat),
        },
        None,
    )
}

fn totals_on(doc: &str, date: Option<time::Date>, net: i64, vat: i64, gross: i64) -> Event {
    event(
        doc,
        "totals",
        date,
        EventPayload::TotalsStated {
            net: zar(net),
            vat: zar(vat),
            gross: zar(gross),
        },
        None,
    )
}

fn numbered(doc: &str, date: Option<time::Date>, number: &str) -> Event {
    event(
        doc,
        "number",
        date,
        EventPayload::InvoiceNumberStated {
            number: number.into(),
        },
        None,
    )
}

#[test]
fn a_line_row_carries_its_document_number_and_page() {
    let reading = read(&facts(
        "doc",
        DocumentStatus::Done,
        vec![
            numbered("doc", Some(d!(2026 - 09 - 10)), "INV-1"),
            line_on("doc", "l1", 1, "Consulting", 1_000, 100_000),
            line_on("doc", "l2", 2, "Travel", 2_500, 10_000),
        ],
    ));
    let rows = lines(&[reading]);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].number, 1);
    assert_eq!(rows[0].invoice_number.as_deref(), Some("INV-1"));
    assert_eq!(rows[0].quantity, "1");
    assert_eq!(rows[0].line_total.minor, 100_000);
    assert_eq!(rows[1].number, 2);
    assert_eq!(rows[1].quantity, "2.5");
    assert_eq!(rows[1].page, 2);
    assert_eq!(rows[1].line_total.minor, 25_000);
}

#[test]
fn vat_is_added_per_category_and_side() {
    let on = Some(d!(2026 - 09 - 15));
    let done = read(&facts(
        "done",
        DocumentStatus::Done,
        vec![
            numbered("done", on, "INV-1"),
            vat_on("done", "std", on, TaxCategory::Standard, 100_000, 15_000),
            vat_on("done", "zero", on, TaxCategory::ZeroRated, 50_000, 0),
        ],
    ));
    let awaiting = read(&facts(
        "awaiting",
        DocumentStatus::NeedsReview,
        vec![
            numbered("awaiting", on, "INV-2"),
            vat_on("awaiting", "std", on, TaxCategory::Standard, 20_000, 3_000),
        ],
    ));
    let groups = vat_by_category(&[done, awaiting]).unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].category, TaxCategory::Standard);
    assert_eq!(groups[0].currency, "ZAR");
    assert_eq!(groups[0].rate_bps, 1_500);
    let ready = groups[0].ready.as_ref().unwrap();
    assert_eq!(ready.documents, 1);
    assert_eq!(ready.taxable.minor, 100_000);
    assert_eq!(ready.vat.minor, 15_000);
    assert_eq!(groups[0].awaiting.as_ref().unwrap().vat.minor, 3_000);
    assert_eq!(groups[1].category, TaxCategory::ZeroRated);
    assert!(groups[1].awaiting.is_none());
}

#[test]
fn a_credit_note_is_kept_apart_from_an_invoice() {
    let on = Some(d!(2026 - 09 - 16));
    let invoice = read(&facts(
        "inv",
        DocumentStatus::Done,
        vec![
            numbered("inv", on, "INV-9"),
            vat_on("inv", "vat", on, TaxCategory::Standard, 100_000, 15_000),
        ],
    ));
    let credit = read(&facts(
        "credit",
        DocumentStatus::Done,
        vec![
            event(
                "credit",
                "ref",
                on,
                EventPayload::CreditNoteReferenced {
                    original_invoice_number: "INV-1".into(),
                    amount: zar(-115_000),
                },
                None,
            ),
            vat_on("credit", "vat", on, TaxCategory::Standard, -100_000, -15_000),
        ],
    ));
    let groups = vat_by_category(&[invoice, credit]).unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].kind, Kind::Invoice);
    assert_eq!(groups[1].kind, Kind::CreditNote);
    assert_eq!(groups[1].ready.as_ref().unwrap().vat.minor, -15_000);
}

#[test]
fn a_document_without_a_date_is_left_out_of_the_sums() {
    let source = facts(
        "undated",
        DocumentStatus::Done,
        vec![
            numbered("undated", None, "INV-3"),
            totals_on("undated", None, 100_000, 15_000, 115_000),
            vat_on("undated", "vat", None, TaxCategory::Standard, 100_000, 15_000),
        ],
    );
    assert!(vat_by_category(&[read(&source)]).unwrap().is_empty());
    assert!(stated_totals(&[read(&source)]).unwrap().is_empty());
    assert!(reconciliation(&[read(&source)]).unwrap().is_empty());
}

#[test]
fn a_mixed_currency_document_is_left_out_of_the_sums() {
    let on = Some(d!(2026 - 09 - 18));
    let source = facts(
        "mixed",
        DocumentStatus::Done,
        vec![
            numbered("mixed", on, "INV-4"),
            event(
                "mixed",
                "totals",
                on,
                EventPayload::TotalsStated {
                    net: zar(100_000),
                    vat: zar(15_000),
                    gross: Money::new(115_000, crate::Currency::Usd),
                },
                None,
            ),
            vat_on("mixed", "vat", on, TaxCategory::Standard, 100_000, 15_000),
        ],
    );
    assert!(vat_by_category(&[read(&source)]).unwrap().is_empty());
    assert!(stated_totals(&[read(&source)]).unwrap().is_empty());
    assert!(reconciliation(&[read(&source)]).unwrap().is_empty());
}

#[test]
fn the_stated_totals_of_a_month_are_added_per_side() {
    let on = Some(d!(2026 - 09 - 19));
    let done = read(&facts(
        "done",
        DocumentStatus::Done,
        vec![
            numbered("done", on, "INV-1"),
            totals_on("done", on, 100_000, 15_000, 115_000),
        ],
    ));
    let awaiting = read(&facts(
        "awaiting",
        DocumentStatus::NeedsReview,
        vec![
            numbered("awaiting", on, "INV-2"),
            totals_on("awaiting", on, 200_000, 30_000, 230_000),
        ],
    ));
    let groups = stated_totals(&[done, awaiting]).unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].month, "2026-09");
    assert_eq!(groups[0].currency, "ZAR");
    let ready = groups[0].ready.as_ref().unwrap();
    assert_eq!(ready.documents, 1);
    assert_eq!(ready.gross.minor, 115_000);
    let awaiting = groups[0].awaiting.as_ref().unwrap();
    assert_eq!(awaiting.documents, 1);
    assert_eq!(awaiting.gross.minor, 230_000);
}

#[test]
fn the_vat_calculated_is_reconciled_with_the_vat_stated() {
    let on = Some(d!(2026 - 09 - 20));
    let agrees = read(&facts(
        "agrees",
        DocumentStatus::Done,
        vec![
            numbered("agrees", on, "INV-1"),
            totals_on("agrees", on, 100_000, 15_000, 115_000),
            vat_on("agrees", "vat", on, TaxCategory::Standard, 100_000, 15_000),
        ],
    ));
    let disagrees = read(&facts(
        "disagrees",
        DocumentStatus::NeedsReview,
        vec![
            numbered("disagrees", on, "INV-2"),
            totals_on("disagrees", on, 100_000, 14_000, 114_000),
            vat_on("disagrees", "vat", on, TaxCategory::Standard, 100_000, 15_000),
        ],
    ));
    let rows = reconciliation(&[agrees, disagrees]).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].calculated_vat.minor, 30_000);
    assert_eq!(rows[0].stated_vat.minor, 29_000);
    assert_eq!(rows[0].difference.minor, 1_000);
    assert_eq!(rows[0].documents_disagree, 1);
}

#[test]
fn a_document_with_no_calculated_vat_is_not_reconciled() {
    let on = Some(d!(2026 - 09 - 21));
    let only_stated = read(&facts(
        "stated",
        DocumentStatus::Done,
        vec![
            numbered("stated", on, "INV-7"),
            totals_on("stated", on, 100_000, 15_000, 115_000),
        ],
    ));
    assert!(reconciliation(&[only_stated]).unwrap().is_empty());
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p clearcut-core register`
Expected: FAIL, `cannot find function lines`, `vat_by_category`, `stated_totals`, `reconciliation`.

- [ ] **Step 3: Write the rows and the sums**

In `crates/clearcut-core/src/register/mod.rs`, add to the `use` block:

```rust
use crate::money::{Currency, Money};
use std::collections::BTreeMap;
```

Then insert the following above `#[cfg(test)] mod tests;`:

```rust
// ---------------------------------------------------------------- line rows ----

/// The line items of every document, in reading order, numbered from one per document.
fn lines(readings: &[Reading]) -> Vec<RegisterLine> {
    let mut out = Vec::new();
    for reading in readings {
        let document = &reading.document;
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
    *total = total
        .checked_add(amount.minor)
        .ok_or(RegisterError::Overflow)?;
    Ok(())
}

/// The last `TotalsStated` a document still says.
fn stated_of(reading: &Reading) -> Option<(Money, Money, Money)> {
    reading.current.iter().rev().find_map(|event| match event.payload() {
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
    let mut groups: BTreeMap<(String, &'static str, u8, u8, u32), VatAcc> = BTreeMap::new();
    for reading in readings {
        let document = &reading.document;
        let Some(ready) = side(document.status) else {
            continue;
        };
        let Some(month) = month_for_sums(document) else {
            continue;
        };
        let mut own: BTreeMap<(u8, u32), (TaxCategory, Currency, i64, i64)> = BTreeMap::new();
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
                .entry((*category as u8, *rate_bps))
                .or_insert((*category, taxable.currency, 0, 0));
            added(&mut entry.2, *taxable)?;
            added(&mut entry.3, *vat)?;
        }
        for ((category, rate_bps), (typed, currency, taxable, vat)) in own {
            let key = (
                month.clone(),
                currency.code(),
                document.kind as u8,
                category,
                rate_bps,
            );
            let acc = groups.entry(key).or_insert_with(|| VatAcc {
                month: month.clone(),
                currency,
                kind: document.kind,
                category: typed,
                rate_bps,
                ready: None,
                awaiting: None,
            });
            let part = if ready {
                &mut acc.ready
            } else {
                &mut acc.awaiting
            };
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
    let mut groups: BTreeMap<(String, &'static str, u8), StatedAcc> = BTreeMap::new();
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
        let key = (month.clone(), net.currency.code(), document.kind as u8);
        let acc = groups.entry(key).or_insert_with(|| StatedAcc {
            month: month.clone(),
            currency: net.currency,
            kind: document.kind,
            ready: None,
            awaiting: None,
        });
        let part = if ready {
            &mut acc.ready
        } else {
            &mut acc.awaiting
        };
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
    let mut groups: BTreeMap<(String, &'static str, u8), ReconAcc> = BTreeMap::new();
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
        let mut calculated = None;
        for event in &reading.current {
            let EventPayload::VatCalculated { vat, .. } = event.payload() else {
                continue;
            };
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
        let key = (month.clone(), stated.currency.code(), document.kind as u8);
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
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p clearcut-core register`
Expected: PASS, 15 tests (6 from Task 1, 9 here).

- [ ] **Step 5: Commit**

```bash
git add crates/clearcut-core/src/register
git commit -m "feat(core): add register line rows, VAT groups and reconciliation"
```

---

### Task 3: Building the register, the To check list and the manifest

**Files:**
- Modify: `crates/clearcut-core/src/register/mod.rs` (add `RegisterInput`, `build`, ordering, limits)
- Modify: `crates/clearcut-core/src/register/tests.rs` (builder tests)

**Interfaces:**
- Consumes: everything from Tasks 1 and 2.
- Produces: `pub struct RegisterInput { pub selection: Selection, pub only_ready: bool, pub undated_in_tenant: usize, pub facts: Vec<RegisterFacts> }` and `pub fn build(input: RegisterInput) -> Result<Register, RegisterError>`. Task 4's store method returns a `RegisterInput`; Task 7's endpoint calls `build`.

- [ ] **Step 1: Write the failing builder tests**

Add to the bottom of `crates/clearcut-core/src/register/tests.rs`:

```rust
fn request(facts: Vec<RegisterFacts>) -> RegisterInput {
    RegisterInput {
        selection: Selection::Documents {
            ids: facts.iter().map(|facts| facts.id.clone()).collect(),
        },
        only_ready: false,
        undated_in_tenant: 0,
        facts,
    }
}

#[test]
fn an_empty_selection_makes_an_empty_register() {
    let register = build(RegisterInput {
        selection: Selection::Documents { ids: Vec::new() },
        only_ready: false,
        undated_in_tenant: 2,
        facts: Vec::new(),
    })
    .unwrap();
    assert!(register.rows.is_empty());
    assert!(register.lines.is_empty());
    assert!(register.to_check.is_empty());
    assert_eq!(register.manifest.documents, 0);
    assert_eq!(register.manifest.undated_in_tenant, 2);
    assert_eq!(register.manifest.automation_rate_bps, None);
    assert_eq!(register.notice, REGISTER_NOTICE);
    assert!(!register.version.is_empty());
}

#[test]
fn rows_are_in_date_order_with_undated_last() {
    let late = facts("late", DocumentStatus::Done, vec![numbered("late", Some(d!(2026 - 09 - 20)), "INV-2")]);
    let early = facts("early", DocumentStatus::Done, vec![numbered("early", Some(d!(2026 - 09 - 01)), "INV-1")]);
    let undated = facts("undated", DocumentStatus::Done, vec![numbered("undated", None, "INV-3")]);
    let register = build(request(vec![late, undated, early])).unwrap();
    let ids: Vec<&str> = register.rows.iter().map(|row| row.id.as_str()).collect();
    assert_eq!(ids, vec!["early", "late", "undated"]);
    assert_eq!(register.manifest.undated_in_selection, 1);
}

#[test]
fn a_candidate_outside_the_period_is_dropped() {
    let inside = facts("inside", DocumentStatus::Done, vec![numbered("inside", Some(d!(2026 - 09 - 10)), "INV-1")]);
    let outside = facts("outside", DocumentStatus::Done, vec![numbered("outside", Some(d!(2026 - 10 - 01)), "INV-2")]);
    let register = build(RegisterInput {
        selection: Selection::Period {
            from: d!(2026 - 09 - 01),
            to: d!(2026 - 09 - 30),
        },
        only_ready: false,
        undated_in_tenant: 0,
        facts: vec![inside, outside],
    })
    .unwrap();
    assert_eq!(register.rows.len(), 1);
    assert_eq!(register.rows[0].id, "inside");
    assert_eq!(register.manifest.documents, 1);
}

#[test]
fn an_undated_document_is_listed_to_check() {
    let undated = facts("undated", DocumentStatus::Done, vec![numbered("undated", None, "INV-3")]);
    let register = build(request(vec![undated])).unwrap();
    assert_eq!(register.rows.len(), 1);
    let row = register
        .to_check
        .iter()
        .find(|row| row.reason == "no_invoice_date")
        .expect("the undated document is listed");
    assert_eq!(row.document_id, "undated");
    assert_eq!(row.page, None);
}

#[test]
fn a_page_that_stopped_for_a_person_is_listed_with_its_page() {
    let mut source = facts(
        "review",
        DocumentStatus::NeedsReview,
        vec![numbered("review", Some(d!(2026 - 09 - 05)), "INV-4")],
    );
    source.pages = vec![PageReview {
        page: 3,
        reason: ReviewReason::AmountsIncludeVat,
    }];
    let register = build(request(vec![source])).unwrap();
    let row = register
        .to_check
        .iter()
        .find(|row| row.reason == "amounts_include_vat")
        .expect("the page is listed");
    assert_eq!(row.page, Some(3));
    assert!(row.sentence.contains("include VAT"));
}

#[test]
fn a_document_still_reading_is_listed_to_check() {
    let reading = facts("reading", DocumentStatus::Processing, Vec::new());
    let register = build(request(vec![reading])).unwrap();
    assert_eq!(register.rows.len(), 1);
    assert!(register
        .to_check
        .iter()
        .any(|row| row.reason == "still_reading"));
    assert_eq!(register.manifest.automation_rate_bps, None);
}

#[test]
fn only_ready_leaves_awaiting_documents_out() {
    let done = facts(
        "done",
        DocumentStatus::Done,
        vec![numbered("done", Some(d!(2026 - 09 - 06)), "INV-1")],
    );
    let awaiting = facts(
        "awaiting",
        DocumentStatus::NeedsReview,
        vec![numbered("awaiting", Some(d!(2026 - 09 - 07)), "INV-2")],
    );
    let mut input = request(vec![done, awaiting]);
    input.only_ready = true;
    let register = build(input).unwrap();
    assert_eq!(register.rows.len(), 1);
    assert_eq!(register.manifest.documents, 2);
    assert_eq!(register.manifest.done, 1);
    assert_eq!(register.manifest.needs_review, 1);
    assert_eq!(register.manifest.awaiting_left_out, 1);
    assert_eq!(register.manifest.automation_rate_bps, Some(5_000));
}

#[test]
fn more_documents_than_the_limit_is_refused() {
    let many: Vec<RegisterFacts> = (0..=MAX_DOCUMENTS)
        .map(|n| facts(&format!("d{n}"), DocumentStatus::Done, Vec::new()))
        .collect();
    assert_eq!(
        build(request(many)).unwrap_err(),
        RegisterError::TooManyDocuments
    );
}

#[test]
fn more_lines_than_the_limit_is_refused() {
    let events: Vec<Event> = (0..=MAX_LINES)
        .map(|n| line_on("big", &format!("l{n}"), 1, "Item", 1_000, 100))
        .collect();
    let source = facts("big", DocumentStatus::Done, events);
    assert_eq!(build(request(vec![source])).unwrap_err(), RegisterError::TooManyLines);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p clearcut-core register`
Expected: FAIL, `cannot find type RegisterInput` and `cannot find function build`.

- [ ] **Step 3: Write the builder**

Insert the following above `#[cfg(test)] mod tests;` in `crates/clearcut-core/src/register/mod.rs`:

```rust
// ---------------------------------------------------------------- the build ----

/// What the store hands in: a selection, whether to keep only ready documents, how many dated
/// documents in the tenant have no date worth putting in a month, and the documents themselves.
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
        ReviewReason::TopRungFailed => {
            "Every model tried could not read this page with enough certainty."
        }
        ReviewReason::TooLong => "This page is longer than one model call can read.",
        ReviewReason::Refused => "The model refused to read this page.",
        ReviewReason::Budget => {
            "This document's spending limit was reached before the page was read."
        }
        ReviewReason::CallCap => {
            "The limit on calls for this document was reached before the page was read."
        }
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
                "This document is still being read, so its figures are not in the register yet."
                    .to_string(),
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
    let total_lines = facts
        .iter()
        .try_fold(0usize, |total, facts| {
            let own = facts
                .events
                .iter()
                .filter(|event| matches!(event.payload(), EventPayload::LineItemBilled { .. }))
                .count();
            total.checked_add(own)
        })
        .ok_or(RegisterError::TooManyLines)?;
    if total_lines > MAX_LINES {
        return Err(RegisterError::TooManyLines);
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
    readings.sort_by(|a, b| order(&a.document).cmp(&order(&b.document)));

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
        included.retain(|reading| reading.document.status == DocumentStatus::Done);
    }

    let rows: Vec<RegisterDocument> = included
        .iter()
        .map(|reading| reading.document.clone())
        .collect();
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
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p clearcut-core register`
Expected: PASS, 22 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/clearcut-core/src/register
git commit -m "feat(core): build a register with its checks, limits and manifest"
```

---

### Task 4: The store selection and the event date column

**Files:**
- Create: `crates/clearcut-store/migrations/0012_event_date.sql`
- Create: `crates/clearcut-store/src/register.rs`
- Create: `crates/clearcut-store/src/register/tests.rs`
- Modify: `crates/clearcut-store/src/lib.rs` (`pub mod register;` and one `Error` variant)

**Interfaces:**
- Consumes: `Selection`, `RegisterFacts`, `RegisterInput`, `PageReview`, `MAX_DOCUMENTS` from `clearcut_core::register`; `ReviewReason::parse`; `Events` as stored.
- Produces: `Store::register_sources(&self, tenant: TenantId, selection: &Selection, only_ready: bool) -> Result<RegisterInput>`. Task 7's `Results` implementation calls it and then `clearcut_core::register::build`.

- [ ] **Step 1: Write the failing store tests**

Create `crates/clearcut-store/src/register/tests.rs`:

```rust
// Written by Kone Tshivhinda & Claude
//! Store tests for the register selection. They need Postgres, like the rest.

use super::*;
use crate::page_readings::{PageDetails, PageState};
use crate::tests::{new_document, new_tenant, setup};
use crate::DocId;
use clearcut_core::document::DocumentStatus;
use clearcut_core::event::{Citation, Confidence, Event, EventId, EventParts, Extractor};
use sqlx::PgPool;
use std::num::NonZeroU32;
use time::macros::date as d;

fn dated(doc: DocId, id: &str, date: Option<time::Date>, number: &str, supersedes: Option<&Event>) -> Event {
    Event::new(EventParts {
        id: EventId::new(format!("{}:{id}", doc.0)),
        date,
        payload: EventPayload::InvoiceNumberStated {
            number: number.into(),
        },
        citation: Citation {
            document: doc.as_core_id(),
            page: NonZeroU32::MIN,
            locator: None,
        },
        extractor: Extractor::System,
        confidence: Confidence::CERTAIN,
        supersedes: supersedes.map(|earlier| earlier.id().clone()),
    })
    .unwrap()
}

async fn second_document(store: &Store, tenant: TenantId, seed: u8) -> DocId {
    store.add_document(tenant, &new_document(seed)).await.unwrap().id
}

fn period(from: time::Date, to: time::Date) -> Selection {
    Selection::Period { from, to }
}

#[sqlx::test]
async fn a_period_takes_the_documents_dated_in_it(pool: PgPool) {
    let (store, tenant, march) = setup(&pool).await;
    let april = second_document(&store, tenant, 2).await;
    store
        .append_events(tenant, march, &[dated(march, "n", Some(d!(2026 - 03 - 31)), "INV-1", None)])
        .await
        .unwrap();
    store
        .append_events(tenant, april, &[dated(april, "n", Some(d!(2026 - 04 - 15)), "INV-2", None)])
        .await
        .unwrap();
    let input = store
        .register_sources(tenant, &period(d!(2026 - 04 - 01), d!(2026 - 04 - 30)), false)
        .await
        .unwrap();
    assert_eq!(input.facts.len(), 1);
    assert_eq!(input.facts[0].id, april.0.to_string());
    assert_eq!(input.facts[0].events.len(), 1);
    assert_eq!(input.undated_in_tenant, 0);
    assert!(!input.only_ready);
}

#[sqlx::test]
async fn a_period_leaves_undated_documents_out_and_counts_them(pool: PgPool) {
    let (store, tenant, dated_document) = setup(&pool).await;
    let undated = second_document(&store, tenant, 2).await;
    store
        .append_events(
            tenant,
            dated_document,
            &[dated(dated_document, "n", Some(d!(2026 - 03 - 31)), "INV-1", None)],
        )
        .await
        .unwrap();
    store
        .append_events(tenant, undated, &[dated(undated, "n", None, "INV-2", None)])
        .await
        .unwrap();
    let input = store
        .register_sources(tenant, &period(d!(2026 - 03 - 01), d!(2026 - 03 - 31)), false)
        .await
        .unwrap();
    assert_eq!(input.facts.len(), 1);
    assert_eq!(input.undated_in_tenant, 1);
}

#[sqlx::test]
async fn a_period_takes_the_date_of_the_latest_current_correction(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let first = dated(doc, "number-v1", Some(d!(2026 - 03 - 10)), "INV-1", None);
    let correction = dated(doc, "number-v2", Some(d!(2026 - 04 - 20)), "INV-1", Some(&first));
    store
        .append_events(tenant, doc, &[first, correction])
        .await
        .unwrap();
    let march = store
        .register_sources(tenant, &period(d!(2026 - 03 - 01), d!(2026 - 03 - 31)), false)
        .await
        .unwrap();
    assert!(march.facts.is_empty());
    let april = store
        .register_sources(tenant, &period(d!(2026 - 04 - 01), d!(2026 - 04 - 30)), false)
        .await
        .unwrap();
    assert_eq!(april.facts.len(), 1);
    assert_eq!(april.facts[0].events.len(), 1);
    assert_eq!(
        april.facts[0].events[0].id().as_str(),
        format!("{}:number-v2", doc.0)
    );
}

#[sqlx::test]
async fn a_named_selection_takes_documents_whatever_their_date(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    store
        .append_events(tenant, doc, &[dated(doc, "n", None, "INV-9", None)])
        .await
        .unwrap();
    let input = store
        .register_sources(
            tenant,
            &Selection::Documents {
                ids: vec![doc.0.to_string()],
            },
            false,
        )
        .await
        .unwrap();
    assert_eq!(input.facts.len(), 1);
    assert_eq!(input.facts[0].status, DocumentStatus::Received);
    assert_eq!(input.facts[0].events.len(), 1);
}

#[sqlx::test]
async fn a_named_document_of_another_tenant_is_not_returned(pool: PgPool) {
    let (store, tenant, _doc) = setup(&pool).await;
    let other = new_tenant(&pool, "Other").await.unwrap();
    let other_doc = store.add_document(other, &new_document(2)).await.unwrap().id;
    let input = store
        .register_sources(
            tenant,
            &Selection::Documents {
                ids: vec![other_doc.0.to_string()],
            },
            false,
        )
        .await
        .unwrap();
    assert!(input.facts.is_empty());
}

#[sqlx::test]
async fn the_pages_waiting_for_a_person_are_carried(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    store
        .save_page_reading(
            tenant,
            doc,
            2,
            &PageState::NeedsReview {
                reason: ReviewReason::AmountsIncludeVat,
                draft: None,
            },
            &PageDetails::default(),
        )
        .await
        .unwrap();
    let input = store
        .register_sources(
            tenant,
            &Selection::Documents {
                ids: vec![doc.0.to_string()],
            },
            true,
        )
        .await
        .unwrap();
    assert!(input.only_ready);
    assert_eq!(
        input.facts[0].pages,
        vec![PageReview {
            page: 2,
            reason: ReviewReason::AmountsIncludeVat,
        }]
    );
}

#[sqlx::test]
async fn a_document_id_that_is_not_a_uuid_is_refused(pool: PgPool) {
    let (store, tenant, _doc) = setup(&pool).await;
    let error = store
        .register_sources(
            tenant,
            &Selection::Documents {
                ids: vec!["not-a-uuid".into()],
            },
            false,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, Error::InvalidSelectionId(id) if id == "not-a-uuid"));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `DATABASE_URL=postgres://clearcut@127.0.0.1:54329/clearcut cargo test -p clearcut-store register`
Expected: FAIL, `no method named register_sources`, `no variant named InvalidSelectionId`.

- [ ] **Step 3: Write the migration, the error and the selection**

Create `crates/clearcut-store/migrations/0012_event_date.sql`:

```sql
-- Written by Kone Tshivhinda & Claude
-- The date a fact belongs to (the invoice date on a reading, not the day it was recorded), copied
-- out of the event body so a period can be selected without reading every body.
--
-- It is text, not a date, because a JSON date is already `YYYY-MM-DD` and that form sorts and
-- compares like a calendar date. A date column would need a cast inside the generated expression,
-- and a cast from text to date is not immutable, so Postgres refuses it in a generated column.

alter table events
    add column event_date text generated always as (body ->> 'date') stored,
    add constraint event_date_is_iso
        check (event_date is null or event_date ~ '^\d{4}-\d{2}-\d{2}$');

create index events_by_event_date on events (tenant_id, event_date)
    where event_date is not null;
```

In `crates/clearcut-store/src/lib.rs`, add `pub mod register;` after `pub mod page_reads;`, and add this variant to `Error` after `DocumentNotFound`:

```rust
    /// A selection named something that is not a document id.
    #[error("not a document id: {0}")]
    InvalidSelectionId(String),
```

Create `crates/clearcut-store/src/register.rs`:

```rust
// Written by Kone Tshivhinda & Claude
//! Which documents a register is made from: a period's documents or a named list, with their
//! current events and the pages that stopped for a person.
//!
//! The register itself is built in `clearcut_core::register`; this module only selects and hands
//! over. A document belongs to a month by the date of its most recent current event that carries
//! one, which is the date the register row shows.

use crate::{parse_status, Error, Result, Store, TenantId};
use clearcut_core::escalation::ReviewReason;
use clearcut_core::event::Event;
use clearcut_core::register::{PageReview, RegisterFacts, RegisterInput, Selection, MAX_DOCUMENTS};
use std::collections::HashMap;
use time::OffsetDateTime;
use uuid::Uuid;

type DocumentRow = (Uuid, String, String, Option<i32>, OffsetDateTime);
type EventRow = (Uuid, serde_json::Value);
type PageRow = (Uuid, i32, String);

impl Store {
    /// The documents a selection names, with their current events and the pages waiting for a
    /// person, in the shape the core's `build` takes.
    ///
    /// A period selection takes a document by the date of its most recent current event that
    /// carries one. A named selection takes the documents whatever their date; an id that is not
    /// a UUID is refused, and an id of another tenant is simply not found.
    pub async fn register_sources(
        &self,
        tenant: TenantId,
        selection: &Selection,
        only_ready: bool,
    ) -> Result<RegisterInput> {
        let mut tx = self.begin(tenant).await?;
        let limit = MAX_DOCUMENTS as i64 + 1;

        let documents: Vec<DocumentRow> = match selection {
            Selection::Period { from, to } => sqlx::query_as(
                "with current_events as ( \
                     select e.document_id, e.seq, e.event_date \
                     from events e \
                     where e.tenant_id = $1 \
                       and not exists (select 1 from events s \
                                       where s.tenant_id = $1 and s.document_id = e.document_id \
                                         and s.supersedes = e.id) \
                 ), \
                 latest as ( \
                     select distinct on (document_id) document_id, event_date \
                     from current_events \
                     where event_date is not null \
                     order by document_id, seq desc \
                 ) \
                 select d.id, d.original_filename, d.status, d.page_count, d.created_at \
                 from documents d \
                 join latest l on l.document_id = d.id \
                 where d.tenant_id = $1 and l.event_date between $2 and $3 \
                 order by l.event_date, d.created_at, d.id \
                 limit $4",
            )
            .bind(tenant.0)
            .bind(from.to_string())
            .bind(to.to_string())
            .bind(limit)
            .fetch_all(&mut *tx)
            .await?,
            Selection::Documents { ids } => {
                let mut wanted = Vec::with_capacity(ids.len());
                for id in ids {
                    let parsed =
                        Uuid::parse_str(id).map_err(|_| Error::InvalidSelectionId(id.clone()))?;
                    wanted.push(parsed);
                }
                sqlx::query_as(
                    "select id, original_filename, status, page_count, created_at \
                     from documents \
                     where tenant_id = $1 and id = any($2) \
                     order by created_at, id \
                     limit $3",
                )
                .bind(tenant.0)
                .bind(&wanted)
                .bind(limit)
                .fetch_all(&mut *tx)
                .await?
            }
        };

        // A document whose current events all lack a date is one the period can never take.
        let undated: i64 = sqlx::query_scalar(
            "with current_events as ( \
                 select e.document_id, e.event_date \
                 from events e \
                 where e.tenant_id = $1 \
                   and not exists (select 1 from events s \
                                   where s.tenant_id = $1 and s.document_id = e.document_id \
                                     and s.supersedes = e.id) \
             ) \
             select count(*) from ( \
                 select document_id from current_events \
                 group by document_id having count(event_date) = 0 \
             ) undated",
        )
        .bind(tenant.0)
        .fetch_one(&mut *tx)
        .await?;

        let document_ids: Vec<Uuid> = documents.iter().map(|row| row.0).collect();

        let bodies: Vec<EventRow> = sqlx::query_as(
            "select e.document_id, e.body \
             from events e \
             where e.tenant_id = $1 and e.document_id = any($2) \
               and not exists (select 1 from events s \
                               where s.tenant_id = $1 and s.document_id = e.document_id \
                                 and s.supersedes = e.id) \
             order by e.document_id, e.seq",
        )
        .bind(tenant.0)
        .bind(&document_ids)
        .fetch_all(&mut *tx)
        .await?;

        let pages: Vec<PageRow> = sqlx::query_as(
            "select document_id, page, review_reason \
             from page_readings \
             where tenant_id = $1 and document_id = any($2) and state = 'needs_review' \
             order by document_id, page",
        )
        .bind(tenant.0)
        .bind(&document_ids)
        .fetch_all(&mut *tx)
        .await?;

        tx.commit().await?;

        let mut facts: Vec<RegisterFacts> = Vec::with_capacity(documents.len());
        let mut place: HashMap<Uuid, usize> = HashMap::new();
        for (id, filename, status, page_count, created_at) in documents {
            place.insert(id, facts.len());
            facts.push(RegisterFacts {
                id: id.to_string(),
                filename,
                uploaded_at: created_at,
                status: parse_status(&status)?,
                page_count: page_count.and_then(|pages| u32::try_from(pages).ok()),
                pages: Vec::new(),
                events: Vec::new(),
            });
        }
        for (document, body) in bodies {
            let event = serde_json::from_value::<Event>(body)
                .map_err(|error| Error::InvalidStoredEvent(error.to_string()))?;
            if let Some(index) = place.get(&document) {
                facts[*index].events.push(event);
            }
        }
        for (document, page, reason) in pages {
            let reason = ReviewReason::parse(&reason)
                .ok_or_else(|| Error::InvalidStoredPageReading("an unknown review reason".to_string()))?;
            let page = u32::try_from(page)
                .map_err(|_| Error::InvalidStoredPageReading("a negative page number".to_string()))?;
            if let Some(index) = place.get(&document) {
                facts[*index].pages.push(PageReview { page, reason });
            }
        }

        Ok(RegisterInput {
            selection: selection.clone(),
            only_ready,
            undated_in_tenant: usize::try_from(undated).unwrap_or(usize::MAX),
            facts,
        })
    }
}

#[cfg(test)]
mod tests;
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `DATABASE_URL=postgres://clearcut@127.0.0.1:54329/clearcut cargo test -p clearcut-store register`
Expected: PASS, 7 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/clearcut-store/migrations/0012_event_date.sql crates/clearcut-store/src/register.rs crates/clearcut-store/src/register crates/clearcut-store/src/lib.rs
git commit -m "feat(store): select the documents and pages a register is made from"
```

---

### Task 5: The register in JSON, HTML, Markdown and plain text

**Files:**
- Create: `crates/clearcut-export/src/register/mod.rs`
- Create: `crates/clearcut-export/src/register/text.rs`
- Create: `crates/clearcut-export/src/register/tests.rs`
- Modify: `crates/clearcut-export/src/lib.rs` (`mod register;` and its exports)
- Modify: `crates/clearcut-export/src/text.rs` (make `STYLE` crate-visible)
- Modify: `crates/clearcut-export/Cargo.toml` (`time`, for the register's dates)

**Interfaces:**
- Consumes: `Register` from `clearcut_core::register` (built by Task 3).
- Produces: `pub enum RegisterFormat { Json, Html, Markdown, Text }` with `ALL`, `parse`, `extension`, `content_type`, and `pub fn render_register(format: RegisterFormat, register: &Register) -> Result<Rendered, ExportError>`. Task 6 adds `Xlsx`, `CsvBundle` and `Pdf` to the same enum and renderer; Task 7 calls `render_register`.

- [ ] **Step 1: Write the failing tests**

Create `crates/clearcut-export/src/register/tests.rs`:

```rust
// Written by Kone Tshivhinda & Claude
//! Layout tests for the register files. They build the register in the core, so the figures are
//! the ones a real selection would produce.

use super::*;
use clearcut_core::document::DocumentStatus;
use clearcut_core::event::{Citation, Confidence, DocumentId, Event, EventId, EventParts, EventPayload, Extractor};
use clearcut_core::register::{build, Register, RegisterFacts, RegisterInput, Selection};
use clearcut_core::Money;
use clearcut_core::TaxCategory;
use std::num::NonZeroU32;
use time::macros::date as d;

fn event(doc: &str, id: &str, payload: EventPayload) -> Event {
    Event::new(EventParts {
        id: EventId::new(format!("{doc}:{id}")),
        date: Some(d!(2026 - 09 - 15)),
        payload,
        citation: Citation {
            document: DocumentId::new(doc),
            page: NonZeroU32::MIN,
            locator: None,
        },
        extractor: Extractor::System,
        confidence: Confidence::CERTAIN,
        supersedes: None,
    })
    .unwrap()
}

fn register() -> Register {
    let doc = "d1";
    let facts = RegisterFacts {
        id: doc.into(),
        filename: "invoice.pdf".into(),
        uploaded_at: time::OffsetDateTime::UNIX_EPOCH,
        status: DocumentStatus::Done,
        page_count: Some(1),
        pages: Vec::new(),
        events: vec![
            event(
                doc,
                "supplier",
                EventPayload::SupplierIdentified {
                    name: "<script>alert(1)</script>".into(),
                    vat_number: Some("4123456789".into()),
                },
            ),
            event(
                doc,
                "number",
                EventPayload::InvoiceNumberStated {
                    number: "INV-1".into(),
                },
            ),
            event(
                doc,
                "line",
                EventPayload::LineItemBilled {
                    description: "Consulting".into(),
                    quantity_milli: 1_000,
                    unit_price: Money::zar(150_000),
                    line_total: Money::zar(150_000),
                    tax_category: TaxCategory::Standard,
                },
            ),
            event(
                doc,
                "vat",
                EventPayload::VatCalculated {
                    category: TaxCategory::Standard,
                    taxable: Money::zar(150_000),
                    rate_bps: 1_500,
                    vat: Money::zar(22_500),
                },
            ),
            event(
                doc,
                "totals",
                EventPayload::TotalsStated {
                    net: Money::zar(150_000),
                    vat: Money::zar(22_500),
                    gross: Money::zar(172_500),
                },
            ),
        ],
    };
    build(RegisterInput {
        selection: Selection::Documents {
            ids: vec![doc.into()],
        },
        only_ready: false,
        undated_in_tenant: 0,
        facts: vec![facts],
    })
    .unwrap()
}

#[test]
fn every_register_format_makes_a_file_of_the_right_kind() {
    let register = register();
    for format in RegisterFormat::ALL {
        let file = render_register(format, &register).unwrap();
        assert!(!file.bytes.is_empty(), "{format:?}");
        assert_eq!(file.extension, format.extension());
        assert_eq!(RegisterFormat::parse(format.extension()), Some(format));
    }
    let json = render_register(RegisterFormat::Json, &register).unwrap();
    assert!(serde_json::from_slice::<serde_json::Value>(&json.bytes).is_ok());
    assert!(render_register(RegisterFormat::Html, &register)
        .unwrap()
        .bytes
        .starts_with(b"<!doctype html>"));
    assert!(render_register(RegisterFormat::Markdown, &register)
        .unwrap()
        .bytes
        .starts_with(b"# Register"));
    assert!(render_register(RegisterFormat::Text, &register)
        .unwrap()
        .bytes
        .starts_with(b"REGISTER"));
    assert_eq!(RegisterFormat::parse("exe"), None);
}

#[test]
fn text_from_the_document_cannot_become_markup() {
    let register = register();
    let html = String::from_utf8(render_register(RegisterFormat::Html, &register).unwrap().bytes).unwrap();
    assert!(!html.contains("<script>"));
    assert!(html.contains("&lt;script&gt;"));
}

#[test]
fn the_same_figures_are_in_every_format() {
    let register = register();
    for format in RegisterFormat::ALL {
        let text = String::from_utf8(render_register(format, &register).unwrap().bytes).unwrap();
        assert!(text.contains("INV-1"), "{format:?}");
        assert!(text.contains("R1\u{a0}500.00"), "{format:?}");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p clearcut-export register`
Expected: FAIL, `cannot find type RegisterFormat`, `cannot find function render_register`.

- [ ] **Step 3: Write the format enum and the renderers**

In `crates/clearcut-export/Cargo.toml`, add `time = "0.3.55"` after `thiserror = "2.0.21"`, and add a `[dev-dependencies]` section holding `zip = "9.1.2"` and `time = { version = "0.3.55", features = ["macros"] }`. The register's dates and the tests' `d!` macro both need it; the export crate did not use `time` before.

In `crates/clearcut-export/src/lib.rs`, add `mod register;` after `mod pdf;` and, beside the other `pub use`, add:

```rust
pub use register::{render_register, RegisterFormat};
```

In `crates/clearcut-export/src/text.rs`, change `const STYLE: &str =` to `pub(crate) const STYLE: &str =`.

Create `crates/clearcut-export/src/register/mod.rs`:

```rust
// Written by Kone Tshivhinda & Claude
//! The register files: one value, laid out in the shapes a bookkeeper or finance team can use.
//!
//! Every file is laid out from **one** [`Register`], built in `clearcut_core::register`, so the
//! formats cannot disagree with each other.

mod text;

use crate::{ExportError, Rendered};
use clearcut_core::register::Register;

/// The formats a register can be downloaded as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegisterFormat {
    Json,
    Html,
    Markdown,
    Text,
}

impl RegisterFormat {
    pub const ALL: [RegisterFormat; 4] = [
        RegisterFormat::Json,
        RegisterFormat::Html,
        RegisterFormat::Markdown,
        RegisterFormat::Text,
    ];

    /// The name used in the API: `json`, `html`, `md`, `txt`.
    pub fn parse(name: &str) -> Option<RegisterFormat> {
        RegisterFormat::ALL
            .into_iter()
            .find(|format| format.extension() == name)
    }

    pub fn extension(self) -> &'static str {
        match self {
            RegisterFormat::Json => "json",
            RegisterFormat::Html => "html",
            RegisterFormat::Markdown => "md",
            RegisterFormat::Text => "txt",
        }
    }

    pub fn content_type(self) -> &'static str {
        match self {
            RegisterFormat::Json => "application/json",
            RegisterFormat::Html => "text/html; charset=utf-8",
            RegisterFormat::Markdown => "text/markdown; charset=utf-8",
            RegisterFormat::Text => "text/plain; charset=utf-8",
        }
    }
}

/// Makes the register file.
pub fn render_register(format: RegisterFormat, register: &Register) -> Result<Rendered, ExportError> {
    let bytes = match format {
        RegisterFormat::Json => text::json(register).into_bytes(),
        RegisterFormat::Html => text::html(register).into_bytes(),
        RegisterFormat::Markdown => text::markdown(register).into_bytes(),
        RegisterFormat::Text => text::plain(register).into_bytes(),
    };
    Ok(Rendered {
        content_type: format.content_type(),
        extension: format.extension(),
        bytes,
    })
}

#[cfg(test)]
mod tests;
```

Create `crates/clearcut-export/src/register/text.rs`:

```rust
// Written by Kone Tshivhinda & Claude
//! The register in JSON, HTML, Markdown and plain text.
//!
//! Names, descriptions and numbers that came off a document are data: HTML escapes every
//! character that matters, and Markdown cannot take a `|` out of a table cell.

use crate::text::{esc, STYLE};
use clearcut_core::export::Amount;
use clearcut_core::narration::format::{rate_bps, tax_category};
use clearcut_core::register::Register;
use time::Date;

/// The canonical register file: every field, with exact minor units beside each amount.
pub fn json(register: &Register) -> String {
    serde_json::to_string_pretty(register).unwrap_or_default()
}

fn date_text(date: Option<Date>) -> String {
    date.map(|date| date.to_string())
        .unwrap_or_else(|| "no date".to_string())
}

fn amount_text(amount: Option<&Amount>) -> &str {
    amount.map(|amount| amount.text.as_str()).unwrap_or("")
}

fn page_text(page: Option<u32>) -> String {
    page.map(|page| page.to_string()).unwrap_or_default()
}

/// A self-contained page: no script, no outside file.
pub fn html(register: &Register) -> String {
    let mut out = String::new();
    out.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    out.push_str(&format!(
        "<title>Register {}</title>\n<style>{STYLE}</style>\n</head>\n<body>\n",
        esc(&register.selection.label())
    ));
    out.push_str("<h1>Register</h1>\n");
    out.push_str(&format!(
        "<p class=\"muted\">{} &middot; {} document{} &middot; made by ClearCut {}</p>\n",
        esc(&register.selection.label()),
        register.manifest.documents,
        if register.manifest.documents == 1 { "" } else { "s" },
        esc(&register.version)
    ));
    if let Some(rate) = register.manifest.automation_rate_bps {
        out.push_str(&format!(
            "<p class=\"muted\">{} of finished documents needed nobody ({} of {} finished).</p>\n",
            rate_bps(rate),
            register.manifest.done,
            register.manifest.finished
        ));
    }
    if register.manifest.undated_in_tenant > 0 {
        out.push_str(&format!(
            "<p class=\"muted\">Documents with no invoice date in this account: {}.</p>\n",
            register.manifest.undated_in_tenant
        ));
    }

    out.push_str("<h2>Documents</h2>\n<table>\n<thead><tr>");
    for head in [
        "Date", "Supplier", "Number", "Kind", "Status", "Currency", "Net", "VAT", "Gross", "Lines",
    ] {
        out.push_str(&format!("<th>{head}</th>"));
    }
    out.push_str("</tr></thead>\n<tbody>\n");
    for row in &register.rows {
        out.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td>\
             <td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td>\
             <td class=\"num\">{}</td></tr>\n",
            esc(&date_text(row.invoice_date)),
            esc(row.supplier.as_deref().unwrap_or("")),
            esc(row.invoice_number.as_deref().unwrap_or("")),
            esc(row.kind.label()),
            esc(row.status.as_str()),
            esc(row.currency.as_deref().unwrap_or("")),
            esc(amount_text(row.net.as_ref())),
            esc(amount_text(row.vat.as_ref())),
            esc(amount_text(row.gross.as_ref())),
            row.line_count
        ));
    }
    out.push_str("</tbody>\n</table>\n");

    // A page cannot usefully carry fifty thousand lines: the workbook and the bundle do.
    if !register.lines.is_empty() {
        out.push_str(&format!(
            "<p class=\"muted\">This page carries the register, the totals and what needs a \
             person. The {} line items are in the workbook and the CSV bundle.</p>\n",
            register.lines.len()
        ));
    }

    if !register.vat_by_category.is_empty() {
        out.push_str("<h2>VAT</h2>\n<table>\n<thead><tr>");
        for head in [
            "Month",
            "Currency",
            "Kind",
            "Category",
            "Rate",
            "Side",
            "Documents",
            "Taxable",
            "VAT",
        ] {
            out.push_str(&format!("<th>{head}</th>"));
        }
        out.push_str("</tr></thead>\n<tbody>\n");
        for group in &register.vat_by_category {
            for (name, part) in [
                ("Ready", group.ready.as_ref()),
                ("Awaiting review", group.awaiting.as_ref()),
            ] {
                let Some(part) = part else {
                    continue;
                };
                out.push_str(&format!(
                    "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td class=\"num\">{}</td>\
                     <td>{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td>\
                     <td class=\"num\">{}</td></tr>\n",
                    esc(&group.month),
                    esc(&group.currency),
                    esc(group.kind.label()),
                    esc(&tax_category(group.category)),
                    esc(&rate_bps(group.rate_bps)),
                    name,
                    part.documents,
                    esc(&part.taxable.text),
                    esc(&part.vat.text)
                ));
            }
        }
        out.push_str("</tbody>\n</table>\n");
    }

    if !register.stated_totals.is_empty() {
        out.push_str("<h2>Stated totals</h2>\n<table>\n<thead><tr>");
        for head in [
            "Month",
            "Currency",
            "Kind",
            "Side",
            "Documents",
            "Net",
            "VAT",
            "Gross",
        ] {
            out.push_str(&format!("<th>{head}</th>"));
        }
        out.push_str("</tr></thead>\n<tbody>\n");
        for group in &register.stated_totals {
            for (name, part) in [
                ("Ready", group.ready.as_ref()),
                ("Awaiting review", group.awaiting.as_ref()),
            ] {
                let Some(part) = part else {
                    continue;
                };
                out.push_str(&format!(
                    "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td class=\"num\">{}</td>\
                     <td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td></tr>\n",
                    esc(&group.month),
                    esc(&group.currency),
                    esc(group.kind.label()),
                    name,
                    part.documents,
                    esc(&part.net.text),
                    esc(&part.vat.text),
                    esc(&part.gross.text)
                ));
            }
        }
        out.push_str("</tbody>\n</table>\n");
    }

    if !register.reconciliation.is_empty() {
        out.push_str("<h2>Reconciliation</h2>\n<table>\n<thead><tr>");
        for head in [
            "Month",
            "Currency",
            "Kind",
            "Calculated VAT",
            "Stated VAT",
            "Difference",
            "Documents that disagree",
        ] {
            out.push_str(&format!("<th>{head}</th>"));
        }
        out.push_str("</tr></thead>\n<tbody>\n");
        for row in &register.reconciliation {
            out.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td class=\"num\">{}</td>\
                 <td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td></tr>\n",
                esc(&row.month),
                esc(&row.currency),
                esc(row.kind.label()),
                esc(&row.calculated_vat.text),
                esc(&row.stated_vat.text),
                esc(&row.difference.text),
                row.documents_disagree
            ));
        }
        out.push_str("</tbody>\n</table>\n");
    }

    out.push_str("<h2>To check</h2>\n");
    if register.to_check.is_empty() {
        out.push_str("<p class=\"muted\">Nothing is waiting for a person.</p>\n");
    } else {
        out.push_str("<table>\n<thead><tr>");
        for head in ["Document", "Reason", "Page", "What to check"] {
            out.push_str(&format!("<th>{head}</th>"));
        }
        out.push_str("</tr></thead>\n<tbody>\n");
        for row in &register.to_check {
            out.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td class=\"num\">{}</td><td>{}</td></tr>\n",
                esc(&row.filename),
                esc(&row.reason),
                esc(&page_text(row.page)),
                esc(&row.sentence)
            ));
        }
        out.push_str("</tbody>\n</table>\n");
    }
    out.push_str(&format!("<p class=\"notice\">{}</p>\n", esc(register.notice)));
    out.push_str("</body>\n</html>\n");
    out
}

fn md(text: &str) -> String {
    text.replace('|', "\\|")
}

/// The register as Markdown, for a note, a wiki or a git repository.
pub fn markdown(register: &Register) -> String {
    let mut out = String::from("# Register\n\n");
    out.push_str(&format!(
        "_{} · {} documents · made by ClearCut {}_\n\n",
        md(&register.selection.label()),
        register.manifest.documents,
        md(&register.version)
    ));
    if let Some(rate) = register.manifest.automation_rate_bps {
        out.push_str(&format!(
            "{} of finished documents needed nobody ({} of {} finished).\n\n",
            rate_bps(rate),
            register.manifest.done,
            register.manifest.finished
        ));
    }

    out.push_str("## Documents\n\n| Date | Supplier | Number | Kind | Status | Currency | Net | VAT | Gross | Lines |\n");
    out.push_str("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |\n");
    for row in &register.rows {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            md(&date_text(row.invoice_date)),
            md(row.supplier.as_deref().unwrap_or("")),
            md(row.invoice_number.as_deref().unwrap_or("")),
            row.kind.label(),
            row.status.as_str(),
            md(row.currency.as_deref().unwrap_or("")),
            md(amount_text(row.net.as_ref())),
            md(amount_text(row.vat.as_ref())),
            md(amount_text(row.gross.as_ref())),
            row.line_count
        ));
    }
    out.push('\n');

    if !register.vat_by_category.is_empty() {
        out.push_str("## VAT\n\n| Month | Currency | Kind | Category | Rate | Side | Documents | Taxable | VAT |\n");
        out.push_str("| --- | --- | --- | --- | --- | --- | --- | --- | --- |\n");
        for group in &register.vat_by_category {
            for (name, part) in [
                ("Ready", group.ready.as_ref()),
                ("Awaiting review", group.awaiting.as_ref()),
            ] {
                let Some(part) = part else {
                    continue;
                };
                out.push_str(&format!(
                    "| {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
                    md(&group.month),
                    md(&group.currency),
                    group.kind.label(),
                    md(&tax_category(group.category)),
                    md(&rate_bps(group.rate_bps)),
                    name,
                    part.documents,
                    md(&part.taxable.text),
                    md(&part.vat.text)
                ));
            }
        }
        out.push('\n');
    }

    if !register.reconciliation.is_empty() {
        out.push_str("## Reconciliation\n\n| Month | Currency | Kind | Calculated VAT | Stated VAT | Difference | Documents that disagree |\n");
        out.push_str("| --- | --- | --- | --- | --- | --- | --- |\n");
        for row in &register.reconciliation {
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} | {} |\n",
                md(&row.month),
                md(&row.currency),
                row.kind.label(),
                md(&row.calculated_vat.text),
                md(&row.stated_vat.text),
                md(&row.difference.text),
                row.documents_disagree
            ));
        }
        out.push('\n');
    }

    out.push_str("## To check\n\n");
    if register.to_check.is_empty() {
        out.push_str("Nothing is waiting for a person.\n\n");
    } else {
        out.push_str("| Document | Reason | Page | What to check |\n| --- | --- | --- | --- |\n");
        for row in &register.to_check {
            out.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                md(&row.filename),
                md(&row.reason),
                md(&page_text(row.page)),
                md(&row.sentence)
            ));
        }
        out.push('\n');
    }
    out.push_str(&format!("---\n\n{}\n", register.notice));
    out
}

/// The register as plain text, for a terminal, an email or a quick check.
pub fn plain(register: &Register) -> String {
    let mut out = String::from("REGISTER\n");
    out.push_str(&format!("{}\n", register.selection.label()));
    out.push_str(&format!(
        "{} documents, {} done, {} awaiting review, {} still reading\n",
        register.manifest.documents,
        register.manifest.done,
        register.manifest.needs_review,
        register.manifest.still_reading
    ));
    if let Some(rate) = register.manifest.automation_rate_bps {
        out.push_str(&format!(
            "{} of finished documents needed nobody\n",
            rate_bps(rate)
        ));
    }

    out.push_str("\nDOCUMENTS\n");
    for row in &register.rows {
        out.push_str(&format!(
            "{}  {}  {}  {}  {}  {}  net {}  VAT {}  gross {}  {} lines\n",
            date_text(row.invoice_date),
            row.supplier.as_deref().unwrap_or(""),
            row.invoice_number.as_deref().unwrap_or(""),
            row.kind.label(),
            row.status.as_str(),
            row.currency.as_deref().unwrap_or(""),
            amount_text(row.net.as_ref()),
            amount_text(row.vat.as_ref()),
            amount_text(row.gross.as_ref()),
            row.line_count
        ));
    }

    // A page cannot usefully carry fifty thousand lines: the workbook and the bundle do.
    if !register.lines.is_empty() {
        out.push_str(&format!(
            "\n{} line items are in the workbook and the CSV bundle.\n",
            register.lines.len()
        ));
    }

    if !register.vat_by_category.is_empty() {
        out.push_str("\nVAT\n");
        for group in &register.vat_by_category {
            for (name, part) in [
                ("ready", group.ready.as_ref()),
                ("awaiting review", group.awaiting.as_ref()),
            ] {
                let Some(part) = part else {
                    continue;
                };
                out.push_str(&format!(
                    "{}  {}  {}  {}  {}  {}  {} doc(s)  {} taxable  {} VAT\n",
                    group.month,
                    group.currency,
                    group.kind.label(),
                    tax_category(group.category),
                    rate_bps(group.rate_bps),
                    name,
                    part.documents,
                    part.taxable.text,
                    part.vat.text
                ));
            }
        }
    }

    if !register.stated_totals.is_empty() {
        out.push_str("\nSTATED TOTALS\n");
        for group in &register.stated_totals {
            for (name, part) in [
                ("ready", group.ready.as_ref()),
                ("awaiting review", group.awaiting.as_ref()),
            ] {
                let Some(part) = part else {
                    continue;
                };
                out.push_str(&format!(
                    "{}  {}  {}  {}  {} doc(s)  net {}  VAT {}  gross {}\n",
                    group.month,
                    group.currency,
                    group.kind.label(),
                    name,
                    part.documents,
                    part.net.text,
                    part.vat.text,
                    part.gross.text
                ));
            }
        }
    }

    if !register.reconciliation.is_empty() {
        out.push_str("\nRECONCILIATION\n");
        for row in &register.reconciliation {
            out.push_str(&format!(
                "{}  {}  {}  calculated {}  stated {}  difference {}  {} disagree\n",
                row.month,
                row.currency,
                row.kind.label(),
                row.calculated_vat.text,
                row.stated_vat.text,
                row.difference.text,
                row.documents_disagree
            ));
        }
    }

    out.push_str("\nTO CHECK\n");
    if register.to_check.is_empty() {
        out.push_str("Nothing is waiting for a person.\n");
    }
    for row in &register.to_check {
        let page = row.page.map(|page| format!("  page {page}")).unwrap_or_default();
        out.push_str(&format!(
            "{}  {}  {}{}  {}\n",
            row.filename, row.reason, row.document_id, page, row.sentence
        ));
    }

    out.push_str(&format!("\n{}\n", register.notice));
    out
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p clearcut-export register`
Expected: PASS, 3 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/clearcut-export/Cargo.toml Cargo.lock crates/clearcut-export/src/register crates/clearcut-export/src/lib.rs crates/clearcut-export/src/text.rs
git commit -m "feat(export): lay the register out as JSON, HTML, Markdown and text"
```

### Task 6: The register in Excel, a zipped CSV bundle and PDF

**Files:**
- Modify: `crates/clearcut-export/Cargo.toml` (move `zip` from dev-dependencies to dependencies)
- Modify: `crates/clearcut-export/src/xlsx.rs` (`units` and `head` become crate-visible)
- Modify: `crates/clearcut-export/src/pdf.rs` (`Layout`, its methods and `BODY` become crate-visible; new `finish`)
- Create: `crates/clearcut-export/src/register/xlsx.rs`
- Create: `crates/clearcut-export/src/register/csv.rs`
- Create: `crates/clearcut-export/src/register/pdf.rs`
- Modify: `crates/clearcut-export/src/register/mod.rs` (`RegisterFormat` gains three variants)
- Modify: `crates/clearcut-export/src/register/tests.rs`

**Interfaces:**
- Consumes: `Register` from `clearcut_core::register`; `crate::xlsx::{head, units}`; `crate::pdf::{Layout, BODY}`; `crate::text::cell`; `RegisterFormat` and `render_register` from Task 5.
- Produces: `RegisterFormat::{Xlsx, CsvBundle, Pdf}`, `RegisterFormat::name` (the API name, which is `csv` for the bundle while its file extension is `zip`), and renderers that `render_register` dispatches to. Task 7 asks for a format by `name` and names the download by `extension`.

- [ ] **Step 1: Extend the failing tests**

`RegisterFormat::ALL` will now include three binary formats, so the two tests that turn every file into a `String` must name the text formats explicitly, and the name/extension pair is no longer the same for the bundle.

In `crates/clearcut-export/src/register/tests.rs`, add this constant under the imports:

```rust
/// The formats whose bytes are text. The workbook, the bundle and the PDF are checked as bytes.
const TEXT_FORMATS: [RegisterFormat; 4] = [
    RegisterFormat::Json,
    RegisterFormat::Html,
    RegisterFormat::Markdown,
    RegisterFormat::Text,
];
```

Then replace, in `every_register_format_makes_a_file_of_the_right_kind`:

```rust
        assert_eq!(file.extension, format.extension());
        assert_eq!(RegisterFormat::parse(format.extension()), Some(format));
```

with

```rust
        assert_eq!(file.extension, format.extension());
        assert_eq!(RegisterFormat::parse(format.name()), Some(format));
```

and add, just before `assert_eq!(RegisterFormat::parse("exe"), None);`:

```rust
    assert_eq!(
        RegisterFormat::parse("csv"),
        Some(RegisterFormat::CsvBundle)
    );
    assert_eq!(RegisterFormat::parse("zip"), None);
```

Replace the loop header in `the_same_figures_are_in_every_format`:

```rust
    for format in RegisterFormat::ALL {
```

with

```rust
    for format in TEXT_FORMATS {
```

Then add the new tests at the end of the file:

```rust
/// Every file inside a workbook or a bundle, by name.
fn entries(bytes: Vec<u8>) -> Vec<(String, String)> {
    use std::io::Read;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("a zip file");
    let names: Vec<String> = archive.file_names().map(str::to_string).collect();
    let mut out = Vec::new();
    for name in names {
        let mut text = String::new();
        archive
            .by_name(&name)
            .expect(&name)
            .read_to_string(&mut text)
            .unwrap();
        out.push((name, text));
    }
    out
}

fn file_named(files: &[(String, String)], name: &str) -> String {
    files
        .iter()
        .find(|(entry, _)| entry == name)
        .map(|(_, text)| text.clone())
        .unwrap_or_else(|| panic!("{name} is missing"))
}

#[test]
fn the_workbook_has_a_sheet_for_each_table_and_real_numbers() {
    let file = render_register(RegisterFormat::Xlsx, &register()).unwrap();
    assert_eq!(
        file.content_type,
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
    );
    assert!(file.bytes.starts_with(b"PK"));
    let files = entries(file.bytes);
    let sheets = files
        .iter()
        .filter(|(name, _)| name.starts_with("xl/worksheets/sheet"))
        .count();
    assert_eq!(sheets, 6);
    let workbook = file_named(&files, "xl/workbook.xml");
    for sheet in [
        "Register",
        "Lines",
        "VAT by category",
        "Stated totals",
        "To check",
        "Manifest",
    ] {
        assert!(workbook.contains(sheet), "{sheet} is missing from the workbook");
    }
    let all: String = files.into_iter().map(|(_, text)| text).collect();
    assert!(all.contains("INV-1"));
    assert!(all.contains("Consulting"));
    // Every row is traceable back to the document it came from.
    assert!(all.contains("Document id"));
    assert!(all.contains("invoice.pdf"));
    // R1 500.00 is stored as the number 1500, from exact minor units, not as text.
    assert!(all.contains("<v>1500</v>"), "the amount should be a number");
}

#[test]
fn the_bundle_holds_a_file_for_every_table_and_the_manifest() {
    let file = render_register(RegisterFormat::CsvBundle, &register()).unwrap();
    assert_eq!(file.extension, "zip");
    assert_eq!(file.content_type, "application/zip");
    let files = entries(file.bytes);
    for name in [
        "register.csv",
        "lines.csv",
        "vat-by-category.csv",
        "stated-totals.csv",
        "reconciliation.csv",
        "to-check.csv",
        "manifest.json",
    ] {
        assert!(
            files.iter().any(|(entry, _)| entry == name),
            "{name} is missing"
        );
    }
    let documents = file_named(&files, "register.csv");
    assert!(documents.starts_with("document_id,filename,"));
    assert!(documents.contains("invoice.pdf"));
    let manifest = file_named(&files, "manifest.json");
    assert!(serde_json::from_str::<serde_json::Value>(&manifest).is_ok());
}

#[test]
fn text_in_the_bundle_cannot_become_a_formula() {
    let mut register = register();
    register.rows[0].supplier = Some("=HYPERLINK(\"http://example.com\")".into());
    let file = render_register(RegisterFormat::CsvBundle, &register).unwrap();
    let documents = file_named(&entries(file.bytes), "register.csv");
    assert!(documents.contains("'=HYPERLINK"));
    assert!(!documents.contains(",=HYPERLINK"));
}

#[test]
fn the_register_pdf_is_a_pdf() {
    let file = render_register(RegisterFormat::Pdf, &register()).unwrap();
    assert_eq!(file.content_type, "application/pdf");
    assert!(file.bytes.starts_with(b"%PDF-"));
    assert!(file.bytes.windows(5).any(|w| w == b"%%EOF"));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p clearcut-export register`

Expected: FAIL, `no variant or associated item named 'Xlsx' found for enum 'RegisterFormat'` and `no method named 'name' found for enum 'RegisterFormat'`.

- [ ] **Step 3: Make the workbook and PDF helpers reachable, and give the PDF one way to finish**

In `crates/clearcut-export/src/xlsx.rs`, change:

```rust
fn units(amount: &Amount) -> f64 {
```

to

```rust
pub(crate) fn units(amount: &Amount) -> f64 {
```

and change:

```rust
fn head(sheet: &mut Worksheet, bold: &Format, titles: &[&str], widths: &[f64]) -> Result<(), XlsxError> {
```

to

```rust
pub(crate) fn head(
    sheet: &mut Worksheet,
    bold: &Format,
    titles: &[&str],
    widths: &[f64],
) -> Result<(), XlsxError> {
```

In `crates/clearcut-export/src/pdf.rs`, change `struct Layout {` to `pub(crate) struct Layout {` and `const BODY: Style =` to `pub(crate) const BODY: Style =`. Inside `impl Layout`, change the signatures of `new`, `paragraph`, `heading` and `table` from `fn` to `pub(crate) fn` (their bodies stay exactly as they are).

Then, after the closing brace of `table` and still inside `impl Layout`, add:

```rust
    /// The finished file: the notice and page numbers at the foot of every page, then the document.
    pub(crate) fn finish(mut self, title: &str, notice: &str) -> Result<Vec<u8>, PdfError> {
        let total = self.pages.len();
        let small = Style {
            size: 7.0,
            bold: false,
            grey: true,
        };
        for (i, ops) in self.pages.iter_mut().enumerate() {
            let lines = wrap(notice, small.size, false, CONTENT_W - 60.0);
            for (n, line) in lines.iter().enumerate() {
                Layout::text_at(
                    ops,
                    MARGIN,
                    MARGIN - 4.0 + 9.0 * (lines.len() - 1 - n) as f32,
                    small,
                    line,
                );
            }
            let label = format!("Page {} of {}", i + 1, total);
            Layout::text_at(
                ops,
                PAGE_W - MARGIN - width(&label, small.size, false),
                MARGIN - 4.0,
                small,
                &label,
            );
        }

        let mut doc = PdfDocument::new(&clean(title));
        let pages: Vec<PdfPage> = self
            .pages
            .into_iter()
            .map(|ops| PdfPage::new(Mm(210.0), Mm(297.0), ops))
            .collect();
        let mut warnings = Vec::new();
        let bytes = doc
            .with_pages(pages)
            .save(&PdfSaveOptions::default(), &mut warnings);
        if bytes.is_empty() {
            return Err(PdfError);
        }
        Ok(bytes)
    }
```

Finally, in `pub fn pdf(model: &ExportModel)`, replace everything from the comment `// The footer goes on once the number of pages is known.` to the end of the function with:

```rust
    layout.finish(&model.document.filename, model.notice)
}
```

The report's PDF is now built exactly as before, through the shared `finish`.

- [ ] **Step 4: Write the workbook**

Create `crates/clearcut-export/src/register/xlsx.rs`:

```rust
// Written by Kone Tshivhinda & Claude
//! The register workbook: Register, Lines, VAT by category, Stated totals, To check and Manifest.
//!
//! The reconciliation sits beneath the VAT table, because it is those figures set against what the
//! documents state.
//!
//! Amounts are real numeric cells built from the exact minor units, so a person can add them up in
//! the spreadsheet. Text that came off a document goes in with `write_string`, which stores plain
//! text and never a formula.

use crate::xlsx::{head, units};
use clearcut_core::export::Amount;
use clearcut_core::narration::format::{rate_bps, tax_category};
use clearcut_core::register::Register;
use rust_xlsxwriter::{Format, Workbook, Worksheet, XlsxError};

fn date_text(date: Option<time::Date>) -> String {
    date.map(|date| date.to_string()).unwrap_or_default()
}

fn amount_text(amount: Option<&Amount>) -> String {
    amount.map(|amount| amount.text.clone()).unwrap_or_default()
}

/// A money cell, left empty when the document stated no such amount.
fn money(sheet: &mut Worksheet, row: u32, col: u16, amount: Option<&Amount>, format: &Format) -> Result<(), XlsxError> {
    if let Some(amount) = amount {
        sheet.write_number_with_format(row, col, units(amount), format)?;
    }
    Ok(())
}

pub fn xlsx(register: &Register) -> Result<Vec<u8>, XlsxError> {
    let bold = Format::new().set_bold();
    let title = Format::new().set_bold().set_font_size(14);
    let money_format = Format::new().set_num_format("#,##0.00");
    let percent = Format::new().set_num_format("0.0%");
    let wrap = Format::new().set_text_wrap();

    let mut book = Workbook::new();

    // ---- Register (the documents, one row each)
    {
        let sheet = book.add_worksheet();
        sheet.set_name("Register")?;
        head(
            sheet,
            &bold,
            &[
                "Date",
                "Supplier",
                "Supplier VAT no",
                "Number",
                "Kind",
                "Status",
                "Currency",
                "Net",
                "VAT",
                "Gross",
                "Lines",
                "Pages",
                "Review",
                "Duplicate of",
                "Read by",
                "Flags",
                "Document id",
                "File",
            ],
            &[
                11.0, 30.0, 16.0, 14.0, 12.0, 14.0, 9.0, 13.0, 13.0, 13.0, 6.0, 6.0, 40.0, 30.0,
                20.0, 40.0, 38.0, 28.0,
            ],
        )?;
        for (i, row) in register.rows.iter().enumerate() {
            let at = i as u32 + 1;
            sheet.write_string(at, 0, date_text(row.invoice_date).as_str())?;
            sheet.write_string_with_format(at, 1, row.supplier.as_deref().unwrap_or(""), &wrap)?;
            sheet.write_string(at, 2, row.supplier_vat_number.as_deref().unwrap_or(""))?;
            sheet.write_string(at, 3, row.invoice_number.as_deref().unwrap_or(""))?;
            sheet.write_string(at, 4, row.kind.label())?;
            sheet.write_string(at, 5, row.status.as_str())?;
            sheet.write_string(at, 6, row.currency.as_deref().unwrap_or(""))?;
            money(sheet, at, 7, row.net.as_ref(), &money_format)?;
            money(sheet, at, 8, row.vat.as_ref(), &money_format)?;
            money(sheet, at, 9, row.gross.as_ref(), &money_format)?;
            sheet.write_number(at, 10, row.line_count as f64)?;
            if let Some(pages) = row.page_count {
                sheet.write_number(at, 11, f64::from(pages))?;
            }
            sheet.write_string_with_format(at, 12, row.review_reasons.join("; ").as_str(), &wrap)?;
            sheet.write_string(at, 13, row.duplicate_of.as_deref().unwrap_or(""))?;
            sheet.write_string(at, 14, row.read_by.join(", ").as_str())?;
            sheet.write_string_with_format(at, 15, row.flags.join("; ").as_str(), &wrap)?;
            sheet.write_string(at, 16, row.id.as_str())?;
            sheet.write_string(at, 17, row.filename.as_str())?;
        }
    }

    // ---- Lines
    {
        let sheet = book.add_worksheet();
        sheet.set_name("Lines")?;
        head(
            sheet,
            &bold,
            &[
                "Date",
                "Supplier",
                "Number",
                "No",
                "Description",
                "Quantity",
                "Unit price",
                "Line total",
                "Tax",
                "Page",
                "Document id",
            ],
            &[10.0, 28.0, 13.0, 5.0, 50.0, 10.0, 14.0, 14.0, 16.0, 6.0, 38.0],
        )?;
        for (i, line) in register.lines.iter().enumerate() {
            let at = i as u32 + 1;
            sheet.write_string(at, 0, date_text(line.invoice_date).as_str())?;
            sheet.write_string(at, 1, line.supplier.as_deref().unwrap_or(""))?;
            sheet.write_string(at, 2, line.invoice_number.as_deref().unwrap_or(""))?;
            sheet.write_number(at, 3, f64::from(line.number))?;
            sheet.write_string_with_format(at, 4, line.description.as_str(), &wrap)?;
            match line.quantity.parse::<f64>() {
                Ok(quantity) => sheet.write_number(at, 5, quantity)?,
                Err(_) => sheet.write_string(at, 5, line.quantity.as_str())?,
            };
            sheet.write_number_with_format(at, 6, units(&line.unit_price), &money_format)?;
            sheet.write_number_with_format(at, 7, units(&line.line_total), &money_format)?;
            sheet.write_string(at, 8, tax_category(line.tax).as_str())?;
            sheet.write_number(at, 9, f64::from(line.page))?;
            sheet.write_string(at, 10, line.document_id.as_str())?;
        }
    }

    // ---- VAT by category, with the reconciliation beneath it
    {
        let sheet = book.add_worksheet();
        sheet.set_name("VAT by category")?;
        head(
            sheet,
            &bold,
            &[
                "Month",
                "Currency",
                "Kind",
                "Category",
                "Rate",
                "Side",
                "Documents",
                "Taxable",
                "VAT",
            ],
            &[11.0, 9.0, 12.0, 16.0, 8.0, 16.0, 11.0, 16.0, 16.0],
        )?;
        let mut at = 1u32;
        for group in &register.vat_by_category {
            for (side, part) in [
                ("Ready", group.ready.as_ref()),
                ("Awaiting review", group.awaiting.as_ref()),
            ] {
                let Some(part) = part else {
                    continue;
                };
                sheet.write_string(at, 0, group.month.as_str())?;
                sheet.write_string(at, 1, group.currency.as_str())?;
                sheet.write_string(at, 2, group.kind.label())?;
                sheet.write_string(at, 3, tax_category(group.category).as_str())?;
                sheet.write_number_with_format(
                    at,
                    4,
                    f64::from(group.rate_bps) / 10_000.0,
                    &percent,
                )?;
                sheet.write_string(at, 5, side)?;
                sheet.write_number(at, 6, part.documents as f64)?;
                sheet.write_number_with_format(at, 7, units(&part.taxable), &money_format)?;
                sheet.write_number_with_format(at, 8, units(&part.vat), &money_format)?;
                at += 1;
            }
        }

        // Calculated VAT beside what the documents state, with the difference shown, never smoothed.
        at += 2;
        for (col, heading) in [
            "Month",
            "Currency",
            "Kind",
            "Calculated VAT",
            "Stated VAT",
            "Difference",
            "Documents that disagree",
        ]
        .iter()
        .enumerate()
        {
            sheet.write_string_with_format(at, col as u16, *heading, &bold)?;
        }
        at += 1;
        for row in &register.reconciliation {
            sheet.write_string(at, 0, row.month.as_str())?;
            sheet.write_string(at, 1, row.currency.as_str())?;
            sheet.write_string(at, 2, row.kind.label())?;
            sheet.write_number_with_format(at, 3, units(&row.calculated_vat), &money_format)?;
            sheet.write_number_with_format(at, 4, units(&row.stated_vat), &money_format)?;
            sheet.write_number_with_format(at, 5, units(&row.difference), &money_format)?;
            sheet.write_number(at, 6, row.documents_disagree as f64)?;
            at += 1;
        }
    }

    // ---- Stated totals
    {
        let sheet = book.add_worksheet();
        sheet.set_name("Stated totals")?;
        head(
            sheet,
            &bold,
            &[
                "Month",
                "Currency",
                "Kind",
                "Side",
                "Documents",
                "Net",
                "VAT",
                "Gross",
            ],
            &[11.0, 9.0, 12.0, 16.0, 11.0, 16.0, 16.0, 16.0],
        )?;
        let mut at = 1u32;
        for group in &register.stated_totals {
            for (side, part) in [
                ("Ready", group.ready.as_ref()),
                ("Awaiting review", group.awaiting.as_ref()),
            ] {
                let Some(part) = part else {
                    continue;
                };
                sheet.write_string(at, 0, group.month.as_str())?;
                sheet.write_string(at, 1, group.currency.as_str())?;
                sheet.write_string(at, 2, group.kind.label())?;
                sheet.write_string(at, 3, side)?;
                sheet.write_number(at, 4, part.documents as f64)?;
                sheet.write_number_with_format(at, 5, units(&part.net), &money_format)?;
                sheet.write_number_with_format(at, 6, units(&part.vat), &money_format)?;
                sheet.write_number_with_format(at, 7, units(&part.gross), &money_format)?;
                at += 1;
            }
        }
    }

    // ---- To check
    {
        let sheet = book.add_worksheet();
        sheet.set_name("To check")?;
        head(
            sheet,
            &bold,
            &["Document", "Reason", "Page", "What to look at"],
            &[34.0, 24.0, 6.0, 70.0],
        )?;
        for (i, row) in register.to_check.iter().enumerate() {
            let at = i as u32 + 1;
            sheet.write_string(at, 0, row.filename.as_str())?;
            sheet.write_string(at, 1, row.reason.as_str())?;
            if let Some(page) = row.page {
                sheet.write_number(at, 2, f64::from(page))?;
            }
            sheet.write_string_with_format(at, 3, row.sentence.as_str(), &wrap)?;
        }
    }

    // ---- Manifest: exactly what went into the register
    {
        let sheet = book.add_worksheet();
        sheet.set_name("Manifest")?;
        sheet.set_column_width(0, 38)?;
        sheet.set_column_width(1, 30)?;
        sheet.write_string_with_format(0, 0, "Register", &title)?;
        let selection = register.selection.label();
        sheet.write_string(1, 0, "Selection")?;
        sheet.write_string(1, 1, selection.as_str())?;
        sheet.write_string(2, 0, "Made by")?;
        sheet.write_string(2, 1, format!("ClearCut {}", register.version).as_str())?;

        let manifest = &register.manifest;
        let facts = [
            ("Documents in the register", manifest.documents.to_string()),
            ("Finished", manifest.finished.to_string()),
            ("Finished with nobody needed", manifest.done.to_string()),
            ("Awaiting review", manifest.needs_review.to_string()),
            ("Still reading", manifest.still_reading.to_string()),
            ("Left out, awaiting review", manifest.awaiting_left_out.to_string()),
            ("Line items", manifest.lines.to_string()),
            ("No invoice date in the account", manifest.undated_in_tenant.to_string()),
            ("No invoice date in the selection", manifest.undated_in_selection.to_string()),
            (
                "Automation",
                manifest
                    .automation_rate_bps
                    .map(rate_bps)
                    .unwrap_or_else(|| "no finished documents".to_string()),
            ),
            (
                "Only ready documents",
                if manifest.only_ready {
                    "yes".to_string()
                } else {
                    "no".to_string()
                },
            ),
        ];
        for (i, (label, value)) in facts.iter().enumerate() {
            let row = 4 + i as u32;
            sheet.write_string(row, 0, *label)?;
            sheet.write_string(row, 1, value.as_str())?;
        }
        let notice_row = 4 + facts.len() as u32 + 1;
        sheet.write_string_with_format(notice_row, 0, register.notice, &wrap)?;
    }

    book.save_to_buffer()
}
```

- [ ] **Step 5: Write the CSV bundle**

Create `crates/clearcut-export/src/register/csv.rs`:

```rust
// Written by Kone Tshivhinda & Claude
//! The register as a zip of plain CSV files, one per table, plus the manifest as JSON.
//!
//! Every cell goes through the same quoting as the report's CSV: a cell a spreadsheet would run
//! as a formula is prefixed with an apostrophe, so text read off a document stays text.

use crate::text::cell;
use clearcut_core::export::Amount;
use clearcut_core::narration::format::{rate_bps, tax_category};
use clearcut_core::register::Register;
use std::io::{Cursor, Write};
use zip::result::ZipError;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

fn date_text(date: Option<time::Date>) -> String {
    date.map(|date| date.to_string()).unwrap_or_default()
}

fn amount_text(amount: Option<&Amount>) -> String {
    amount.map(|amount| amount.text.clone()).unwrap_or_default()
}

fn csv(header: &str, rows: &[Vec<String>]) -> String {
    let mut out = String::from(header);
    out.push('\n');
    for row in rows {
        out.push_str(&row.iter().map(|text| cell(text)).collect::<Vec<_>>().join(","));
        out.push('\n');
    }
    out
}

/// The bundle: one file for every table a bookkeeper works from, and the exact JSON beside them.
pub fn bundle(register: &Register) -> Result<Vec<u8>, ZipError> {
    let documents: Vec<Vec<String>> = register
        .rows
        .iter()
        .map(|row| {
            vec![
                row.id.clone(),
                row.filename.clone(),
                date_text(row.invoice_date),
                row.supplier.clone().unwrap_or_default(),
                row.supplier_vat_number.clone().unwrap_or_default(),
                row.invoice_number.clone().unwrap_or_default(),
                row.kind.label().to_string(),
                row.status.as_str().to_string(),
                row.currency.clone().unwrap_or_default(),
                amount_text(row.net.as_ref()),
                amount_text(row.vat.as_ref()),
                amount_text(row.gross.as_ref()),
                row.line_count.to_string(),
                row.page_count.map(|pages| pages.to_string()).unwrap_or_default(),
                row.review_reasons.join("; "),
                row.duplicate_of.clone().unwrap_or_default(),
                row.read_by.join(", "),
                row.flags.join("; "),
            ]
        })
        .collect();

    let lines: Vec<Vec<String>> = register
        .lines
        .iter()
        .map(|line| {
            vec![
                line.document_id.clone(),
                date_text(line.invoice_date),
                line.supplier.clone().unwrap_or_default(),
                line.invoice_number.clone().unwrap_or_default(),
                line.number.to_string(),
                line.description.clone(),
                line.quantity.clone(),
                line.unit_price.text.clone(),
                line.line_total.text.clone(),
                tax_category(line.tax),
                line.page.to_string(),
            ]
        })
        .collect();

    let mut vat = Vec::new();
    for group in &register.vat_by_category {
        for (side, part) in [
            ("ready", group.ready.as_ref()),
            ("awaiting review", group.awaiting.as_ref()),
        ] {
            let Some(part) = part else {
                continue;
            };
            vat.push(vec![
                group.month.clone(),
                group.currency.clone(),
                group.kind.label().to_string(),
                tax_category(group.category),
                rate_bps(group.rate_bps),
                side.to_string(),
                part.documents.to_string(),
                part.taxable.text.clone(),
                part.vat.text.clone(),
            ]);
        }
    }

    let mut stated = Vec::new();
    for group in &register.stated_totals {
        for (side, part) in [
            ("ready", group.ready.as_ref()),
            ("awaiting review", group.awaiting.as_ref()),
        ] {
            let Some(part) = part else {
                continue;
            };
            stated.push(vec![
                group.month.clone(),
                group.currency.clone(),
                group.kind.label().to_string(),
                side.to_string(),
                part.documents.to_string(),
                part.net.text.clone(),
                part.vat.text.clone(),
                part.gross.text.clone(),
            ]);
        }
    }

    let reconciliation: Vec<Vec<String>> = register
        .reconciliation
        .iter()
        .map(|row| {
            vec![
                row.month.clone(),
                row.currency.clone(),
                row.kind.label().to_string(),
                row.calculated_vat.text.clone(),
                row.stated_vat.text.clone(),
                row.difference.text.clone(),
                row.documents_disagree.to_string(),
            ]
        })
        .collect();

    let to_check: Vec<Vec<String>> = register
        .to_check
        .iter()
        .map(|row| {
            vec![
                row.document_id.clone(),
                row.filename.clone(),
                row.reason.clone(),
                row.page.map(|page| page.to_string()).unwrap_or_default(),
                row.sentence.clone(),
            ]
        })
        .collect();

    let files = [
        (
            "register.csv",
            csv(
                "document_id,filename,date,supplier,supplier_vat_number,invoice_number,kind,status,currency,net,vat,gross,lines,pages,review,duplicate_of,read_by,flags",
                &documents,
            ),
        ),
        (
            "lines.csv",
            csv(
                "document_id,date,supplier,invoice_number,no,description,quantity,unit_price,line_total,tax,page",
                &lines,
            ),
        ),
        (
            "vat-by-category.csv",
            csv(
                "month,currency,kind,category,rate,side,documents,taxable,vat",
                &vat,
            ),
        ),
        (
            "stated-totals.csv",
            csv("month,currency,kind,side,documents,net,vat,gross", &stated),
        ),
        (
            "reconciliation.csv",
            csv(
                "month,currency,kind,calculated_vat,stated_vat,difference,documents_disagree",
                &reconciliation,
            ),
        ),
        (
            "to-check.csv",
            csv("document_id,filename,reason,page,sentence", &to_check),
        ),
        (
            "manifest.json",
            serde_json::to_string_pretty(&register.manifest).unwrap_or_default(),
        ),
    ];

    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, text) in files {
        writer.start_file(name, options)?;
        writer.write_all(text.as_bytes())?;
    }
    let cursor = writer.finish()?;
    Ok(cursor.into_inner())
}
```

- [ ] **Step 6: Write the PDF**

Create `crates/clearcut-export/src/register/pdf.rs`:

```rust
// Written by Kone Tshivhinda & Claude
//! The register as a PDF: the tables a finance team reads, with nothing else in the way.

use crate::pdf::{Layout, PdfError, BODY};
use clearcut_core::export::Amount;
use clearcut_core::narration::format::{rate_bps, tax_category};
use clearcut_core::register::Register;

fn date_text(date: Option<time::Date>) -> String {
    date.map(|date| date.to_string()).unwrap_or_default()
}

fn amount_text(amount: Option<&Amount>) -> String {
    amount.map(|amount| amount.text.clone()).unwrap_or_default()
}

fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

pub fn pdf(register: &Register) -> Result<Vec<u8>, PdfError> {
    let mut layout = Layout::new();
    let title = format!("Register {}", register.selection.label());
    layout.heading(&title, 18.0);
    layout.paragraph(
        &format!(
            "{}, {}. Made by ClearCut {}.",
            count(register.manifest.documents, "document", "documents"),
            count(register.manifest.lines, "line item", "line items"),
            register.version
        ),
        BODY,
        0.0,
    );

    layout.heading("Documents", 12.0);
    let rows: Vec<Vec<String>> = register
        .rows
        .iter()
        .map(|row| {
            vec![
                date_text(row.invoice_date),
                row.supplier.clone().unwrap_or_default(),
                row.invoice_number.clone().unwrap_or_default(),
                row.kind.label().to_string(),
                row.status.as_str().to_string(),
                amount_text(row.net.as_ref()),
                amount_text(row.vat.as_ref()),
                amount_text(row.gross.as_ref()),
                row.line_count.to_string(),
            ]
        })
        .collect();
    layout.table(
        &[
            "Date", "Supplier", "Number", "Kind", "Status", "Net", "VAT", "Gross", "Lines",
        ],
        &[44.0, 118.0, 58.0, 48.0, 52.0, 50.0, 50.0, 53.0, 20.0],
        &[false, false, false, false, false, true, true, true, true],
        &rows,
        8.0,
    );

    // A page cannot usefully carry fifty thousand lines: the workbook and the bundle do.
    if !register.lines.is_empty() {
        layout.paragraph(
            &format!(
                "The {} line items are in the workbook and the CSV bundle; this page carries the \
                 register, the totals and what needs a person.",
                register.lines.len()
            ),
            BODY,
            0.0,
        );
    }

    if !register.vat_by_category.is_empty() {
        layout.heading("VAT", 12.0);
        let mut rows = Vec::new();
        for group in &register.vat_by_category {
            for (side, part) in [
                ("Ready", group.ready.as_ref()),
                ("Awaiting review", group.awaiting.as_ref()),
            ] {
                let Some(part) = part else {
                    continue;
                };
                rows.push(vec![
                    group.month.clone(),
                    group.currency.clone(),
                    group.kind.label().to_string(),
                    tax_category(group.category),
                    rate_bps(group.rate_bps),
                    side.to_string(),
                    part.documents.to_string(),
                    part.taxable.text.clone(),
                    part.vat.text.clone(),
                ]);
            }
        }
        layout.table(
            &[
                "Month",
                "Currency",
                "Kind",
                "Category",
                "Rate",
                "Side",
                "Documents",
                "Taxable",
                "VAT",
            ],
            &[40.0, 32.0, 40.0, 50.0, 30.0, 54.0, 26.0, 74.0, 72.0],
            &[false, false, false, false, true, false, true, true, true],
            &rows,
            8.0,
        );
    }

    if !register.stated_totals.is_empty() {
        layout.heading("Stated totals", 12.0);
        let mut rows = Vec::new();
        for group in &register.stated_totals {
            for (side, part) in [
                ("Ready", group.ready.as_ref()),
                ("Awaiting review", group.awaiting.as_ref()),
            ] {
                let Some(part) = part else {
                    continue;
                };
                rows.push(vec![
                    group.month.clone(),
                    group.currency.clone(),
                    group.kind.label().to_string(),
                    side.to_string(),
                    part.documents.to_string(),
                    part.net.text.clone(),
                    part.vat.text.clone(),
                    part.gross.text.clone(),
                ]);
            }
        }
        layout.table(
            &[
                "Month",
                "Currency",
                "Kind",
                "Side",
                "Documents",
                "Net",
                "VAT",
                "Gross",
            ],
            &[40.0, 32.0, 40.0, 54.0, 26.0, 74.0, 72.0, 72.0],
            &[false, false, false, false, true, true, true, true],
            &rows,
            8.0,
        );
    }

    if !register.reconciliation.is_empty() {
        layout.heading("Reconciliation", 12.0);
        let rows: Vec<Vec<String>> = register
            .reconciliation
            .iter()
            .map(|row| {
                vec![
                    row.month.clone(),
                    row.currency.clone(),
                    row.kind.label().to_string(),
                    row.calculated_vat.text.clone(),
                    row.stated_vat.text.clone(),
                    row.difference.text.clone(),
                    row.documents_disagree.to_string(),
                ]
            })
            .collect();
        layout.table(
            &[
                "Month",
                "Currency",
                "Kind",
                "Calculated VAT",
                "Stated VAT",
                "Difference",
                "Disagree",
            ],
            &[40.0, 32.0, 40.0, 76.0, 76.0, 76.0, 40.0],
            &[false, false, false, true, true, true, true],
            &rows,
            8.0,
        );
    }

    if !register.to_check.is_empty() {
        layout.heading("To check", 12.0);
        let rows: Vec<Vec<String>> = register
            .to_check
            .iter()
            .map(|row| {
                vec![
                    row.filename.clone(),
                    row.reason.clone(),
                    row.page.map(|page| page.to_string()).unwrap_or_default(),
                    row.sentence.clone(),
                ]
            })
            .collect();
        layout.table(
            &["Document", "Reason", "Page", "What to look at"],
            &[110.0, 96.0, 26.0, 240.0],
            &[false, false, true, false],
            &rows,
            8.0,
        );
    }

    layout.finish(&title, register.notice)
}
```

- [ ] **Step 7: Extend the format enum and the renderer, and put `zip` in the normal dependencies**

In `crates/clearcut-export/Cargo.toml`, add `zip = "9.1.2"` after `time = "0.3.55"` under `[dependencies]`, and remove `zip = "9.1.2"` from `[dev-dependencies]`, which keeps only the `time` line with the `macros` feature. The bundle is written at runtime now, not only in tests.

Replace `crates/clearcut-export/src/register/mod.rs` with:

```rust
// Written by Kone Tshivhinda & Claude
//! The register files: one value, laid out in the shapes a bookkeeper or finance team can use.
//!
//! Every file is laid out from **one** [`Register`], built in `clearcut_core::register`, so the
//! formats cannot disagree with each other.

mod csv;
mod pdf;
mod text;
mod xlsx;

use crate::{ExportError, Rendered};
use clearcut_core::register::Register;

/// The formats a register can be downloaded as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegisterFormat {
    Json,
    Html,
    Markdown,
    Text,
    Xlsx,
    CsvBundle,
    Pdf,
}

impl RegisterFormat {
    pub const ALL: [RegisterFormat; 7] = [
        RegisterFormat::Json,
        RegisterFormat::Html,
        RegisterFormat::Markdown,
        RegisterFormat::Text,
        RegisterFormat::Xlsx,
        RegisterFormat::CsvBundle,
        RegisterFormat::Pdf,
    ];

    /// The name used in the API: `json`, `html`, `md`, `txt`, `xlsx`, `csv`, `pdf`.
    ///
    /// The CSV bundle is named for what it holds; it saves as `.zip` because that is what it is.
    pub fn name(self) -> &'static str {
        match self {
            RegisterFormat::Json => "json",
            RegisterFormat::Html => "html",
            RegisterFormat::Markdown => "md",
            RegisterFormat::Text => "txt",
            RegisterFormat::Xlsx => "xlsx",
            RegisterFormat::CsvBundle => "csv",
            RegisterFormat::Pdf => "pdf",
        }
    }

    pub fn parse(name: &str) -> Option<RegisterFormat> {
        RegisterFormat::ALL
            .into_iter()
            .find(|format| format.name() == name)
    }

    pub fn extension(self) -> &'static str {
        match self {
            RegisterFormat::Json => "json",
            RegisterFormat::Html => "html",
            RegisterFormat::Markdown => "md",
            RegisterFormat::Text => "txt",
            RegisterFormat::Xlsx => "xlsx",
            RegisterFormat::CsvBundle => "zip",
            RegisterFormat::Pdf => "pdf",
        }
    }

    pub fn content_type(self) -> &'static str {
        match self {
            RegisterFormat::Json => "application/json",
            RegisterFormat::Html => "text/html; charset=utf-8",
            RegisterFormat::Markdown => "text/markdown; charset=utf-8",
            RegisterFormat::Text => "text/plain; charset=utf-8",
            RegisterFormat::Xlsx => {
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
            }
            RegisterFormat::CsvBundle => "application/zip",
            RegisterFormat::Pdf => "application/pdf",
        }
    }
}

/// Makes the register file.
pub fn render_register(format: RegisterFormat, register: &Register) -> Result<Rendered, ExportError> {
    let bytes = match format {
        RegisterFormat::Json => text::json(register).into_bytes(),
        RegisterFormat::Html => text::html(register).into_bytes(),
        RegisterFormat::Markdown => text::markdown(register).into_bytes(),
        RegisterFormat::Text => text::plain(register).into_bytes(),
        RegisterFormat::Xlsx => xlsx::xlsx(register)
            .map_err(|_| ExportError::Failed("register workbook"))?,
        RegisterFormat::CsvBundle => csv::bundle(register)
            .map_err(|_| ExportError::Failed("register CSV bundle"))?,
        RegisterFormat::Pdf => pdf::pdf(register).map_err(|_| ExportError::Failed("register PDF"))?,
    };
    Ok(Rendered {
        content_type: format.content_type(),
        extension: format.extension(),
        bytes,
    })
}

#[cfg(test)]
mod tests;
```

- [ ] **Step 8: Run the tests to verify they pass**

Run: `cargo test -p clearcut-export register`

Expected: PASS, 7 tests: `every_register_format_makes_a_file_of_the_right_kind`, `text_from_the_document_cannot_become_markup`, `the_same_figures_are_in_every_format`, `the_workbook_has_a_sheet_for_each_table_and_real_numbers`, `the_bundle_holds_a_file_for_every_table_and_the_manifest`, `text_in_the_bundle_cannot_become_a_formula`, `the_register_pdf_is_a_pdf`.

Run: `cargo test -p clearcut-export`

Expected: PASS. The report's own tests (including the PDF and the workbook) prove the shared `finish` and the wider `units` and `head` changed nothing.

- [ ] **Step 9: Commit**

```bash
git add crates/clearcut-export/Cargo.toml Cargo.lock crates/clearcut-export/src/xlsx.rs crates/clearcut-export/src/pdf.rs crates/clearcut-export/src/register
git commit -m "feat(export): lay the register out as a workbook, a CSV bundle and a PDF"
```

### Task 7: `POST /v1/register`

**Files:**
- Create: `crates/clearcut-server/src/register.rs`
- Create: `crates/clearcut-server/src/register/tests.rs`
- Modify: `crates/clearcut-server/src/lib.rs` (`mod register;` and the route)
- Modify: `crates/clearcut-server/src/results/mod.rs` (`Results::register`, its `Store` implementation, and `InvalidSelectionId` as a `400`)
- Modify: `crates/clearcut-server/src/results/tests.rs` (the existing fake gains the new method)
- Modify: `crates/clearcut-server/src/openapi.rs` (the `/v1/register` path)
- Modify: `contracts/openapi.json` (regenerated)

**Interfaces:**
- Consumes: `Selection`, `RegisterInput`, `build`, `parse_date`, `RegisterError` and `MAX_DOCUMENTS` from `clearcut_core::register`; `Store::register_sources` through `Results`; `RegisterFormat` and `render_register` from `clearcut_export`; `ResultsProblem` and `AppState::results`.
- Produces: `POST /v1/register` answered by `pub(crate) async fn register::make_register`; `Results::register(&self, tenant, &Selection, only_ready) -> ResultsFuture<RegisterInput>`; `Error::InvalidSelectionId` answered as `400`.

- [ ] **Step 1: Write the failing tests**

Create `crates/clearcut-server/src/register/tests.rs`:

```rust
// Written by Kone Tshivhinda & Claude
//! The HTTP rules for the register, against a fake store (no database).

use super::*;
use crate::results::{Results, ResultsFuture};
use crate::tests::{bearer, json, send, FakeAuth, ADMIN_ONLY, READER};
use crate::{finish, routes, AppState};
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use clearcut_core::document::DocumentStatus;
use clearcut_core::event::{
    Citation, Confidence, DocumentId, Event, EventId, EventParts, EventPayload, Extractor,
};
use clearcut_core::register::{RegisterFacts, RegisterInput, Selection, MAX_DOCUMENTS};
use clearcut_store::{DocId, DocumentInfo, DocumentSummary, Error, SearchHit, TenantId};
use std::num::NonZeroU32;
use std::sync::Arc;
use time::OffsetDateTime;

const ONE: &str = "00000000-0000-0000-0000-000000000001";

/// What the store would hand in: a fixed number of finished documents, or the error a named id
/// that is not a UUID produces.
struct Fake {
    facts: usize,
    invalid_id: bool,
}

impl Fake {
    fn fact(&self, n: usize) -> RegisterFacts {
        RegisterFacts {
            id: format!("00000000-0000-0000-0000-{n:012}"),
            filename: format!("invoice-{n}.pdf"),
            uploaded_at: OffsetDateTime::UNIX_EPOCH,
            status: DocumentStatus::Done,
            page_count: Some(1),
            pages: Vec::new(),
            events: vec![Event::new(EventParts {
                id: EventId::new(&format!("doc_{n}:r1:supplier")),
                date: None,
                payload: EventPayload::SupplierIdentified {
                    name: format!("Acme {n} (Pty) Ltd"),
                    vat_number: None,
                },
                citation: Citation {
                    document: DocumentId::new(&format!("doc_{n}")),
                    page: NonZeroU32::new(1).unwrap(),
                    locator: None,
                },
                extractor: Extractor::System,
                confidence: Confidence::new(950).unwrap(),
                supersedes: None,
            })
            .unwrap()],
        }
    }
}

impl Results for Arc<Fake> {
    fn list<'a>(
        &'a self,
        _tenant: TenantId,
        _limit: u32,
    ) -> ResultsFuture<'a, Vec<DocumentSummary>> {
        Box::pin(async { unreachable!("the register has its own tests") })
    }

    fn info<'a>(&'a self, _tenant: TenantId, _document: DocId) -> ResultsFuture<'a, DocumentInfo> {
        Box::pin(async { unreachable!("the register has its own tests") })
    }

    fn events<'a>(&'a self, _tenant: TenantId, _document: DocId) -> ResultsFuture<'a, Vec<Event>> {
        Box::pin(async { unreachable!("the register has its own tests") })
    }

    fn search<'a>(
        &'a self,
        _tenant: TenantId,
        _query: &'a str,
        _limit: u32,
    ) -> ResultsFuture<'a, Vec<SearchHit>> {
        Box::pin(async { unreachable!("the register has its own tests") })
    }

    fn register<'a>(
        &'a self,
        _tenant: TenantId,
        selection: &'a Selection,
        only_ready: bool,
    ) -> ResultsFuture<'a, RegisterInput> {
        Box::pin(async move {
            if self.invalid_id {
                return Err(Error::InvalidSelectionId("not-a-uuid".into()));
            }
            Ok(RegisterInput {
                selection: selection.clone(),
                only_ready,
                undated_in_tenant: 0,
                facts: (0..self.facts).map(|n| self.fact(n)).collect(),
            })
        })
    }
}

fn app(fake: &Arc<Fake>) -> Router {
    finish(
        routes(),
        AppState::shared(FakeAuth::new()).with_results(fake.clone()),
    )
}

fn fake(facts: usize, invalid_id: bool) -> Arc<Fake> {
    Arc::new(Fake { facts, invalid_id })
}

fn post(token: Option<&str>, body: &str) -> Request<Body> {
    let mut builder =
        Request::post("/v1/register").header(header::CONTENT_TYPE, "application/json");
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, bearer(token));
    }
    builder.body(Body::from(body.to_string())).unwrap()
}

/// A request that names one document, whatever the format.
fn chosen(format: &str) -> String {
    serde_json::json!({ "document_ids": [ONE], "format": format }).to_string()
}

#[tokio::test]
async fn the_key_is_checked_before_the_body_and_the_scope_is_documents_read() {
    let app = app(&fake(1, false));
    for (token, expected) in [
        (None, StatusCode::UNAUTHORIZED),
        (Some(ADMIN_ONLY), StatusCode::FORBIDDEN),
    ] {
        let (status, _, body) = send(&app, post(token, "{not json")).await;
        assert_eq!(status, expected, "{body}");
    }
}

#[tokio::test]
async fn without_a_store_the_register_is_unavailable() {
    let app = finish(routes(), AppState::shared(FakeAuth::new()));
    let (status, _, _) = send(
        &app,
        post(Some(READER), r#"{"from":"2026-03-01","to":"2026-03-31"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn the_default_file_is_an_excel_workbook_named_for_the_period() {
    let app = app(&fake(1, false));
    let (status, headers, body) = send(
        &app,
        post(Some(READER), r#"{"from":"2026-03-01","to":"2026-03-31"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers[header::CONTENT_TYPE],
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
    );
    assert_eq!(
        headers[header::CONTENT_DISPOSITION],
        "attachment; filename=\"clearcut-register-2026-03-01-to-2026-03-31.xlsx\""
    );
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    assert!(body.starts_with("PK"), "an Excel file is a zip");
}

#[tokio::test]
async fn a_chosen_list_wins_over_a_period_and_names_the_file_for_the_selection() {
    let app = app(&fake(1, false));
    let body = serde_json::json!({
        "from": "2026-03-01",
        "to": "2026-03-31",
        "document_ids": [ONE],
        "format": "json",
    })
    .to_string();
    let (status, headers, body) = send(&app, post(Some(READER), &body)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "application/json");
    assert_eq!(
        headers[header::CONTENT_DISPOSITION],
        "attachment; filename=\"clearcut-register-selection.json\""
    );
    let view = json(&body);
    assert_eq!(view["selection"]["type"], "documents");
    assert_eq!(view["manifest"]["documents"], 1);
    assert_eq!(view["rows"][0]["supplier"], "Acme 0 (Pty) Ltd");

    // The list wins even when the period beside it could not be used.
    let body = serde_json::json!({
        "from": "not-a-date",
        "document_ids": [ONE],
        "format": "json",
    })
    .to_string();
    let (status, _, answer) = send(&app, post(Some(READER), &body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
}

#[tokio::test]
async fn the_csv_bundle_is_a_zip_and_the_pdf_starts_with_pdf() {
    let app = app(&fake(1, false));
    for (format, content_type, extension, magic) in [
        ("csv", "application/zip", "zip", "PK"),
        ("pdf", "application/pdf", "pdf", "%PDF-"),
    ] {
        let (status, headers, body) = send(&app, post(Some(READER), &chosen(format))).await;
        assert_eq!(status, StatusCode::OK, "{format}");
        assert_eq!(headers[header::CONTENT_TYPE], content_type, "{format}");
        assert_eq!(
            headers[header::CONTENT_DISPOSITION],
            format!("attachment; filename=\"clearcut-register-selection.{extension}\""),
            "{format}"
        );
        assert!(body.starts_with(magic), "{format}");
    }
}

#[tokio::test]
async fn only_ready_is_honoured_and_the_file_says_so() {
    let app = app(&fake(1, false));
    let (status, _, body) = send(
        &app,
        post(
            Some(READER),
            r#"{"from":"2026-03-01","to":"2026-03-31","format":"json","only_ready":true}"#,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let view = json(&body);
    assert_eq!(view["only_ready"], true);
    assert_eq!(view["manifest"]["only_ready"], true);
}

#[tokio::test]
async fn a_period_or_a_list_that_makes_no_sense_is_refused() {
    let app = app(&fake(1, false));
    for body in [
        r#"{}"#,
        r#"{"from":"2026-03-01"}"#,
        r#"{"to":"2026-03-31"}"#,
        r#"{"from":"2026-13-01","to":"2026-03-31"}"#,
        r#"{"from":"2026-03-31","to":"2026-03-01"}"#,
        r#"{"document_ids":[]}"#,
    ] {
        let (status, _, answer) = send(&app, post(Some(READER), body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {answer}");
    }
    let many = serde_json::json!({
        "document_ids": (0..=MAX_DOCUMENTS)
            .map(|n| format!("00000000-0000-0000-0000-{n:012}"))
            .collect::<Vec<_>>(),
    })
    .to_string();
    let (status, _, answer) = send(&app, post(Some(READER), &many)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "501 ids: {answer}");
}

#[tokio::test]
async fn a_period_over_the_limit_is_refused_with_a_sentence() {
    let app = app(&fake(MAX_DOCUMENTS + 1, false));
    let (status, _, body) = send(
        &app,
        post(
            Some(READER),
            r#"{"from":"2026-03-01","to":"2026-03-31","format":"json"}"#,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("split the period"), "{body}");
}

#[tokio::test]
async fn a_format_that_does_not_exist_is_refused() {
    let app = app(&fake(1, false));
    // `zip` is the extension of the bundle; the API calls it `csv`.
    for format in ["exe", "zip"] {
        let (status, _, answer) = send(&app, post(Some(READER), &chosen(format))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{format}: {answer}");
    }
}

#[tokio::test]
async fn a_named_document_that_is_not_the_accounts_is_not_found() {
    let app = app(&fake(0, false));
    let (status, _, body) = send(&app, post(Some(READER), &chosen("json"))).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

#[tokio::test]
async fn an_id_that_is_not_a_document_id_is_refused() {
    let app = app(&fake(0, true));
    let (status, _, _) = send(
        &app,
        post(Some(READER), r#"{"document_ids":["not-a-uuid"]}"#),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p clearcut-server register`
Expected: FAIL, `no method named register` and `unresolved module register`.

- [ ] **Step 3: Add `Results::register` and the `400` for a bad id**

In `crates/clearcut-server/src/results/mod.rs`, add to the imports:

```rust
use clearcut_core::register::{RegisterInput, Selection};
```

Add the method to the `Results` trait, after `search`:

```rust
    /// The documents a selection names, with their current events and the pages waiting for a
    /// person, in the shape `clearcut_core::register::build` takes.
    fn register<'a>(
        &'a self,
        tenant: TenantId,
        selection: &'a Selection,
        only_ready: bool,
    ) -> ResultsFuture<'a, RegisterInput>;
```

Add the `Store` implementation beside the other four:

```rust
    fn register<'a>(
        &'a self,
        tenant: TenantId,
        selection: &'a Selection,
        only_ready: bool,
    ) -> ResultsFuture<'a, RegisterInput> {
        Box::pin(async move { self.register_sources(tenant, selection, only_ready).await })
    }
```

In `impl IntoResponse for ResultsProblem`, add the arm beside `DocumentNotFound`:

```rust
                clearcut_store::Error::InvalidSelectionId(id) => {
                    message(StatusCode::BAD_REQUEST, &format!("not a document id: {id}"))
                }
```

In `crates/clearcut-server/src/results/tests.rs`, add the method to the fake, after `search`:

```rust
    fn register<'a>(
        &'a self,
        _tenant: TenantId,
        _selection: &'a Selection,
        _only_ready: bool,
    ) -> ResultsFuture<'a, RegisterInput> {
        Box::pin(async { unreachable!("the register has its own tests") })
    }
```

(`use super::*` already brings `Selection` and `RegisterInput` in.)

- [ ] **Step 4: Write the handler**

Create `crates/clearcut-server/src/register.rs`:

```rust
// Written by Kone Tshivhinda & Claude
//! `POST /v1/register`: one period's documents, read and checked, in a file a bookkeeper or a
//! finance team can use: Excel, a zipped CSV bundle, JSON, PDF, HTML, Markdown or plain text.
//!
//! The register is built from the stored events and page states each time it is asked for
//! (`clearcut_core::register::build`); nothing is stored and no background job runs. Needs
//! `documents:read`. A period (`from` and `to`, both included) or a list of `document_ids` says
//! which documents; the list wins when both are given. Over 500 documents or 50 000 line items is
//! a `400` asking the caller to split the selection. A named document that is not this account's
//! is a `404`. The file says at its foot that its figures must be checked against the original.

use crate::auth::{ApiProblem, AppState, Authed};
use crate::results::ResultsProblem;
use axum::extract::State;
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::Json;
use clearcut_core::auth::Scope;
use clearcut_core::register::{build, parse_date, RegisterError, Selection, MAX_DOCUMENTS};
use clearcut_export::{render_register, RegisterFormat};
use clearcut_store::TenantId;
use serde::Deserialize;

/// The body of `POST /v1/register`. A field is optional on its own; `select` says which
/// combinations make sense.
#[derive(Debug, Deserialize)]
struct RegisterRequest {
    from: Option<String>,
    to: Option<String>,
    document_ids: Option<Vec<String>>,
    format: Option<String>,
    only_ready: Option<bool>,
}

/// Which documents the request names, checked. The list wins over the period; the period's two
/// ends go together; an end before the start is refused. Repeats in the list are counted once.
fn select(body: &RegisterRequest) -> Result<Selection, ResultsProblem> {
    if let Some(ids) = &body.document_ids {
        let mut unique: Vec<String> = Vec::with_capacity(ids.len());
        for id in ids {
            if !unique.contains(id) {
                unique.push(id.clone());
            }
        }
        if unique.is_empty() {
            return Err(ResultsProblem::BadChoice(
                "document_ids must name at least one document",
            ));
        }
        if unique.len() > MAX_DOCUMENTS {
            return Err(ResultsProblem::BadChoice(
                "document_ids may name at most 500 documents",
            ));
        }
        return Ok(Selection::Documents { ids: unique });
    }
    let from = match body.from.as_deref() {
        None => None,
        Some(text) => Some(parse_date(text).ok_or(ResultsProblem::BadChoice(
            "from must be a date written as YYYY-MM-DD",
        ))?),
    };
    let to = match body.to.as_deref() {
        None => None,
        Some(text) => Some(parse_date(text).ok_or(ResultsProblem::BadChoice(
            "to must be a date written as YYYY-MM-DD",
        ))?),
    };
    if from.is_some() != to.is_some() {
        return Err(ResultsProblem::BadChoice(
            "give from and to together, or neither",
        ));
    }
    let (Some(from), Some(to)) = (from, to) else {
        return Err(ResultsProblem::BadChoice(
            "name a period with from and to, or up to 500 documents",
        ));
    };
    if to < from {
        return Err(ResultsProblem::BadChoice("to is before from"));
    }
    Ok(Selection::Period { from, to })
}

/// The download's file name: the period it covers, or the selection, and the file's own
/// extension (the CSV bundle is a `.zip`).
fn filename(selection: &Selection, extension: &str) -> String {
    match selection {
        Selection::Period { from, to } => format!("clearcut-register-{from}-to-{to}.{extension}"),
        Selection::Documents { .. } => format!("clearcut-register-selection.{extension}"),
    }
}

/// Why a register could not be built, as a sentence for the caller. All three ask for a smaller
/// selection; none of them is the caller's fault in the sense of a bad format.
fn why(error: RegisterError) -> ResultsProblem {
    match error {
        RegisterError::TooManyDocuments => ResultsProblem::BadChoice(
            "the selection names more than 500 documents; split the period or the list",
        ),
        RegisterError::TooManyLines => ResultsProblem::BadChoice(
            "the selection holds more than 50 000 line items; split the period",
        ),
        RegisterError::Overflow => ResultsProblem::BadChoice(
            "the amounts in this selection are too large to add up; split the period",
        ),
    }
}

/// `POST /v1/register`
pub(crate) async fn make_register(
    State(state): State<AppState>,
    auth: Authed,
    Json(body): Json<RegisterRequest>,
) -> Result<Response, ResultsProblem> {
    let principal = auth.require(Scope::DocumentsRead)?;
    let selection = select(&body)?;
    let format = RegisterFormat::parse(body.format.as_deref().unwrap_or("xlsx")).ok_or(
        ResultsProblem::BadChoice("format must be json, html, md, txt, xlsx, csv or pdf"),
    )?;
    let results = state.results().ok_or(ResultsProblem::NotConfigured)?;
    let tenant = TenantId(principal.tenant());
    let only_ready = body.only_ready.unwrap_or(false);
    let input = results.register(tenant, &selection, only_ready).await?;
    if matches!(&selection, Selection::Documents { ids } if input.facts.len() != ids.len()) {
        return Err(ResultsProblem::Store(Box::new(
            clearcut_store::Error::DocumentNotFound,
        )));
    }
    let register = build(input).map_err(why)?;
    tracing::info!(
        tenant = %tenant.0,
        format = format.name(),
        documents = register.rows.len(),
        lines = register.lines.len(),
        only_ready,
        "a register was made"
    );
    let file = render_register(format, &register).map_err(|error| {
        tracing::error!(%error, "a register could not be made");
        ResultsProblem::Api(ApiProblem::Internal)
    })?;
    Ok((
        [
            (header::CONTENT_TYPE, file.content_type.to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!(
                    "attachment; filename=\"{}\"",
                    filename(&selection, file.extension)
                ),
            ),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
        file.bytes,
    )
        .into_response())
}

#[cfg(test)]
mod tests;
```

- [ ] **Step 5: Register the route**

In `crates/clearcut-server/src/lib.rs`, add `mod register;` beside `mod review;`, and the route beside `/v1/search`:

```rust
        .route("/v1/register", post(register::make_register))
```

- [ ] **Step 6: Document the path**

In `crates/clearcut-server/src/openapi.rs`, inside the `paths` map, after the `/v1/search` entry, add:

```rust
        "/v1/register": { "post": op("The register for a period or a list of documents, as a file: Excel, a zipped CSV bundle, JSON, PDF, HTML, Markdown or plain text", Some("documents:read"), vec![],
            Some(json!({ "required": true, "content": { "application/json": { "schema": {
                "type": "object",
                "properties": {
                    "from": { "type": "string", "format": "date", "description": "First day, included; give it with to" },
                    "to": { "type": "string", "format": "date", "description": "Last day, included; give it with from" },
                    "document_ids": { "type": "array", "items": { "type": "string" }, "maxItems": 500, "description": "Exactly these documents; wins over the period" },
                    "format": { "type": "string", "enum": ["json", "html", "md", "txt", "xlsx", "csv", "pdf"], "default": "xlsx" },
                    "only_ready": { "type": "boolean", "default": false, "description": "Leave out documents awaiting review, and say how many in the manifest" }
                } } } } })),
            responses(json!({ "200": { "description": "The file", "content": { "application/json": {}, "text/html": {}, "text/markdown": {}, "text/plain": {}, "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet": {}, "application/zip": {}, "application/pdf": {} } } }),
                &with_auth(&[("400", "The dates, the list or the format make no sense, or the register is over 500 documents or 50 000 line items"), ("404", "A named document is not this account's"), ("503", "The server has no store")]))) },
```

- [ ] **Step 7: Regenerate the contract and run the server tests**

Run: `cargo fmt --all`

Run: `CLEARCUT_UPDATE_CONTRACTS=1 cargo test -p clearcut-server openapi`
Expected: PASS; `contracts/openapi.json` is rewritten with the new path.

Run: `cargo test -p clearcut-server register`
Expected: PASS, 11 tests: `the_key_is_checked_before_the_body_and_the_scope_is_documents_read`, `without_a_store_the_register_is_unavailable`, `the_default_file_is_an_excel_workbook_named_for_the_period`, `a_chosen_list_wins_over_a_period_and_names_the_file_for_the_selection`, `the_csv_bundle_is_a_zip_and_the_pdf_starts_with_pdf`, `only_ready_is_honoured_and_the_file_says_so`, `a_period_or_a_list_that_makes_no_sense_is_refused`, `a_period_over_the_limit_is_refused_with_a_sentence`, `a_format_that_does_not_exist_is_refused`, `a_named_document_that_is_not_the_accounts_is_not_found`, `an_id_that_is_not_a_document_id_is_refused`.

Run: `cargo test -p clearcut-server`
Expected: PASS, including the existing results tests against their new trait method and `every_documented_route_exists_and_every_protected_one_asks_for_a_key`.

- [ ] **Step 8: Commit**

```bash
git add crates/clearcut-server/src/register.rs crates/clearcut-server/src/register crates/clearcut-server/src/lib.rs crates/clearcut-server/src/results/mod.rs crates/clearcut-server/src/results/tests.rs crates/clearcut-server/src/openapi.rs contracts/openapi.json
git commit -m "feat(server): answer POST /v1/register with the register as a file"
```

### Task 8: The Register screen

**Files:**
- Modify: `crates/clearcut-server/src/web/index.html` (the nav link)
- Modify: `crates/clearcut-server/src/web/app.js` (the screen, the period rules, the download helper, and choosing documents on the Documents screen)
- Modify: `crates/clearcut-server/src/web/app.css` (date inputs, the checklist column)
- Modify: `crates/clearcut-server/src/web.rs` (a test for the screen's markers)

**Interfaces:**
- Consumes: `POST /v1/register` from Task 7 (JSON preview, files for download); the existing `h`, `show`, `api`, `badge`, `failure`, `signIn` and `stopPolling`.
- Produces: `#/register` and `#/register?ids=<uuid,...>`; `registerPage(chosenIds)`; `saveFile(response, fallback)`; `periodBounds(choice, from, to)`.

- [ ] **Step 1: Write the failing web test**

In `crates/clearcut-server/src/web.rs`, inside `mod tests`, add:

```rust
    #[tokio::test]
    async fn the_register_screen_is_reachable_and_says_what_it_is() {
        let (_, _, html) = fetch("/").await;
        assert!(
            html.contains("id=\"register-link\"") && html.contains("href=\"#/register\""),
            "the header links the register"
        );
        let (_, _, js) = fetch("/app.js").await;
        for marker in [
            "function registerPage",
            "const PERIODS",
            "Last month",
            "#/register?ids=",
            "clearcut-register",
            "The file is built when you ask for it; nothing is kept.",
            "Check the file against the originals",
        ] {
            assert!(js.contains(marker), "{marker} in app.js");
        }
        let (_, _, css) = fetch("/app.css").await;
        assert!(
            css.contains("input[type=\"date\"]"),
            "the custom range's dates are styled"
        );
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p clearcut-server the_register_screen_is_reachable_and_says_what_it_is`
Expected: FAIL, `the header links the register`.

- [ ] **Step 3: Add the nav link and show it when signed in**

In `crates/clearcut-server/src/web/index.html`, add beside the settings link:

```html
  <a id="register-link" class="nav" href="#/register" hidden>Register</a>
```

In `crates/clearcut-server/src/web/app.js`, beside the other elements at the top:

```js
const registerLink = document.getElementById("register-link");
```

and in `setSignedIn`:

```js
  registerLink.hidden = !signedIn;
```

- [ ] **Step 4: Add the download helper**

In `crates/clearcut-server/src/web/app.js`, after `json(path)` and before `failure`, add:

```js
// Saves a response as a file, under the name the server chose when it gave one.
async function saveFile(response, fallback) {
  if (!response.ok) {
    const body = await response.json().catch(() => ({}));
    throw new Error(body.error || "The download failed (" + response.status + ").");
  }
  const found = (response.headers.get("Content-Disposition") || "").match(/filename="([^"]+)"/);
  const url = URL.createObjectURL(await response.blob());
  const link = h("a", { href: url, download: found ? found[1] : fallback });
  document.body.append(link);
  link.click();
  link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 10000);
}
```

- [ ] **Step 5: Choose documents on the Documents screen**

In `documents()`, after `const fileInput = ...`, add:

```js
  const chosen = new Set();
  const makeRegister = h("button", { type: "button", class: "secondary", disabled: true,
    onclick: () => {
      location.hash = "#/register?ids=" + [...chosen].join(",");
    } }, "Make a register from these");
```

In the table header, add a first `th`:

```js
h("th", {}, "Choose"),
```

In the row, add a first `td` before the file name:

```js
h("td", {}, h("input", { type: "checkbox", "aria-label": "Choose " + d.original_filename,
  onclick: (event) => {
    event.stopPropagation();
    if (event.currentTarget.checked) chosen.add(d.id);
    else chosen.delete(d.id);
    makeRegister.disabled = chosen.size === 0;
  } })),
```

And in the page markup, between the upload card and `h("h2", {}, "Your documents")`:

```js
    h("div", { class: "row" }, makeRegister,
      h("p", { class: "small muted" }, "Tick documents to put them in one register.")),
```

- [ ] **Step 6: Add the period rules and the screen**

In `crates/clearcut-server/src/web/app.js`, before the `// ---------------------------------------------------------------- routing ----` section, add:

```js
// ---------------------------------------------------------------- 4. register ----

const PERIODS = [
  ["last", "Last month"],
  ["this", "This month"],
  ["quarter", "This quarter"],
  ["custom", "Custom range"],
];

const REGISTER_KINDS = { invoice: "Invoice", credit_note: "Credit note", unclassified: "Unclassified" };

const REGISTER_TAX = {
  standard: "Standard",
  zero_rated: "Zero-rated",
  exempt: "Exempt",
  out_of_scope: "Outside scope",
};

function isoDate(date) {
  return date.getFullYear() + "-" + String(date.getMonth() + 1).padStart(2, "0") + "-" +
    String(date.getDate()).padStart(2, "0");
}

// The first and last day of the chosen period, as YYYY-MM-DD. The local calendar decides what
// "last month" is; the server is given plain dates and does no time-zone arithmetic.
function periodBounds(choice, from, to) {
  const now = new Date();
  if (choice === "custom") return [from, to];
  if (choice === "this") {
    return [isoDate(new Date(now.getFullYear(), now.getMonth(), 1)),
      isoDate(new Date(now.getFullYear(), now.getMonth() + 1, 0))];
  }
  if (choice === "quarter") {
    const first = Math.floor(now.getMonth() / 3) * 3;
    return [isoDate(new Date(now.getFullYear(), first, 1)),
      isoDate(new Date(now.getFullYear(), first + 3, 0))];
  }
  return [isoDate(new Date(now.getFullYear(), now.getMonth() - 1, 1)),
    isoDate(new Date(now.getFullYear(), now.getMonth(), 0))];
}

function amountText(amount) {
  return amount ? amount.text : "–";
}

function rateText(bps) {
  const percent = bps / 100;
  return (Number.isInteger(percent) ? String(percent) : percent.toFixed(1)) + "%";
}

function previewParts(view) {
  const m = view.manifest;
  const parts = [];
  parts.push(h("div", { class: "card" },
    h("h2", {}, "This register"),
    h("p", {}, m.documents + " document(s): " + m.done + " done, " + m.needs_review +
      " awaiting review, " + m.still_reading + " still reading."),
    h("p", { class: "small muted" },
      m.automation_rate_bps == null
        ? "No finished documents yet, so there is no automation rate."
        : rateText(m.automation_rate_bps) + " of finished documents needed nobody.",
      " ",
      m.undated_in_selection
        ? m.undated_in_selection + " document(s) have no date and are not in a month. "
        : "",
      m.awaiting_left_out
        ? m.awaiting_left_out + " awaiting review were left out."
        : "")));

  if (view.rows.length === 0) {
    parts.push(h("div", { class: "states" }, "No documents in this register. Change the period, or upload an invoice."));
  } else {
    const shown = view.rows.slice(0, 200);
    parts.push(h("h2", {}, "Documents"));
    parts.push(h("div", { class: "table-wrap" }, h("table", {},
      h("thead", {}, h("tr", {},
        h("th", {}, "Date"), h("th", {}, "Supplier"), h("th", {}, "Number"),
        h("th", {}, "Kind"), h("th", {}, "Status"), h("th", {}, "Currency"),
        h("th", { class: "num" }, "Net"), h("th", { class: "num" }, "VAT"),
        h("th", { class: "num" }, "Gross"), h("th", { class: "num" }, "Lines"))),
      h("tbody", {}, shown.map((row) => h("tr", {},
        h("td", {}, row.invoice_date || "No date"),
        h("td", {}, row.supplier || "–"),
        h("td", {}, row.invoice_number || "–"),
        h("td", {}, REGISTER_KINDS[row.kind] || row.kind),
        h("td", {}, badge(row.status)),
        h("td", {}, row.currency || "–"),
        h("td", { class: "num" }, amountText(row.net)),
        h("td", { class: "num" }, amountText(row.vat)),
        h("td", { class: "num" }, amountText(row.gross)),
        h("td", { class: "num" }, row.line_count)))))));
    if (view.rows.length > shown.length) {
      parts.push(h("p", { class: "small muted" },
        "Showing the first " + shown.length + " of " + view.rows.length + " documents. The file holds all of them."));
    }
  }

  if (view.vat_by_category.length) {
    const lines = [];
    for (const group of view.vat_by_category) {
      for (const [side, part] of [["Ready", group.ready], ["Awaiting review", group.awaiting]]) {
        if (!part) continue;
        lines.push(h("tr", {},
          h("td", {}, group.month), h("td", {}, group.currency),
          h("td", {}, REGISTER_KINDS[group.kind] || group.kind),
          h("td", {}, REGISTER_TAX[group.category] || group.category),
          h("td", { class: "num" }, rateText(group.rate_bps)),
          h("td", {}, side),
          h("td", { class: "num" }, part.documents),
          h("td", { class: "num" }, part.taxable.text),
          h("td", { class: "num" }, part.vat.text)));
      }
    }
    parts.push(h("h2", {}, "VAT by category"));
    parts.push(h("div", { class: "table-wrap" }, h("table", {},
      h("thead", {}, h("tr", {},
        h("th", {}, "Month"), h("th", {}, "Currency"), h("th", {}, "Kind"),
        h("th", {}, "Category"), h("th", { class: "num" }, "Rate"), h("th", {}, "Side"),
        h("th", { class: "num" }, "Documents"), h("th", { class: "num" }, "Taxable"),
        h("th", { class: "num" }, "VAT"))),
      h("tbody", {}, lines))));
  }

  if (view.reconciliation.length) {
    parts.push(h("h2", {}, "Calculated against stated"));
    parts.push(h("div", { class: "table-wrap" }, h("table", {},
      h("thead", {}, h("tr", {},
        h("th", {}, "Month"), h("th", {}, "Currency"), h("th", {}, "Kind"),
        h("th", { class: "num" }, "Calculated VAT"), h("th", { class: "num" }, "Stated VAT"),
        h("th", { class: "num" }, "Difference"), h("th", { class: "num" }, "Documents that disagree"))),
      h("tbody", {}, view.reconciliation.map((row) => h("tr", {},
        h("td", {}, row.month), h("td", {}, row.currency),
        h("td", {}, REGISTER_KINDS[row.kind] || row.kind),
        h("td", { class: "num" }, row.calculated_vat.text),
        h("td", { class: "num" }, row.stated_vat.text),
        h("td", { class: "num" }, row.difference.text),
        h("td", { class: "num" }, row.documents_disagree))))));
  }

  parts.push(h("h2", {}, "To check"));
  if (view.to_check.length === 0) {
    parts.push(h("p", { class: "small muted" }, "Nothing is waiting for a person."));
  } else {
    parts.push(h("div", { class: "table-wrap" }, h("table", {},
      h("thead", {}, h("tr", {},
        h("th", {}, "Document"), h("th", {}, "Reason"), h("th", { class: "num" }, "Page"),
        h("th", {}, "What to check"))),
      h("tbody", {}, view.to_check.map((row) => h("tr", {},
        h("td", {}, row.filename),
        h("td", {}, row.reason),
        h("td", { class: "num" }, row.page ?? "–"),
        h("td", {}, row.sentence))))));
  }

  parts.push(h("p", { class: "small muted" }, view.notice));
  return parts;
}

function registerPage(chosenIds) {
  stopPolling();
  const ids = (chosenIds || "").split(",").map((id) => id.trim()).filter(Boolean);

  const choice = h("select", { id: "register-period" },
    PERIODS.map(([value, label]) => h("option", { value }, label)));
  choice.value = "last";
  const from = h("input", { type: "date", id: "register-from" });
  const to = h("input", { type: "date", id: "register-to" });
  const [firstFrom, firstTo] = periodBounds("last", "", "");
  from.value = firstFrom;
  to.value = firstTo;
  const custom = h("span", { class: "row", hidden: true },
    h("label", { for: "register-from" }, "From"), from,
    h("label", { for: "register-to" }, "To"), to);
  const periodRow = h("div", { class: "row", hidden: ids.length > 0 },
    h("label", { for: "register-period" }, "Period"), choice, custom);
  choice.addEventListener("change", () => {
    custom.hidden = choice.value !== "custom";
  });
  const chosenNote = h("p", { class: "small muted" },
    ids.length + (ids.length === 1 ? " chosen document." : " chosen documents."));

  const status = h("p", { class: "small muted", role: "status" }, "");
  const preview = h("div", {}, h("div", { class: "states" },
    "The file is built when you ask for it; nothing is kept."));

  function request(format) {
    if (ids.length) return { document_ids: ids, format };
    const [start, end] = periodBounds(choice.value, from.value, to.value);
    if (!start || !end) throw new Error("Choose both dates.");
    return { from: start, to: end, format };
  }

  function ask(format) {
    return api("/v1/register", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(request(format)),
    });
  }

  async function previewRegister() {
    status.className = "small muted";
    status.textContent = "Building the preview…";
    try {
      const response = await ask("json");
      const view = await response.json().catch(() => ({}));
      if (!response.ok) throw new Error(view.error || "The register could not be built (" + response.status + ").");
      preview.replaceChildren(...previewParts(view));
      status.textContent = "";
    } catch (e) {
      if (e instanceof AuthError) return signIn(e.message);
      status.className = "small error";
      status.textContent = e.message || "Something went wrong.";
    }
  }

  async function download(format, label) {
    status.className = "small muted";
    status.textContent = "Making the " + label + "…";
    try {
      await saveFile(await ask(format), "clearcut-register." + (format === "csv" ? "zip" : format));
      status.textContent = "";
    } catch (e) {
      if (e instanceof AuthError) return signIn(e.message);
      status.className = "small error";
      status.textContent = e.message || "Something went wrong.";
    }
  }

  const button = (format, label) => h("button", {
    type: "button", class: format === "xlsx" ? "" : "secondary",
    onclick: () => download(format, label),
  }, label);

  show(h("div", { class: "page" },
    h("div", { class: "page-head" },
      h("p", { class: "eyebrow" }, "Workspace"),
      h("h1", {}, "Register"),
      h("p", { class: "lede" }, "Every document in a period, with its figures, its VAT and the things that need a person. Check the file against the originals before you use it.")),
    h("div", { class: "card" },
      ids.length ? chosenNote : null,
      periodRow,
      h("div", { class: "row" },
        h("button", { type: "button", onclick: previewRegister }, "Preview"),
        button("xlsx", "Excel"),
        button("csv", "CSV bundle"),
        button("json", "JSON"),
        button("pdf", "PDF")),
      status),
    preview));

  previewRegister();
}
```

- [ ] **Step 7: Route the screen**

In `route()`, after `setSignedIn(true);`, before the `#/d/` match:

```js
  const register = hash.match(/^#\/register(?:\?ids=([0-9a-f,-]+))?$/i);
  if (register) return registerPage(register[1] || "");
```

- [ ] **Step 8: Style the date inputs and the choosing column**

In `crates/clearcut-server/src/web/app.css`, add `input[type="date"]` to the input rule:

```css
input[type="password"], input[type="text"], input[type="number"], input[type="date"] {
```

- [ ] **Step 9: Run the tests**

Run: `cargo test -p clearcut-server web::tests`
Expected: PASS, including `the_register_screen_is_reachable_and_says_what_it_is`.

Run: `cargo test -p clearcut-server`
Expected: PASS.

- [ ] **Step 10: Check the screen in a browser**

Start the local stack as `docs/RUN-LOCALLY.md` says, or use the live app, and sign in with a key that has `documents:read`.

1. Open `#/` and sign in. The header shows Register beside About and Settings.
2. Open `#/register`. Last month is chosen; the preview appears by itself. The summary names the document count, the done and awaiting counts, the automation rate and anything undated.
3. Change the period to This quarter, then to Custom range. The two date inputs appear; clearing one and pressing Preview shows "Choose both dates."
4. Download Excel and CSV bundle. The saved names are `clearcut-register-<from>-to-<to>.xlsx` and `.zip`; open the workbook and confirm the six sheets and that an amount cell is a number, not text.
5. On `#/`, tick two documents, press "Make a register from these", and confirm `#/register?ids=...` previews only those two and the downloads are named `clearcut-register-selection.<ext>`.
6. Press Tab from the top of the screen: the nav, the period, the dates, Preview and every download button are reachable, and Enter on a document row still opens the document. Space on a checkbox ticks it without opening the document.

This check reads existing documents only. Do not upload a file during it: an upload sends a document to the model and spends money.

- [ ] **Step 11: Commit**

```bash
git add crates/clearcut-server/src/web/index.html crates/clearcut-server/src/web/app.js crates/clearcut-server/src/web/app.css crates/clearcut-server/src/web.rs
git commit -m "feat(web): add the Register screen with a preview and downloads"
```

### Task 9: Bulk intake

**Files:**
- Modify: `crates/clearcut-server/src/web/app.js` (the batch card, the queue and its polling)
- Modify: `crates/clearcut-server/src/web.rs` (a test for the batch control)

**Interfaces:**
- Consumes: `POST /v1/documents?filename=...` (unchanged), `GET /v1/documents/{id}` (unchanged), `json`, `api`, `h`, `badge`, `AuthError`, `signIn`.
- Produces: `bulkCard(refresh)`; no API change.

- [ ] **Step 1: Write the failing web test**

In `crates/clearcut-server/src/web.rs`, inside `mod tests`, add:

```rust
    #[tokio::test]
    async fn the_upload_screen_takes_a_batch_and_says_what_it_costs() {
        let (_, _, js) = fetch("/app.js").await;
        for marker in [
            "function bulkCard",
            "up to 20 invoices",
            "Each document is sent to Anthropic to be read and costs money.",
            "multiple: true",
            "Try again",
            "Still reading",
        ] {
            assert!(js.contains(marker), "{marker} in app.js");
        }
        // The batch path must not reintroduce markup from a file name or the server.
        for forbidden in ["innerHTML", "outerHTML", "insertAdjacentHTML"] {
            assert!(!js.contains(forbidden), "{forbidden} in app.js");
        }
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p clearcut-server the_upload_screen_takes_a_batch_and_says_what_it_costs`
Expected: FAIL, `function bulkCard in app.js`.

- [ ] **Step 3: Write the batch control**

In `crates/clearcut-server/src/web/app.js`, before the `// ---------------------------------------------------------------- 2. documents ----` section, add:

```js
// ---------------------------------------------------------------- bulk intake ----

// Several files, one after another through the same upload call. A failure on one does not stop
// the rest. The batch is capped because every document that is read is sent to Anthropic and
// costs money; the screen says so before anything is chosen.
function bulkCard(refresh) {
  const input = h("input", { type: "file", multiple: true,
    accept: ".pdf,.png,.jpg,.jpeg,application/pdf,image/png,image/jpeg" });
  const pick = h("label", { class: "drop" },
    h("strong", {}, "Choose up to 20 invoices"),
    " to upload one after another, or drop them here",
    input);
  const note = h("p", { class: "small muted" },
    "Each document is sent to Anthropic to be read and costs money. Files are uploaded one at a time; a failure does not stop the others.");
  const list = h("div", {});
  const rows = [];

  function redraw() {
    list.replaceChildren(h("div", { class: "table-wrap" }, h("table", {},
      h("thead", {}, h("tr", {},
        h("th", {}, "File"), h("th", {}, "Status"), h("th", {}, "Open"))),
      h("tbody", {}, rows.map((row) => h("tr", {},
        h("td", {}, row.file.name), row.state, row.act))))));
  }

  function retry(row) {
    row.act.replaceChildren(h("button", { type: "button", class: "secondary",
      onclick: () => {
        row.act.replaceChildren();
        // A file whose upload succeeded is only asked about again; it is never sent twice.
        const again = row.documentId ? follow(row) : uploadOne(row);
        again.catch((e) => {
          if (e instanceof AuthError) {
            key = null;
            setSignedIn(false);
            signIn(e.message);
          }
        });
      } }, "Try again"));
  }

  async function follow(row) {
    row.state.textContent = "Reading…";
    for (let tries = 0; tries < 120; tries += 1) {
      let info;
      try {
        info = await json("/v1/documents/" + row.documentId);
      } catch (e) {
        if (e instanceof AuthError) throw e;
        row.state.textContent = e.message || "Could not check the document.";
        retry(row);
        return;
      }
      if (info.status === "done" || info.status === "needs_review") {
        row.state.replaceChildren(badge(info.status));
        row.act.replaceChildren(h("a", { href: "#/d/" + row.documentId }, "Open"));
        if (refresh) refresh();
        return;
      }
      if (info.status !== "received" && info.status !== "processing") {
        row.state.textContent = "Stopped (" + info.status + ")";
        retry(row);
        return;
      }
      await new Promise((done) => setTimeout(done, 3000));
    }
    row.state.textContent = "Still reading";
    row.act.replaceChildren(h("a", { href: "#/d/" + row.documentId }, "Open"));
    if (refresh) refresh();
  }

  async function uploadOne(row) {
    row.state.textContent = "Uploading…";
    try {
      const response = await api("/v1/documents?filename=" + encodeURIComponent(row.file.name), {
        method: "POST",
        headers: { "Content-Type": row.file.type || "application/pdf" },
        body: row.file,
      });
      const body = await response.json().catch(() => ({}));
      if (!response.ok) throw new Error(body.error || "The upload was refused (" + response.status + ").");
      row.documentId = body.document_id;
      await follow(row);
    } catch (e) {
      if (e instanceof AuthError) throw e;
      row.state.textContent = e.message || "The upload failed.";
      retry(row);
    }
  }

  async function run(files) {
    const batch = files.slice(0, 20);
    if (files.length > 20) {
      note.textContent = "Only the first 20 files were taken. Choose the rest in another batch.";
    }
    rows.length = 0;
    for (const file of batch) {
      rows.push({ file, documentId: null, state: h("td", {}, "Waiting"), act: h("td", {}) });
    }
    redraw();
    for (const row of rows) {
      try {
        await uploadOne(row);
      } catch (e) {
        if (e instanceof AuthError) {
          key = null;
          setSignedIn(false);
          return signIn(e.message);
        }
      }
    }
    if (refresh) refresh();
  }

  input.addEventListener("change", () => {
    const files = [...input.files];
    input.value = "";
    if (files.length) run(files);
  });
  pick.addEventListener("dragover", (event) => { event.preventDefault(); pick.classList.add("over"); });
  pick.addEventListener("dragleave", () => pick.classList.remove("over"));
  pick.addEventListener("drop", (event) => {
    event.preventDefault();
    pick.classList.remove("over");
    if (event.dataTransfer.files.length) run([...event.dataTransfer.files]);
  });

  return h("div", {}, pick, note, list);
}
```

- [ ] **Step 4: Put the batch control on the Documents screen**

In `documents()`, after `const list = h("div", {}, ...)`, add:

```js
  const bulk = bulkCard(load);
```

In the page markup, after `h("div", { class: "card" }, drop, status),` and before `h("h2", {}, "Your documents"),`, add:

```js
    h("div", { class: "card" },
      h("h2", {}, "Upload several"),
      bulk),
```

The single-file control above it is unchanged.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p clearcut-server web::tests`
Expected: PASS, including `the_upload_screen_takes_a_batch_and_says_what_it_costs`.

Run: `cargo test -p clearcut-server`
Expected: PASS.

- [ ] **Step 6: Check the batch in a browser**

This step uploads real files, so it spends money on the document that is read. Use a throwaway tenant, one small synthetic invoice and no customer documents.

1. Create a throwaway tenant and key:

```bash
cargo run -p clearcut-admin -- create-tenant "Bulk check"
cargo run -p clearcut-admin -- create-key --tenant <the id it printed> --name bulk --scopes documents:write,documents:read
```

2. Make a clear one-page invoice image by hand (`good.png`): supplier, invoice date, invoice number, one line with quantity and unit price, and net, VAT and gross. Copy it to `good-copy.png`. Also make `notes.txt`.
3. Sign in on the local stack with the new key, open `#/`, and drag all three files onto the "Upload several" control. Expect a row per file: `notes.txt` refuses with the server's sentence and the other two keep going; `good.png` reaches Done; `good-copy.png` finishes without a second read (the server sees the same bytes and returns the existing document). The document list gains one document.
4. Press "Try again" on the refused row after renaming `notes.txt` to `notes.pdf` (or drag the image again for a failed row) and confirm the retry never sends a file whose upload already succeeded twice; watch the server log to see one upload per file.
5. Drop 21 copies of `good.png`: exactly 20 rows appear, the note says only the first 20 were taken, and the reads finish quickly because every copy after the first is a duplicate.
6. Tab through the control: "Choose up to 20 invoices" is reachable, Space opens the picker, and each row's Open link and Try again are reachable.

The one read of `good.png` is the only model call. Afterwards revoke the throwaway key and remove the local files:

```bash
cargo run -p clearcut-admin -- revoke-key --tenant <the id> --key <the key id>
```

- [ ] **Step 7: Commit**

```bash
git add crates/clearcut-server/src/web/app.js crates/clearcut-server/src/web.rs
git commit -m "feat(web): upload a batch of invoices one after another"
```

### Task 10: Documentation, delivery and the live check

**Files:**
- Modify: `docs/PHASES.md:78`
- Modify: `docs/ARCHITECTURE.md:28-43`
- Modify: `docs/LAUNCH.md:27`
- Modify: `docs/WORKLOG.md:3`

**Interfaces:**
- Consumes: the six merged slices from Tasks 1 to 9.
- Produces: docs that name the register, the export crate and the endpoint, and a feature that is merged, deployed and checked on the live host.

- [ ] **Step 1: Close P4-08 in PHASES**

Replace:

```markdown
- ⬜ P4-08 Batch export (many documents → one workbook/zip)
```

with:

```markdown
- ✅ P4-08 Batch export (many documents → one workbook/zip) — **built** (`POST /v1/register`: a period or a list of documents returns the register as an Excel workbook with Register, Lines, VAT by category, Stated totals, To check and Manifest, a zipped CSV bundle, JSON, PDF, HTML, Markdown or plain text; the Register screen previews it and downloads it)
```

- [ ] **Step 2: Record the export crate and the endpoint in ARCHITECTURE**

Add a row immediately after the `crates/clearcut-core` row (before the `crates/clearcut-server` row):

```markdown
| `crates/clearcut-export` | Rust | The register layouts (P4-08): one `Register` built by the core, laid out as an Excel workbook with six sheets (Register, Lines, VAT by category, Stated totals, To check, Manifest), a zipped CSV bundle (one table per file plus `manifest.json`), JSON, PDF, HTML, Markdown and plain text. Amounts are real numeric cells and never formulas; a CSV cell that starts with `=`, `+`, `-` or `@` is prefixed with an apostrophe; text from a document is escaped; every file carries the notice that the register is a reading of the documents, not a set of books. One entry, `render_register(format, &Register)` | tested layout by layout |
```

In the `crates/clearcut-server` row, find the sentence that ends `and quotes the matching text).` and insert this sentence after it:

```markdown
`POST /v1/register` (`documents:read`; takes a period or a list of document ids, reads the current events only, and answers with the register as Excel (the default), a zipped CSV bundle, JSON, PDF, HTML, Markdown or plain text, named for the period; nothing is stored; 400 for a selection or format that does not make sense, 404 for a named document that is not the account's, 503 without a store).
```

- [ ] **Step 3: Record batch export in LAUNCH row 3**

Replace:

```markdown
| 3 | ~~**Exports in the formats promised**~~ **Built**: PDF, Excel (five sheets, real numbers), Word, HTML, plain text, Markdown, CSV and JSON, all laid out from one export model made from the stored events (`GET /v1/documents/{id}/export?format=`), with download buttons on the document screen. **Remaining:** batch export of many documents (P4-08), the MCP export tool (P4-09), and the Excel and Word files have been checked by their XML, not opened in Excel or Word | The product's core promise. |
```

with:

```markdown
| 3 | ~~**Exports in the formats promised**~~ **Built**: PDF, Excel (five sheets, real numbers), Word, HTML, plain text, Markdown, CSV and JSON for one document, all laid out from one export model made from the stored events (`GET /v1/documents/{id}/export?format=`), with download buttons on the document screen. **Batch export is built too** (P4-08): `POST /v1/register` takes a period or a list of documents and answers with an Excel workbook with six sheets (Register, Lines, VAT by category, Stated totals, To check, Manifest), a zipped CSV bundle, JSON, PDF, HTML, Markdown or plain text, built from the same events and never stored; the Register screen previews it and downloads it. **Remaining:** the MCP export tool (P4-09), and the Excel and Word files have been checked by their XML, not opened in Excel or Word | The product's core promise. |
```

- [ ] **Step 4: Write the worklog entry**

Insert a new entry immediately after the `_Newest first. ..._` line at the top of `docs/WORKLOG.md`:

```markdown
## 2026-10-30 (part 81): a period register, and the files an accountant asks for
**Done**
- **One register from the stored events.** `POST /v1/register` takes a period (the invoice date is read from the events, so an invoice uploaded months late still lands in its own month) or a list of document ids, and answers with the register as a file. Every figure comes from the append-only events. Nothing is stored and no model is called.
- **Six sheets.** The Excel workbook (`clearcut-export`) carries Register, Lines, VAT by category, Stated totals, To check and Manifest. Amounts are real numeric cells, never formulas.
- **The other shapes.** A zipped CSV bundle with one table per file and `manifest.json`, plus JSON, PDF, HTML, Markdown and plain text. A CSV cell that starts with `=`, `+`, `-` or `@` gets an apostrophe in front, and HTML and PDF escape text from the document.
- **Separate money, never netted.** Credit notes stay in their own group, currencies are never added together, undated documents are listed as exceptions instead of being put in a month, and documents that need review have their figures shown apart from the ones that are ready.
- **The automation rate is on the manifest**, with the counts behind it, so a period shows what it cost a person to finish.
- **A Register screen.** A period picker or the documents ticked in the list; a preview that agrees with the downloaded JSON; buttons for Excel, the CSV bundle, JSON and PDF.
- **Bulk intake.** Up to 20 files at once, uploaded one after another, with a row per file. A retry never uploads a file twice: if the upload succeeded, the row follows the document instead.
- **Tests:** core rules, the store selection against two tenants, every layout, the endpoint (content types, filenames, 400/403/404/503), the Register screen markers and bulk intake; the workspace is green against local Postgres, clippy and fmt clean, and the OpenAPI contract is current.
- **Docs:** PHASES (P4-08 ✅), ARCHITECTURE (the export crate, the endpoint), LAUNCH (row 3), this entry.
**Not done:** the Excel workbook has been checked by its XML and the PDF by its bytes, not opened by hand in Excel or a PDF viewer; the register has been tried on synthetic events and the local stack, not on a customer's documents; native accounting files, a VAT201 form, input and output classification, statement support, line categorisation, scheduled or emailed registers and stored export archives remain (spec §14); the MCP export tool (P4-09) is still open.
```

- [ ] **Step 5: Run the gate**

```bash
cargo fmt --all
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
DATABASE_URL=postgres://clearcut@127.0.0.1:54329/clearcut cargo test --workspace
cargo test -p clearcut-server openapi
```

Expected: every command exits 0. The last test includes the staleness check that `contracts/openapi.json` matches the generated document.

- [ ] **Step 6: Commit the docs, push and merge**

```bash
git switch -c docs/export-readiness
git add docs/PHASES.md docs/ARCHITECTURE.md docs/LAUNCH.md docs/WORKLOG.md
git commit -m "docs: record the register and the files it makes"
git push -u origin docs/export-readiness
gh pr create --title "docs: record the register and the files it makes" --body "$(cat <<'EOF'
## Summary
- close P4-08 and record the export crate, the register endpoint and the Register screen
- add the worklog entry for the register and bulk intake

## Test plan
- [x] cargo fmt --all --check
- [x] cargo clippy --workspace --all-targets -- -D warnings
- [x] DATABASE_URL=postgres://clearcut@127.0.0.1:54329/clearcut cargo test --workspace
- [x] cargo test -p clearcut-server openapi
EOF
)"
gh pr checks --watch
gh pr merge --squash --delete-branch
```

- [ ] **Step 7: Deploy and check the live host**

Run on the VM:

```bash
ssh ubuntu@145.241.100.85 'cd ~/clearcut && git pull && docker compose --profile https up -d --build server'
```

Then, from this computer:

```bash
curl -fsS https://145-241-100-85.sslip.io/health
curl -s -o /dev/null -w '%{http_code}\n' https://145-241-100-85.sslip.io/
curl -s -o /dev/null -w '%{http_code}\n' -X POST https://145-241-100-85.sslip.io/v1/register
curl -fsS https://145-241-100-85.sslip.io/app.js | grep -c 'function registerPage'
curl -fsS https://145-241-100-85.sslip.io/app.js | grep -c 'function bulkCard'
```

Expected: the health JSON, then `200`, then `401` (the key is checked before the body), then `1`, then `1`. A `502` or `503` means the image did not come up; read `docker compose logs server` on the VM before going further. Do not send the admin key to the live register endpoint: the checks above need no key, and a real read would touch live data.

- [ ] **Step 8: Record the merge and the live result, then merge that**

Add this line to the worklog entry, above `**Not done:**`:

```markdown
- **Shipped and deployed.** Squash-merged as the six slice PRs and this docs PR; the VM pulled `main`, `docker compose --profile https up -d --build server` rebuilt the server image, and `/health` answered, `/` returned 200 and an empty `POST /v1/register` returned 401; the served `app.js` carries `function registerPage` and `function bulkCard`.
```

Name the merged PR numbers in that sentence (the numbers come from `gh pr list --state merged --limit 10 --json number,title`), and make every claim match what Step 7 printed. Do not claim a live register was made: no check in this plan sends a document or a key to the live host. Then:

```bash
git switch -c docs/worklog-register
git add docs/WORKLOG.md
git commit -m "docs(worklog): the register reached the live server"
git push -u origin docs/worklog-register
gh pr create --title "docs(worklog): the register reached the live server" --body "$(cat <<'EOF'
## Summary
- record the merge and the live check for the register and bulk intake

## Test plan
- docs only; the live checks were made after the feature PRs merged
EOF
)"
gh pr checks --watch
gh pr merge --squash --delete-branch
```
