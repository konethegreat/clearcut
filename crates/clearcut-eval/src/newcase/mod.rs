// Written by Kone Tshivhinda & Claude
//! `clearcut-gold new-case`: start a new case in a gold set (ADR-0010).
//!
//! Creating a case by hand means choosing the next id, copying the document
//! under a neutral name, computing its SHA-256 and writing a `label.json` in
//! the right shape: all easy to get slightly wrong. This does those steps and
//! leaves the part only a person can do, reading the document, undone.
//!
//! The new case is well formed but **visibly unfinished**: the label has no
//! lines, so `check` reports it, and a real document starts as "not anonymised"
//! until the labeller has done that and says so. The document's original file
//! name is never kept (it may name a client), only its extension.

use crate::hash::sha256_hex_of_reader;
use crate::label::{is_case_id, DocumentRef, Expect, Label, Provenance, Split, Tier, Truth, FORMAT};
use clearcut_core::Currency;
use std::fs;
use std::io;
use std::path::Path;
use time::Date;

/// What to create.
#[derive(Debug, Clone, Copy)]
pub struct NewCase<'a> {
    /// The gold set folder (created if it does not exist).
    pub gold: &'a Path,
    /// The document to copy in. It is never modified or moved.
    pub document: &'a Path,
    pub split: Split,
    pub tier: Tier,
    pub labelled_by: &'a str,
    /// The document's page count (a person counts; this tool does not read PDFs).
    pub pages: u32,
    pub consent_ref: Option<&'a str>,
    pub retain_until: Option<Date>,
}

/// Why a case could not be created. Never includes a name or a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum NewCaseError {
    #[error("the document cannot be used ({0})")]
    Source(io::ErrorKind),
    #[error("the document is empty")]
    EmptyDocument,
    #[error("the document needs a plain file extension of 1 to 5 letters or digits, such as .pdf")]
    BadExtension,
    #[error("the value of --{0} is not acceptable")]
    BadArgument(&'static str),
    #[error("the case could not be written ({0})")]
    Io(io::ErrorKind),
}

impl From<io::Error> for NewCaseError {
    fn from(error: io::Error) -> Self {
        NewCaseError::Io(error.kind())
    }
}

/// Creates the next case and returns its id (`g-0008`).
pub fn create(request: &NewCase<'_>) -> Result<String, NewCaseError> {
    if !(1..=10_000).contains(&request.pages) {
        return Err(NewCaseError::BadArgument("pages"));
    }
    let labeller = request.labelled_by;
    if labeller.trim().is_empty() || labeller.chars().count() > 64 || labeller.chars().any(char::is_control) {
        return Err(NewCaseError::BadArgument("labelled-by"));
    }
    if request
        .consent_ref
        .is_some_and(|reference| reference.trim().is_empty())
    {
        return Err(NewCaseError::BadArgument("consent-ref"));
    }

    // The source: a plain, non-empty file with a plain extension.
    let metadata =
        fs::symlink_metadata(request.document).map_err(|error| NewCaseError::Source(error.kind()))?;
    if !metadata.is_file() {
        return Err(NewCaseError::Source(io::ErrorKind::InvalidInput));
    }
    let extension = request
        .document
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .filter(|extension| {
            (1..=5).contains(&extension.len()) && extension.bytes().all(|b| b.is_ascii_alphanumeric())
        })
        .ok_or(NewCaseError::BadExtension)?;
    if metadata.len() == 0 {
        return Err(NewCaseError::EmptyDocument);
    }

    fs::create_dir_all(request.gold)?;
    let id = next_id(request.gold)?;
    let folder = request.gold.join(&id);
    // `create_dir` (not `create_dir_all`) fails if the folder already exists,
    // so two runs at once cannot share a case.
    fs::create_dir(&folder)?;

    let written = write_case(request, &folder, &id, &extension);
    if written.is_err() {
        let _ = fs::remove_dir_all(&folder);
    }
    written.map(|()| id)
}

fn write_case(request: &NewCase<'_>, folder: &Path, id: &str, extension: &str) -> Result<(), NewCaseError> {
    let file = format!("document.{extension}");
    let destination = folder.join(&file);
    fs::copy(request.document, &destination)?;
    let (sha256, _) = sha256_hex_of_reader(fs::File::open(&destination)?)?;

    let label = Label {
        format: FORMAT,
        case: id.to_string(),
        document: DocumentRef {
            file,
            sha256,
            pages: request.pages,
        },
        tags: Vec::new(),
        split: request.split,
        provenance: Provenance {
            tier: request.tier,
            consent_ref: request.consent_ref.map(str::to_string),
            // A real document must be anonymised before it counts; a person says so.
            anonymised: request.tier == Tier::Synthetic,
            retain_until: request.retain_until,
            labelled_by: request.labelled_by.to_string(),
            reviewed_by: None,
        },
        truth: Truth {
            currency: Currency::Zar,
            invoice_number: None,
            invoice_date: None,
            supplier: None,
            recipient: None,
            lines: Vec::new(),
            stated_totals: None,
            due: None,
            credit_note: None,
        },
        expect: Expect::default(),
    };
    let mut text =
        serde_json::to_string_pretty(&label).map_err(|_| NewCaseError::Io(io::ErrorKind::InvalidData))?;
    text.push('\n');
    fs::write(folder.join("label.json"), text)?;
    Ok(())
}

/// One more than the highest existing case number (folders named like cases).
fn next_id(gold: &Path) -> Result<String, NewCaseError> {
    let mut numbers = Vec::new();
    for entry in fs::read_dir(gold)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type()?.is_dir() && is_case_id(&name) {
            numbers.extend(name[2..].parse::<u64>());
        }
    }
    let highest = numbers.into_iter().max().unwrap_or(0);
    Ok(format!("g-{:04}", highest + 1))
}

#[cfg(test)]
mod tests;
