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
    /// The canonical file, every field with exact minor units.
    Json,
    /// A self-contained page with no script.
    Html,
    /// Tables for a note, a wiki or a git repository.
    Markdown,
    /// Lines for a terminal, an email or a quick check.
    Text,
}

impl RegisterFormat {
    /// Every format, in the order the API lists them.
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

    /// The file extension, which is also the API name.
    pub fn extension(self) -> &'static str {
        match self {
            RegisterFormat::Json => "json",
            RegisterFormat::Html => "html",
            RegisterFormat::Markdown => "md",
            RegisterFormat::Text => "txt",
        }
    }

    /// The content type the file is served with.
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
