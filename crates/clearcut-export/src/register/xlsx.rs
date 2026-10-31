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

/// A money cell, left empty when the document stated no such amount.
fn money(
    sheet: &mut Worksheet,
    row: u32,
    col: u16,
    amount: Option<&Amount>,
    format: &Format,
) -> Result<(), XlsxError> {
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
                11.0, 30.0, 16.0, 14.0, 12.0, 14.0, 9.0, 13.0, 13.0, 13.0, 6.0, 6.0, 40.0, 30.0, 20.0, 40.0,
                38.0, 28.0,
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
            sheet.write_string(at, 8, tax_category(line.tax))?;
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
                sheet.write_string(at, 3, tax_category(group.category))?;
                sheet.write_number_with_format(at, 4, f64::from(group.rate_bps) / 10_000.0, &percent)?;
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
            (
                "Left out, awaiting review",
                manifest.awaiting_left_out.to_string(),
            ),
            ("Line items", manifest.lines.to_string()),
            (
                "No invoice date in the account",
                manifest.undated_in_tenant.to_string(),
            ),
            (
                "No invoice date in the selection",
                manifest.undated_in_selection.to_string(),
            ),
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
