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
//! IDs are deterministic (`<document>:line-2`, `<document>:totals-mismatch-vat`,
//! `<document>:issue-1`), so deriving twice gives identical events and the core
//! never needs a random number or a clock. Storage may assign its own IDs.
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
    Citation, Confidence, DocumentId, Event, EventId, EventParts, EventPayload, Extractor, TotalField,
};
use crate::{Money, TaxCategory};
use time::Date;

/// The events derived from one extraction, with the validation they came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Derived {
    pub validation: Validation,
    pub events: Vec<Event>,
}

/// Validates `extraction` and derives its events: printed facts first, then
/// calculated VAT, then one event per finding, each group in a fixed order.
pub fn derive_events(extraction: &Extraction) -> Result<Derived> {
    let validation = validate(extraction);
    let mut out = Builder {
        document: &extraction.document,
        date: extraction.invoice_date.as_ref().map(|d| d.value),
        events: Vec::new(),
    };
    let x = extraction;

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
    date: Option<Date>,
    events: Vec<Event>,
}

impl Builder<'_> {
    /// Adds an event with ID `<document>:<slug>`. If its amounts use more than
    /// one currency it cannot be an event of its own kind, so a
    /// compliance-issue event with the same ID is added in its place.
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
            id: EventId::new(format!("{}:{slug}", self.document.as_str())),
            date: self.date,
            payload,
            citation: origin.citation,
            extractor: origin.extractor,
            confidence: origin.confidence,
            supersedes: None,
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
        Finding::LineTotalMismatch { .. } | Finding::TotalsMismatch { .. } => return None,
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
