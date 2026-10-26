// Written by Kone Tshivhinda & Claude
//! The Word document: the report as an editable file, with real tables.

use clearcut_core::export::{ExportModel, ModelSentence};
use docx_rs::{
    AlignmentType, Docx, DocxError, Footer, Paragraph, Run, Table, TableCell, TableRow, WidthType,
};
use std::io::Cursor;

fn text(value: &str) -> Run {
    Run::new().add_text(value)
}

fn heading(value: &str, size: usize) -> Paragraph {
    Paragraph::new().add_run(Run::new().add_text(value).bold().size(size))
}

fn bullets(doc: Docx, sentences: &[ModelSentence]) -> Docx {
    sentences.iter().fold(doc, |doc, sentence| {
        doc.add_paragraph(
            Paragraph::new()
                .add_run(text("\u{2022} "))
                .add_run(text(&sentence.text))
                .add_run(
                    Run::new()
                        .add_text(format!("  [{}]", sentence.cites.join(", ")))
                        .size(16)
                        .color("5F6B73"),
                ),
        )
    })
}

fn cell(value: &str, bold: bool, right: bool) -> TableCell {
    let mut run = Run::new().add_text(value).size(18);
    if bold {
        run = run.bold();
    }
    let mut paragraph = Paragraph::new().add_run(run);
    if right {
        paragraph = paragraph.align(AlignmentType::Right);
    }
    TableCell::new().add_paragraph(paragraph)
}

/// A table with a bold header row. `numeric` says which columns are right-aligned.
fn table(head: &[&str], numeric: &[bool], widths: &[usize], rows: Vec<Vec<String>>) -> Table {
    let mut all = vec![TableRow::new(
        head.iter()
            .enumerate()
            .map(|(i, h)| cell(h, true, numeric.get(i) == Some(&true)))
            .collect(),
    )];
    for row in rows {
        all.push(TableRow::new(
            row.iter()
                .enumerate()
                .map(|(i, c)| cell(c, false, numeric.get(i) == Some(&true)))
                .collect(),
        ));
    }
    Table::new(all)
        .set_grid(widths.to_vec())
        .width(widths.iter().sum(), WidthType::Dxa)
}

pub fn docx(model: &ExportModel) -> Result<Vec<u8>, DocxError> {
    let mut doc = Docx::new()
        .footer(Footer::new().add_paragraph(
            Paragraph::new().add_run(Run::new().add_text(model.notice).size(14).color("5F6B73")),
        ))
        .add_paragraph(heading(&model.document.filename, 36))
        .add_paragraph(Paragraph::new().add_run(text(&format!(
            "Status: {}{}",
            model.document.status,
            model
                .document
                .page_count
                .map(|n| format!(" \u{b7} {n} page{}", if n == 1 { "" } else { "s" }))
                .unwrap_or_default()
        ))));

    if !model.summary.is_empty() {
        doc = bullets(doc.add_paragraph(heading("Summary", 28)), &model.summary);
    }
    if !model.sections.is_empty() {
        doc = doc.add_paragraph(heading("Report", 28));
        for section in &model.sections {
            if let Some(title) = &section.heading {
                doc = doc.add_paragraph(heading(title, 24));
            }
            doc = bullets(doc, &section.sentences);
        }
    }
    if !model.lines.is_empty() {
        doc = doc.add_paragraph(heading("Line items", 28)).add_table(table(
            &["No", "Description", "Quantity", "Unit price", "Line total", "Tax"],
            &[true, false, true, true, true, false],
            &[500, 3300, 900, 1400, 1400, 1100],
            model
                .lines
                .iter()
                .map(|l| {
                    vec![
                        l.number.to_string(),
                        l.description.clone(),
                        l.quantity.clone(),
                        l.unit_price.text.clone(),
                        l.line_total.text.clone(),
                        l.tax.clone(),
                    ]
                })
                .collect(),
        ));
    }
    if !model.totals.is_empty() {
        doc = doc.add_paragraph(heading("Totals", 28)).add_table(table(
            &["", "Amount", "Basis"],
            &[false, true, false],
            &[4000, 1800, 2800],
            model
                .totals
                .iter()
                .map(|t| vec![t.label.clone(), t.amount.text.clone(), t.basis.to_string()])
                .collect(),
        ));
    }
    if !model.checks.is_empty() {
        doc = doc
            .add_paragraph(heading("What the checks found", 28))
            .add_table(table(
                &["Severity", "Statement", "Page"],
                &[false, false, true],
                &[1200, 6600, 800],
                model
                    .checks
                    .iter()
                    .map(|c| vec![c.severity.clone(), c.text.clone(), c.page.to_string()])
                    .collect(),
            ));
    }
    if !model.events.is_empty() {
        doc = doc
            .add_paragraph(heading("Everything that was read", 28))
            .add_table(table(
                &["What", "Statement", "Page", "Sure"],
                &[false, false, true, true],
                &[1900, 5200, 700, 800],
                model
                    .events
                    .iter()
                    .map(|e| {
                        vec![
                            format!("{} \u{b7} {}", e.group, e.kind.replace('_', " ")),
                            e.text.clone(),
                            e.page.to_string(),
                            format!("{}%", e.confidence_per_mille / 10),
                        ]
                    })
                    .collect(),
            ));
    }

    let mut buffer = Cursor::new(Vec::new());
    doc.build().pack(&mut buffer)?;
    Ok(buffer.into_inner())
}
