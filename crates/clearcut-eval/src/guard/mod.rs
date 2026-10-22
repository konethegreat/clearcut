// Written by Kone Tshivhinda & Claude
//! The repository guard (ADR-0010, privacy rule 4).
//!
//! This repository is public. Real invoices are personal and financial
//! information and must never be committed, and a secrets scanner cannot see a
//! PDF. So CI runs this guard, which fails if
//!
//! - a **document** (a PDF, an image, an Office file, an archive, an email) is
//!   anywhere in the repository outside the **approved fixture folders** listed
//!   in `gold-guard.json`, whatever its extension (a file that *starts like* a
//!   document is caught too);
//! - a gold-set **`label.json`** is outside those folders, or inside one but not
//!   marked synthetic (so a real case cannot hide among the samples);
//! - `.gitignore` does not keep `/gold/` and `/gold-*/` out (a second net, for
//!   a gold folder created inside a working copy by mistake).
//!
//! Adding an approved folder is a deliberate act: it means editing
//! `gold-guard.json`, which shows up in review. Build output, dependencies and
//! git's own folder are not looked into. Links are never followed.
//!
//! The report names the offending *path*, which is already visible in the
//! repository, and never anything from inside a file.

use crate::label::{Label, ParseError, Tier};
use serde::Deserialize;
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::Path;
use std::process::Command;

/// The configuration file, at the root of the repository.
pub const CONFIG_FILE: &str = "gold-guard.json";

/// `.gitignore` must contain these lines.
pub const REQUIRED_GITIGNORE: [&str; 2] = ["/gold/", "/gold-*/"];

/// File extensions (lower case) that mean a document, an image or a bundle.
pub const DOCUMENT_EXTENSIONS: [&str; 23] = [
    "pdf", "png", "jpg", "jpeg", "tif", "tiff", "gif", "bmp", "webp", "heic", "heif", "doc", "docx", "xls",
    "xlsx", "odt", "ods", "zip", "tar", "gz", "7z", "eml", "msg",
];

/// Folders whose contents are never looked at: build output, dependencies,
/// caches and git's own data.
const SKIPPED_FOLDERS: [&str; 9] = [
    ".git",
    "target",
    "node_modules",
    ".venv",
    "__pycache__",
    ".mypy_cache",
    ".ruff_cache",
    ".pytest_cache",
    "dist",
];

/// How many bytes at the start of a file are looked at.
const HEAD_BYTES: usize = 16;

/// The first bytes of common document, image, Office and archive formats.
const MAGIC: [&[u8]; 11] = [
    b"%PDF-",
    b"\x89PNG\r\n\x1a\n",
    b"\xff\xd8\xff",
    b"GIF87a",
    b"GIF89a",
    b"II*\x00",
    b"MM\x00*",
    b"PK\x03\x04",
    // Old Office files (.doc, .xls, .ppt, .msg) are OLE compound files.
    b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1",
    b"\x1f\x8b",
    b"7z\xbc\xaf\x27\x1c",
];

/// Labels are small; a bigger "label" in an approved folder is not read.
const MAX_LABEL_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuardConfig {
    /// Folders (relative to the repository root, `/`-separated) where documents
    /// and synthetic cases are allowed.
    pub approved_folders: Vec<String>,
}

/// Why the guard could not run. Positions and names only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GuardError {
    #[error("{CONFIG_FILE} is missing from the repository root")]
    ConfigMissing,
    #[error("{CONFIG_FILE} is not valid: {0}")]
    ConfigInvalid(ParseError),
    #[error("approved folder number {index} must be a plain relative path of letters, digits, dot, dash and underscore")]
    BadApprovedFolder { index: usize },
    #[error("the repository cannot be read ({0})")]
    Unreadable(io::ErrorKind),
}

impl GuardConfig {
    pub fn parse(bytes: &[u8]) -> Result<GuardConfig, GuardError> {
        let config: GuardConfig = serde_json::from_slice(bytes).map_err(|error| {
            let (line, column) = (error.line(), error.column());
            GuardError::ConfigInvalid(match error.classify() {
                serde_json::error::Category::Data => ParseError::WrongShape { line, column },
                _ => ParseError::NotJson { line, column },
            })
        })?;
        for (index, folder) in config.approved_folders.iter().enumerate() {
            if !is_plain_relative_path(folder) {
                return Err(GuardError::BadApprovedFolder { index });
            }
        }
        Ok(config)
    }

