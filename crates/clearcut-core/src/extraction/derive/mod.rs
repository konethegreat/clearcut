// Written by Kone Tshivhinda & Claude
//! Turns an [`Extraction`] into cited [events](crate::event) (ADR-0004: events
//! are the source of truth; narration and export are derived from them).
//!
//! What becomes an event:
//! - **Printed facts** (supplier, each line, the stated totals, the due date, a
//!   credit-note reference) are attributed to the extractor and cite the page
//!   they were read from, with the extractor's confidence.
//! - **Calculated facts** (VAT per tax category) and **findings** from
//!   [`validate`] (a wrong line total, a wrong printed total, a missing field)
//!   are attributed to [`Extractor::System`]. A finding cites the printed
//!   value it is about; a finding about something missing cites page 1.
//!
//! IDs are deterministic and carry the extraction run
//! (`<document>:r1:line-2`, `<document>:r1:totals-mismatch-vat`,
//! `<document>:r1:issue-1`), so deriving twice gives identical events and the
//! core never needs a random number or a clock.
//!
//! **Re-extraction** (ADR-0006): when a document is extracted again, run 2's
//! events get new IDs and each one supersedes the current event with the same
//! slug from an earlier run. Limits, deliberately visible rather than hidden:
//! a fact the new run no longer finds has nothing to supersede its old event,
//! so the old one stays current; and a fact a person has confirmed or
//! corrected is never superseded by a machine re-run, so both stay current for
//! a reviewer to reconcile.
//!
//! The extraction is untrusted. Nothing is corrected: a line that is wrong is
//! still narrated as printed, next to an event that says it is wrong. A fact
//! that cannot be represented, such as amounts in two currencies, becomes a
//! compliance-issue event instead of an error. An invalid location from the
//! extractor *is* an error, so a bad extraction is rejected visibly rather
//! than quietly losing its citations.

use super::{validate, Extraction, Finding, Sourced, Validation};
use crate::error::Result;
use crate::event::{
    BalanceRole, Citation, Confidence, DocumentId, Event, EventId, EventParts, EventPayload, Extractor,
    TotalField,
};
use crate::{Money, TaxCategory};
use std::collections::HashMap;
use std::num::NonZeroU32;
use time::Date;

/// The events derived from one extraction, with the validation they came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Derived {
    pub validation: Validation,
    pub events: Vec<Event>,
}

