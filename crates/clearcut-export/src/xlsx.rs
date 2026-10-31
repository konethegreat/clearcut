// Written by Kone Tshivhinda & Claude
//! The Excel workbook: Summary, Events, Line items, Checks and Sources.
//!
//! Amounts are **real numeric cells** (so a person can add them up), shown to two decimals; the
//! exact integer minor units are what they are made from. Every piece of text goes in with
//! `write_string`, which stores plain text and never a formula, so text read off an uploaded
//! document cannot run in the reader's spreadsheet.

use clearcut_core::export::{Amount, ExportModel};
use rust_xlsxwriter::{Format, Workbook, Worksheet, XlsxError};

/// A cell's number from exact minor units. Only here, for the spreadsheet's own number type; the
/// value is an exact multiple of a cent and stays exact for any amount a spreadsheet can show.
pub(crate) fn units(amount: &Amount) -> f64 {
    amount.minor as f64 / 100.0
}

pub(crate) fn head(
    sheet: &mut Worksheet,
    bold: &Format,
    titles: &[&str],
    widths: &[f64],
) -> Result<(), XlsxError> {
    for (col, title) in titles.iter().enumerate() {
        let col = col as u16;
        sheet.write_string_with_format(0, col, *title, bold)?;
        sheet.set_column_width(col, widths.get(col as usize).copied().unwrap_or(14.0))?;
    }
    sheet.set_freeze_panes(1, 0)?;
    Ok(())
}

pub fn xlsx(model: &ExportModel) -> Result<Vec<u8>, XlsxError> {
    let bold = Format::new().set_bold();
    let title = Format::new().set_bold().set_font_size(14);
    let money = Format::new().set_num_format("#,##0.00");
    let percent = Format::new().set_num_format("0%");
    let wrap = Format::new().set_text_wrap();

    let mut book = Workbook::new();

    // ---- Summary
    {
        let sheet = book.add_worksheet();
        sheet.set_name("Summary")?;
        sheet.set_column_width(0, 34)?;
        sheet.set_column_width(1, 18)?;
        sheet.set_column_width(2, 10)?;
        sheet.set_column_width(3, 26)?;
        sheet.write_string_with_format(0, 0, model.document.filename.as_str(), &title)?;
        sheet.write_string(1, 0, "Status")?;
        sheet.write_string(1, 1, model.document.status.as_str())?;
        let mut row = 2u32;
        if let Some(pages) = model.document.page_count {
            sheet.write_string(row, 0, "Pages")?;
            sheet.write_number(row, 1, f64::from(pages))?;
            row += 1;
        }
        row += 1;
        sheet.write_string_with_format(row, 0, "Summary", &bold)?;
        row += 1;
        for sentence in &model.summary {
            sheet.write_string(row, 0, sentence.text.as_str())?;
            row += 1;
        }
        row += 1;
        if !model.totals.is_empty() {
            for (col, title) in ["Totals", "Amount", "Currency", "Basis"].iter().enumerate() {
                sheet.write_string_with_format(row, col as u16, *title, &bold)?;
            }
            row += 1;
            for total in &model.totals {
                sheet.write_string(row, 0, total.label.as_str())?;
                sheet.write_number_with_format(row, 1, units(&total.amount), &money)?;
                sheet.write_string(row, 2, total.amount.currency.as_str())?;
                sheet.write_string(row, 3, total.basis)?;
                row += 1;
            }
            row += 1;
        }
        sheet.write_string(row, 0, model.notice)?;
    }

    // ---- Events
    {
        let sheet = book.add_worksheet();
        sheet.set_name("Events")?;
        head(
            sheet,
            &bold,
            &[
                "Event",
                "Date",
                "Group",
                "Type",
                "Severity",
                "Sure",
                "Page",
                "Statement",
            ],
            &[34.0, 12.0, 12.0, 24.0, 10.0, 8.0, 6.0, 90.0],
        )?;
        for (i, event) in model.events.iter().enumerate() {
            let row = i as u32 + 1;
            sheet.write_string(row, 0, event.id.as_str())?;
            sheet.write_string(row, 1, event.date.as_deref().unwrap_or(""))?;
            sheet.write_string(row, 2, event.group.as_str())?;
            sheet.write_string(row, 3, event.kind.as_str())?;
            sheet.write_string(row, 4, event.severity.as_str())?;
            sheet.write_number_with_format(
                row,
                5,
                f64::from(event.confidence_per_mille) / 1000.0,
                &percent,
            )?;
            sheet.write_number(row, 6, f64::from(event.page))?;
            sheet.write_string_with_format(row, 7, event.text.as_str(), &wrap)?;
        }
    }

    // ---- Line items
    {
        let sheet = book.add_worksheet();
        sheet.set_name("Line items")?;
        head(
            sheet,
            &bold,
            &[
                "No",
                "Description",
                "Quantity",
                "Unit price",
                "Line total",
                "Currency",
                "Tax",
                "Page",
            ],
            &[5.0, 50.0, 10.0, 14.0, 14.0, 9.0, 16.0, 6.0],
        )?;
        for (i, line) in model.lines.iter().enumerate() {
            let row = i as u32 + 1;
            sheet.write_number(row, 0, f64::from(line.number))?;
            sheet.write_string_with_format(row, 1, line.description.as_str(), &wrap)?;
            match line.quantity.parse::<f64>() {
                Ok(quantity) => sheet.write_number(row, 2, quantity)?,
                Err(_) => sheet.write_string(row, 2, line.quantity.as_str())?,
            };
            sheet.write_number_with_format(row, 3, units(&line.unit_price), &money)?;
            sheet.write_number_with_format(row, 4, units(&line.line_total), &money)?;
            sheet.write_string(row, 5, line.line_total.currency.as_str())?;
            sheet.write_string(row, 6, line.tax.as_str())?;
            sheet.write_number(row, 7, f64::from(line.page))?;
        }
    }

    // ---- Checks
    {
        let sheet = book.add_worksheet();
        sheet.set_name("Checks")?;
        head(
            sheet,
            &bold,
            &["Severity", "Statement", "Event", "Page"],
            &[10.0, 100.0, 34.0, 6.0],
        )?;
        for (i, check) in model.checks.iter().enumerate() {
            let row = i as u32 + 1;
            sheet.write_string(row, 0, check.severity.as_str())?;
            sheet.write_string_with_format(row, 1, check.text.as_str(), &wrap)?;
            sheet.write_string(row, 2, check.event_id.as_str())?;
            sheet.write_number(row, 3, f64::from(check.page))?;
        }
        if model.checks.is_empty() {
            sheet.write_string(1, 1, "Nothing was found wrong.")?;
        }
    }

    // ---- Sources
    {
        let sheet = book.add_worksheet();
        sheet.set_name("Sources")?;
        head(
            sheet,
            &bold,
            &["Page", "Events", "Least sure", "Read by"],
            &[6.0, 8.0, 12.0, 50.0],
        )?;
        for (i, source) in model.sources.iter().enumerate() {
            let row = i as u32 + 1;
            sheet.write_number(row, 0, f64::from(source.page))?;
            sheet.write_number(row, 1, f64::from(source.events))?;
            sheet.write_number_with_format(
                row,
                2,
                f64::from(source.lowest_confidence_per_mille) / 1000.0,
                &percent,
            )?;
            sheet.write_string(row, 3, source.read_by.join(", "))?;
        }
    }

    book.save_to_buffer()
}
