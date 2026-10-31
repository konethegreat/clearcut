// Written by Kone Tshivhinda & Claude
//! The files a document is downloaded as: PDF, Excel, Word, HTML, plain text, Markdown, CSV and
//! JSON (P4-01 to P4-07).
//!
//! Every format is laid out from **one** model, [`clearcut_core::export::ExportModel`], built once
//! from the stored events and nothing else, so the formats cannot disagree with each other. This
//! crate only lays it out; it decides nothing about what a document says.
//!
//! Text that came off an uploaded document is data. HTML escapes every character that matters, a
//! spreadsheet cell holds it as plain text (never a formula), CSV neutralises a leading `=`, `+`,
//! `-` or `@`, and the PDF and Word files carry it as text only.

mod docx;
mod pdf;
mod register;
mod text;
mod xlsx;

use clearcut_core::event::Event;
use clearcut_core::export::build;
pub use clearcut_core::export::{DocumentFacts, ExportModel};
pub use register::{render_register, RegisterFormat};

/// The formats a document can be downloaded as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Json,
    Csv,
    Markdown,
    Html,
    Text,
    Xlsx,
    Docx,
    Pdf,
}

impl Format {
    pub const ALL: [Format; 8] = [
        Format::Json,
        Format::Csv,
        Format::Markdown,
        Format::Html,
        Format::Text,
        Format::Xlsx,
        Format::Docx,
        Format::Pdf,
    ];

    /// The name used in the API: `json`, `csv`, `md`, `html`, `txt`, `xlsx`, `docx`, `pdf`.
    pub fn parse(name: &str) -> Option<Format> {
        Format::ALL.into_iter().find(|format| format.extension() == name)
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::Json => "json",
            Format::Csv => "csv",
            Format::Markdown => "md",
            Format::Html => "html",
            Format::Text => "txt",
            Format::Xlsx => "xlsx",
            Format::Docx => "docx",
            Format::Pdf => "pdf",
        }
    }

    pub fn content_type(self) -> &'static str {
        match self {
            Format::Json => "application/json",
            Format::Csv => "text/csv; charset=utf-8",
            Format::Markdown => "text/markdown; charset=utf-8",
            Format::Html => "text/html; charset=utf-8",
            Format::Text => "text/plain; charset=utf-8",
            Format::Xlsx => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            Format::Docx => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            Format::Pdf => "application/pdf",
        }
    }
}

/// What an export is made from.
pub struct ExportInput<'a> {
    pub document: DocumentFacts,
    /// The document's events, oldest first, as stored.
    pub events: &'a [Event],
}

/// A finished file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    pub content_type: &'static str,
    pub extension: &'static str,
    pub bytes: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("the {0} file could not be made")]
    Failed(&'static str),
}

/// Makes the file.
pub fn render(format: Format, input: &ExportInput<'_>) -> Result<Rendered, ExportError> {
    let model = build(input.document.clone(), input.events);
    let bytes = match format {
        Format::Json => text::json(&model, input.events).into_bytes(),
        Format::Csv => text::csv(&model).into_bytes(),
        Format::Markdown => text::markdown(&model).into_bytes(),
        Format::Html => text::html(&model).into_bytes(),
        Format::Text => text::plain(&model).into_bytes(),
        Format::Xlsx => xlsx::xlsx(&model).map_err(|_| ExportError::Failed("Excel"))?,
        Format::Docx => docx::docx(&model).map_err(|_| ExportError::Failed("Word"))?,
        Format::Pdf => pdf::pdf(&model).map_err(|_| ExportError::Failed("PDF"))?,
    };
    Ok(Rendered {
        content_type: format.content_type(),
        extension: format.extension(),
        bytes,
    })
}

#[cfg(test)]
mod tests;