/// Validates `extraction` and derives its events: printed facts first, then
/// calculated VAT, then one event per finding, each group in a fixed order.
///
/// `run` numbers the extraction attempt for this document (1 for the first,
/// 2 for a re-extraction with a better model, and so on). `earlier` holds the
/// events that already exist for the document; each new event supersedes the
/// matching event from an earlier run, as described on the module.
pub fn derive_events(extraction: &Extraction, run: NonZeroU32, earlier: &[Event]) -> Result<Derived> {
    let validation = validate(extraction);
    let mut out = Builder {
        document: &extraction.document,
        run,
        date: extraction.invoice_date.as_ref().map(|d| d.value),
        replaces: replaceable_events(&extraction.document, run, earlier),
        events: Vec::new(),
    };
    let x = extraction;

    if let Some(number) = &x.invoice_number {
        out.push(
            "invoice-number",
            EventPayload::InvoiceNumberStated {
                number: number.value.clone(),
            },
            Origin::printed(x, number),
        )?;
    }

    if let Some(supplier) = &x.supplier {
        out.push(
            "supplier",
            EventPayload::SupplierIdentified {
                name: supplier.value.name.clone(),
                vat_number: supplier.value.vat_number.clone(),
            },
            Origin::printed(x, supplier),
        )?;
    }

    if let Some(recipient) = &x.recipient {
        out.push(
            "recipient",
            EventPayload::RecipientIdentified {
                name: recipient.value.name.clone(),
                vat_number: recipient.value.vat_number.clone(),
            },
            Origin::printed(x, recipient),
        )?;
    }

    for (index, sourced) in x.lines.iter().enumerate() {
        let number = line_number(index);
        // A line in the wrong currency is reported by a finding below instead.
        if validation
            .findings
            .contains(&Finding::CurrencyMismatch { line: number })
        {
            continue;
        }
        let l = &sourced.value;
        out.push(
            &format!("line-{number}"),
            EventPayload::LineItemBilled {
                description: l.description.clone(),
                quantity_milli: l.quantity_milli,
                unit_price: l.unit_price,
                line_total: l.line_total,
                tax_category: l.tax,
            },
            Origin::printed(x, sourced),
        )?;
    }

    // VAT is only calculated when the lines can be trusted.
    if validation.recomputed.is_some() {
        for category in [
            TaxCategory::Standard,
            TaxCategory::ZeroRated,
            TaxCategory::Exempt,
            TaxCategory::OutOfScope,
        ] {
            let group: Vec<&Sourced<_>> = x.lines.iter().filter(|l| l.value.tax == category).collect();
            let Some(first) = group.first() else { continue };
            let mut taxable = Money::zero(x.currency);
            for l in &group {
                taxable = taxable.checked_add(l.value.unit_price.mul_milli(l.value.quantity_milli)?)?;
            }
            let weakest = group
                .iter()
                .map(|l| l.confidence)
                .min()
                .unwrap_or(first.confidence);
            out.push(
                &format!("vat-{}", tax_slug(category)),
                EventPayload::VatCalculated {
                    category,
                    taxable,
                    rate_bps: category.rate_bps(),
                    vat: taxable.percent_bps(category.rate_bps())?,
                },
                Origin::calculated(first.citation(&x.document), weakest),
            )?;
        }
    }

    if let Some(totals) = &x.stated_totals {
        out.push(
            "totals-stated",
            EventPayload::TotalsStated {
                net: totals.value.net,
                vat: totals.value.vat,
                gross: totals.value.gross,
            },
            Origin::printed(x, totals),
        )?;
    }
    if let Some(due) = &x.due {
        out.push(
            "due",
            EventPayload::DueDateStated {
                due: due.value.due,
                terms: due.value.terms.clone(),
            },
            Origin::printed(x, due),
        )?;
    }
    if let Some(credit) = &x.credit_note {
        out.push(
            "credit-note",
            EventPayload::CreditNoteReferenced {
                original_invoice_number: credit.value.original_invoice_number.clone(),
                amount: credit.value.amount,
            },
            Origin::printed(x, credit),
        )?;
    }

    // The page-continuation figures a long invoice prints (ADR-0022): one Info event per printed
    // figure, in page order, each cited to the page that printed it.
    for carried in &x.carried {
        let page = carried.page.get();
        if let Some(amount) = carried.value.brought_forward {
            out.push(
                &format!("brought-forward-{page}"),
                EventPayload::BalanceStated {
                    role: BalanceRole::BroughtForward,
                    amount,
                },
                Origin::printed(x, carried),
            )?;
        }
        if let Some(amount) = carried.value.carried_forward {
            out.push(
                &format!("carried-forward-{page}"),
                EventPayload::BalanceStated {
                    role: BalanceRole::CarriedForward,
                    amount,
                },
                Origin::printed(x, carried),
            )?;
        }
    }

    // One event per finding. Findings that are not about a printed value are
    // compliance issues, numbered in the order they were found.
    let mut issues = 0;
    for finding in &validation.findings {
        match finding {
            Finding::LineTotalMismatch {
                line,
                stated,
                recomputed,
            } => out.push(
                &format!("line-mismatch-{line}"),
                EventPayload::LineTotalMismatch {
                    line: *line,
                    stated: *stated,
                    recomputed: *recomputed,
                },
                Origin::about(x, line_source(x, *line)),
            )?,
            Finding::TotalsMismatch {
                field,
                stated,
                recomputed,
            } => out.push(
                &format!("totals-mismatch-{}", total_slug(*field)),
                EventPayload::TotalsMismatch {
                    field: *field,
                    stated: *stated,
                    recomputed: *recomputed,
                },
                Origin::about(
                    x,
                    x.stated_totals
                        .as_ref()
                        .map(|t| (t.citation(&x.document), t.confidence)),
                ),
            )?,
            Finding::CarriedBalanceMismatch {
                after_page,
                carried_forward,
                brought_forward,
            } => {
                let source = x
                    .carried
                    .iter()
                    .find(|c| c.page.get() == *after_page)
                    .map(|c| (c.citation(&x.document), c.confidence));
                out.push(
                    &format!("carried-mismatch-{after_page}"),
                    EventPayload::BalanceMismatch {
                        after_page: *after_page,
                        carried_forward: *carried_forward,
                        brought_forward: *brought_forward,
                    },
                    Origin::about(x, source),
                )?
            }
            other => {
                if let Some((field, message, source)) = issue_of(x, other) {
                    issues += 1;
                    out.push(
                        &format!("issue-{issues}"),
                        EventPayload::ComplianceIssueFound { field, message },
                        Origin::about(x, source),
                    )?;
                }
            }
        }
    }

    Ok(Derived {
        validation,
        events: out.events,
    })
}

