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
                    tax_category(group.category).to_string(),
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
