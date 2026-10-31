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
    /// The canonical file, every field with exact minor units.
    Json,
    /// A self-contained page with no script.
    Html,
    /// Tables for a note, a wiki or a git repository.
    Markdown,
    /// Lines for a terminal, an email or a quick check.
    Text,
    /// A six-sheet Excel workbook.
    Xlsx,
    /// A zipped bundle of CSV files plus the manifest as JSON.
    CsvBundle,
    /// A PDF of the tables a finance team reads.
    Pdf,
}

impl RegisterFormat {
    /// Every format, in the order the API lists them.
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

    /// The file extension. The bundle's is `zip`, while its [`name`](Self::name) is `csv`.
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

    /// The content type the file is served with.
    pub fn content_type(self) -> &'static str {
        match self {
            RegisterFormat::Json => "application/json",
            RegisterFormat::Html => "text/html; charset=utf-8",
            RegisterFormat::Markdown => "text/markdown; charset=utf-8",
            RegisterFormat::Text => "text/plain; charset=utf-8",
            RegisterFormat::Xlsx => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
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
        RegisterFormat::Xlsx => xlsx::xlsx(register).map_err(|_| ExportError::Failed("register workbook"))?,
        RegisterFormat::CsvBundle => {
            csv::bundle(register).map_err(|_| ExportError::Failed("register CSV bundle"))?
        }
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