/// Who produced an event, where it was read, and how sure that source was.
struct Origin {
    citation: Citation,
    extractor: Extractor,
    confidence: Confidence,
}

impl Origin {
    /// A fact printed on the document and read by the extractor.
    fn printed<T>(x: &Extraction, sourced: &Sourced<T>) -> Self {
        Self {
            citation: sourced.citation(&x.document),
            extractor: x.extractor.clone(),
            confidence: sourced.confidence,
        }
    }

    /// A fact ClearCut calculated from the cited source.
    fn calculated(citation: Citation, confidence: Confidence) -> Self {
        Self {
            citation,
            extractor: Extractor::System,
            confidence,
        }
    }

    /// A finding about `source`, or about the document as a whole (page 1,
    /// full confidence, since the check itself is exact) when there is none.
    fn about(x: &Extraction, source: Option<(Citation, Confidence)>) -> Self {
        match source {
            Some((citation, confidence)) => Self::calculated(citation, confidence),
            None => Self::calculated(
                Citation {
                    document: x.document.clone(),
                    page: std::num::NonZeroU32::MIN,
                    locator: None,
                },
                Confidence::CERTAIN,
            ),
        }
    }
}

struct Builder<'a> {
    document: &'a DocumentId,
    run: NonZeroU32,
    date: Option<Date>,
    /// For each slug, the earlier event a new event with that slug supersedes.
    replaces: HashMap<String, EventId>,
    events: Vec<Event>,
}

/// Splits `<document>:r<run>:<slug>` into the run and the slug. `prefix` is
/// `<document>:r`. Anything else, such as a reviewer's correction, gives `None`.
fn split_run_id<'a>(id: &'a str, prefix: &str) -> Option<(u32, &'a str)> {
    let (run, slug) = id.strip_prefix(prefix)?.split_once(':')?;
    Some((run.parse().ok()?, slug))
}

/// For each slug, the event from an earlier run that a new event with that slug
/// should supersede: the *current head* of that slug's chain, so run 3
/// supersedes run 2, not run 1. A head that a person confirmed or corrected
/// ([`Extractor::Human`]) is never superseded by a machine re-run. Events of
/// other documents and of this run or later runs are ignored.
fn replaceable_events(document: &DocumentId, run: NonZeroU32, earlier: &[Event]) -> HashMap<String, EventId> {
    let prefix = format!("{}:r", document.as_str());
    // Events of this run or later runs play no part, not even as the
    // "superseder" that a chain is followed into.
    let is_earlier = |event: &Event| {
        split_run_id(event.id().as_str(), &prefix).is_none_or(|(event_run, _)| event_run < run.get())
    };
    // superseded event -> the event that superseded it
    let replaced_by: HashMap<&EventId, &Event> = earlier
        .iter()
        .filter(|event| is_earlier(event))
        .filter_map(|event| event.supersedes().map(|old| (old, event)))
        .collect();

    let mut best: HashMap<&str, (u32, &EventId)> = HashMap::new();
    for event in earlier {
        let Some((event_run, slug)) = split_run_id(event.id().as_str(), &prefix) else {
            continue;
        };
        if event_run >= run.get() {
            continue;
        }
        // Follow corrections forward to the head (bounded, in case of a cycle).
        let mut head = event;
        for _ in 0..earlier.len() {
            match replaced_by.get(head.id()) {
                Some(next) => head = next,
                None => break,
            }
        }
        if matches!(head.extractor(), Extractor::Human { .. }) {
            continue;
        }
        if best.get(slug).is_none_or(|(best_run, _)| event_run > *best_run) {
            best.insert(slug, (event_run, head.id()));
        }
    }
    best.into_iter()
        .map(|(slug, (_, id))| (slug.to_string(), id.clone()))
        .collect()
}

