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
                tax_category(line.tax).to_string(),
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
                tax_category(group.category).to_string(),
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