    fn approves(&self, path: &str) -> bool {
        self.approved_folders.iter().any(|folder| {
            path.strip_prefix(folder.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
        })
    }
}

/// `a`, `a/b`, `a/b.c-d_e`: no empty parts, no `.` or `..`, nothing outside
/// letters, digits, dot, dash and underscore.
fn is_plain_relative_path(path: &str) -> bool {
    path.split('/').all(|part| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
    })
}

/// One thing that must not be in the repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Violation {
    Document { path: String },
    RenamedDocument { path: String },
    Label { path: String },
    ApprovedLabelNotSynthetic { path: String },
    ApprovedLabelUnreadable { path: String },
    GitignoreMissing { entry: &'static str },
}

impl Violation {
    fn path(&self) -> &str {
        match self {
            Violation::Document { path }
            | Violation::RenamedDocument { path }
            | Violation::Label { path }
            | Violation::ApprovedLabelNotSynthetic { path }
            | Violation::ApprovedLabelUnreadable { path } => path,
            Violation::GitignoreMissing { .. } => ".gitignore",
        }
    }
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let path = self.path();
        match self {
            Violation::Document { .. } => write!(f, "{path}: a document file outside the approved fixture folders"),
            Violation::RenamedDocument { .. } => {
                write!(f, "{path}: looks like a document (by its first bytes) outside the approved fixture folders")
            }
            Violation::Label { .. } => write!(f, "{path}: a gold-set label outside the approved fixture folders"),
            Violation::ApprovedLabelNotSynthetic { .. } => {
                write!(f, "{path}: a label in an approved folder that is not synthetic")
            }
            Violation::ApprovedLabelUnreadable { .. } => {
                write!(f, "{path}: a label in an approved folder that cannot be read")
            }
            Violation::GitignoreMissing { entry } => write!(
                f,
                ".gitignore: the entry {entry} is missing, so a gold folder inside the repository could be committed"
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardReport {
    pub files_scanned: usize,
    pub violations: Vec<Violation>,
}

impl GuardReport {
    pub fn is_clean(&self) -> bool {
        self.violations.is_empty()
    }
}

/// Checks the repository at `root`.
pub fn guard(root: &Path) -> Result<GuardReport, GuardError> {
    let config = match fs::read(root.join(CONFIG_FILE)) {
        Ok(bytes) => GuardConfig::parse(&bytes)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Err(GuardError::ConfigMissing),
        Err(error) => return Err(GuardError::Unreadable(error.kind())),
    };

    let mut report = GuardReport {
        files_scanned: 0,
        violations: Vec::new(),
    };
    for file in list_files(root)? {
        report.files_scanned += 1;
        let name = file.path.rsplit('/').next().unwrap_or(&file.path).to_string();
        check_file(
            &root.join(&file.path),
            &file.path,
            &name,
            file.is_link,
            &config,
            &mut report,
        );
    }

    let gitignore = fs::read_to_string(root.join(".gitignore")).unwrap_or_default();
    for entry in REQUIRED_GITIGNORE {
        if !gitignore.lines().any(|line| line.trim() == entry) {
            report.violations.push(Violation::GitignoreMissing { entry });
        }
    }

    // Stable sort: violations on one path keep the order they were found in.
    report.violations.sort_by(|a, b| a.path().cmp(b.path()));
    Ok(report)
}

/// A file to judge: its path relative to the root, `/`-separated.
struct Listed {
    path: String,
    is_link: bool,
}

/// The files that are, or could become, part of the repository. In a git
/// working copy that is exactly what git would commit (tracked files, plus
/// untracked ones that are not ignored), so a local scratch file that git is
/// told to ignore is not a finding. Anywhere else (a folder that is not a git
/// working copy, or when git cannot be run) every file is visited, apart from
/// build output, dependencies and caches.
fn list_files(root: &Path) -> Result<Vec<Listed>, GuardError> {
    if root.join(".git").exists() {
        if let Some(files) = git_files(root) {
            return Ok(files);
        }
    }
    let mut files = Vec::new();
    walk(root, "", &mut files)?;
    Ok(files)
}

fn git_files(root: &Path) -> Option<Vec<Listed>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z", "--cached", "--others", "--exclude-standard"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let mut files = Vec::new();
    for raw in output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|raw| !raw.is_empty())
    {
        let path = String::from_utf8_lossy(raw).into_owned();
        // A file that git lists but that is gone from the working copy is not a finding.
        let Ok(metadata) = fs::symlink_metadata(root.join(&path)) else {
            continue;
        };
        if metadata.is_dir() {
            continue; // a submodule or similar
        }
        files.push(Listed {
            path,
            is_link: metadata.file_type().is_symlink(),
        });
    }
    Some(files)
}

/// Visits every file under `folder` (whose path relative to the root is
/// `relative`, empty for the root itself). Links are not followed.
fn walk(folder: &Path, relative: &str, files: &mut Vec<Listed>) -> Result<(), GuardError> {
    let unreadable = |error: io::Error| GuardError::Unreadable(error.kind());
    for entry in fs::read_dir(folder).map_err(unreadable)? {
        let entry = entry.map_err(unreadable)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = if relative.is_empty() {
            name.clone()
        } else {
            format!("{relative}/{name}")
        };
        let file_type = entry.file_type().map_err(unreadable)?;
        if file_type.is_dir() {
            if !SKIPPED_FOLDERS.contains(&name.as_str()) {
                walk(&entry.path(), &path, files)?;
            }
        } else {
            files.push(Listed {
                path,
                is_link: file_type.is_symlink(),
            });
        }
    }
    Ok(())
}

fn check_file(
    full: &Path,
    path: &str,
    name: &str,
    is_link: bool,
    config: &GuardConfig,
    report: &mut GuardReport,
) {
    let approved = config.approves(path);
    let lower = name.to_ascii_lowercase();
    let extension = lower.rsplit_once('.').map(|(_, extension)| extension);

    if lower == "label.json" {
        if !approved {
            report.violations.push(Violation::Label {
                path: path.to_string(),
            });
        } else if !is_link {
            match read_label(full) {
                Some(label) if label.provenance.tier == Tier::Synthetic => {}
                Some(_) => report.violations.push(Violation::ApprovedLabelNotSynthetic {
                    path: path.to_string(),
                }),
                None => report.violations.push(Violation::ApprovedLabelUnreadable {
                    path: path.to_string(),
                }),
            }
        }
        return;
    }
    if approved {
        return;
    }
    if extension.is_some_and(|extension| DOCUMENT_EXTENSIONS.contains(&extension)) {
        report.violations.push(Violation::Document {
            path: path.to_string(),
        });
    } else if !is_link && file_starts_like_a_document(full) {
        report.violations.push(Violation::RenamedDocument {
            path: path.to_string(),
        });
    }
}

fn read_label(path: &Path) -> Option<Label> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(MAX_LABEL_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_LABEL_BYTES {
        return None;
    }
    Label::parse(&bytes).ok()
}

/// Whether the file begins with the signature of a document format.
fn file_starts_like_a_document(path: &Path) -> bool {
    let mut head = [0u8; HEAD_BYTES];
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let Ok(read) = file.read(&mut head) else {
        return false;
    };
    starts_like_a_document(&head[..read])
}

/// Whether these first bytes look like a document, image or archive.
pub fn starts_like_a_document(head: &[u8]) -> bool {
    MAGIC.iter().any(|magic| head.starts_with(magic)) || is_webp(head) || is_bmp(head)
}

/// `RIFF`, a four-byte size, then `WEBP` (other RIFF files, such as WAV audio, are not images).
fn is_webp(head: &[u8]) -> bool {
    head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP"
}

/// `BM` is only two letters, so a bitmap must also have its reserved bytes
/// zero and a known size of the info header that follows (otherwise text
/// that starts with "BM" would be taken for a picture).
fn is_bmp(head: &[u8]) -> bool {
    head.len() >= 16
        && &head[..2] == b"BM"
        && head[6..10] == [0, 0, 0, 0]
        && matches!(head[14], 12 | 40 | 52 | 56 | 64 | 108 | 124)
        && head[15] == 0
}

#[cfg(test)]
mod tests;