impl Builder<'_> {
    /// Adds an event with ID `<document>:r<run>:<slug>`, superseding the earlier
    /// event for that slug if there is one. If its amounts use more than one
    /// currency it cannot be an event of its own kind, so a compliance-issue
    /// event with the same ID is added in its place.
    fn push(&mut self, slug: &str, payload: EventPayload, origin: Origin) -> Result<()> {
        let payload = if payload.has_single_currency() {
            payload
        } else {
            EventPayload::ComplianceIssueFound {
                field: slug.to_string(),
                message: "amounts in one fact must share a currency".to_string(),
            }
        };
        self.events.push(Event::new(EventParts {
            id: EventId::new(format!("{}:r{}:{slug}", self.document.as_str(), self.run)),
            date: self.date,
            payload,
            citation: origin.citation,
            extractor: origin.extractor,
            confidence: origin.confidence,
            supersedes: self.replaces.get(slug).cloned(),
        })?);
        Ok(())
    }
}

/// The source (citation and confidence) of a 1-based line.
fn line_source(x: &Extraction, line: u32) -> Option<(Citation, Confidence)> {
    let index = usize::try_from(line).ok()?.checked_sub(1)?;
    let sourced = x.lines.get(index)?;
    Some((sourced.citation(&x.document), sourced.confidence))
}

/// A compliance issue: its field, its message, and where it was read from.
type IssueText = (String, String, Option<(Citation, Confidence)>);

/// The issue a finding becomes, or `None` for the two findings that have an
/// event kind of their own (a wrong line total, a wrong printed total).
fn issue_of(x: &Extraction, finding: &Finding) -> Option<IssueText> {
    let line_issue = |line: &u32, field: &str, message: &str| -> IssueText {
        (
            format!("line {line} {field}"),
            message.into(),
            line_source(x, *line),
        )
    };
    Some(match finding {
        Finding::NoLines => (
            "lines".into(),
            "no line items were read from the document".into(),
            None,
        ),
        Finding::CurrencyMismatch { line } => {
            line_issue(line, "currency", "line amounts are not in the document currency")
        }
        Finding::NonPositiveQuantity { line } => line_issue(line, "quantity", "quantity must be positive"),
        Finding::AmountOutOfRange { line } => {
            line_issue(line, "amounts", "amounts are too large to calculate")
        }
        Finding::TotalsOutOfRange => (
            "totals".into(),
            "the lines add up to more than can be calculated".into(),
            None,
        ),
        Finding::DueBeforeIssue => (
            "due_date".into(),
            "due date is before the invoice date".into(),
            x.due.as_ref().map(|d| (d.citation(&x.document), d.confidence)),
        ),
        Finding::Compliance(issue) => (issue.field.into(), issue.message.into(), None),
        Finding::LineTotalMismatch { .. }
        | Finding::TotalsMismatch { .. }
        | Finding::CarriedBalanceMismatch { .. } => return None,
    })
}

/// 1-based line number for a 0-based index. Saturates instead of panicking.
fn line_number(index: usize) -> u32 {
    u32::try_from(index + 1).unwrap_or(u32::MAX)
}

fn tax_slug(category: TaxCategory) -> &'static str {
    match category {
        TaxCategory::Standard => "standard",
        TaxCategory::ZeroRated => "zero-rated",
        TaxCategory::Exempt => "exempt",
        TaxCategory::OutOfScope => "out-of-scope",
    }
}

fn total_slug(field: TotalField) -> &'static str {
    match field {
        TotalField::Net => "net",
        TotalField::Vat => "vat",
        TotalField::Gross => "gross",
    }
}

#[cfg(test)]
mod tests;
