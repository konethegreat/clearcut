// Written by Kone Tshivhinda & Claude
//! `clearcut-gold check`: is this gold set well formed and safe? (ADR-0010)
//!
//! A gold set is a folder of case folders. Each holds exactly two files: the
//! anonymised document and its `label.json`. The checker looks at every case
//! and at the set as a whole, and reports every problem it finds, not just the
//! first.
//!
//! **A report never contains anything from a label or a document.** It names
//! cases by their opaque ids (`g-0001`), fields by their position
//! (`lines[2].unit_price`) and problems by a fixed sentence. Folder names that
//! are not case ids are counted, never printed, because a name like
//! `Acme invoices` would itself be personal information.

use crate::hash::sha256_hex_of_reader;
use crate::keys::{compare, finding_keys, Difference};
use crate::label::{
    is_case_id, is_finding_key, is_tag, Label, LabelProblem, ParseError, Split, Tier, TruthError,
};
use crate::personal::{personal_data_in, PersonalData};
use clearcut_core::extraction::validate;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Write as _};
use std::fs;
use std::io::{self, Read};
use std::path::Path;

/// Labels are small. Anything bigger is refused rather than read.
const MAX_LABEL_BYTES: u64 = 1024 * 1024;

/// Targets from ADR-0010, shown beside the counts.
pub const FIRST_TARGET_CASES: usize = 25;
pub const GOAL_CASES: usize = 200;
pub const TARGET_SUPPLIERS: usize = 60;
pub const TARGET_EXPECTED_FINDINGS: usize = 10;

/// The set itself could not be examined at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CheckError {
    #[error("the gold set folder cannot be read ({0})")]
    Unreadable(io::ErrorKind),
}

/// One thing wrong. No variant holds text from a label or a document.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Problem {
    // ---- about the set ----
    #[error("the folder holds no cases")]
    NoCases,
    #[error("{count} loose file(s) in the gold set folder; it may hold only case folders")]
    UnexpectedRootFiles { count: usize },
    #[error("folder number {index} (not named like g-0001) is not a case folder")]
    FolderNotACaseId { index: usize },
    // ---- about a case ----
    #[error("label.json is missing")]
    LabelMissing,
    #[error("label.json is larger than 1 MiB")]
    LabelTooLarge,
    #[error("label.json cannot be read ({0})")]
    LabelUnreadable(io::ErrorKind),
    #[error("{0}")]
    Parse(ParseError),
    #[error("{0}")]
    Label(LabelProblem),
    #[error("the case id in label.json is not the name of its folder")]
    CaseFolderMismatch,
    #[error("the document named in the label is missing")]
    DocumentMissing,
    #[error("the document is not a plain file in the case folder")]
    DocumentNotAFile,
    #[error("the document cannot be read ({0})")]
    DocumentUnreadable(io::ErrorKind),
    #[error("the document is empty")]
    DocumentEmpty,
    #[error("the document does not match the sha256 in the label")]
    HashMismatch,
    #[error("{count} unexpected file(s) or folder(s) in the case folder; it may hold only label.json and the document")]
    UnexpectedEntries { count: usize },
    #[error("{0}")]
    Truth(TruthError),
    #[error("{}", describe_difference(.0))]
    FindingsDiffer(Difference),
    #[error("{field} looks like it holds {kind}")]
    Personal { field: String, kind: PersonalData },
    #[error("the same document as {other}")]
    DuplicateDocument { other: String },
    #[error("the supplier also appears in {other}, which is in the other split")]
    SupplierInBothSplits { other: String },
}

fn describe_difference(difference: &Difference) -> String {
    let mut text = String::from("the validator's findings differ from `expect`");
    if !difference.missing.is_empty() {
        let _ = write!(
            text,
            "; expected but not reported: {}",
            difference.missing.join(", ")
        );
    }
    if !difference.unexpected.is_empty() {
        let _ = write!(
            text,
            "; reported but not expected: {}",
            difference.unexpected.join(", ")
        );
    }
    text
}

/// A problem and the case it belongs to (`None` for the set as a whole).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    pub case: Option<String>,
    pub problem: Problem,
}

impl fmt::Display for Located {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.case {
            Some(case) => write!(f, "{case}: {}", self.problem),
            None => write!(f, "(set): {}", self.problem),
        }
    }
}

/// What the set holds, to compare with the targets in ADR-0010.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Coverage {
    /// Cases whose label could be read.
    pub cases: usize,
    pub dev: usize,
    pub test: usize,
    pub own: usize,
    pub consented: usize,
    pub synthetic: usize,
    /// Distinct suppliers (by VAT number, else by name).
    pub suppliers: usize,
    /// Cases whose label expects at least one validation finding.
    pub with_expected_findings: usize,
    pub pages: usize,
    pub tags: BTreeMap<String, usize>,
}

/// The result of checking a gold set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckReport {
    /// Case folders examined.
    pub cases: usize,
    pub problems: Vec<Located>,
    pub coverage: Coverage,
}

impl CheckReport {
    pub fn is_ok(&self) -> bool {
        self.problems.is_empty()
    }

    /// The report as text for a person: coverage first, then every problem.
    pub fn render(&self) -> String {
        let c = &self.coverage;
        let mut out = String::new();
        let _ = writeln!(
            out,
            "Gold set: {} {} (dev {}, test {}); first target {FIRST_TARGET_CASES}, goal {GOAL_CASES}+",
            self.cases,
            if self.cases == 1 { "case" } else { "cases" },
            c.dev,
            c.test
        );
        let _ = writeln!(
            out,
            "Provenance: own {}, consented {}, synthetic {}",
            c.own, c.consented, c.synthetic
        );
        let _ = writeln!(
            out,
            "Distinct suppliers: {} (target {TARGET_SUPPLIERS})",
            c.suppliers
        );
        let _ = writeln!(
            out,
            "Documents with expected findings (such as wrong printed totals): {} (target {TARGET_EXPECTED_FINDINGS})",
            c.with_expected_findings
        );
        let _ = writeln!(out, "Pages: {}", c.pages);
        if c.tags.is_empty() {
            let _ = writeln!(out, "Tags: none");
        } else {
            let tags: Vec<String> = c
                .tags
                .iter()
                .map(|(tag, count)| format!("{tag} {count}"))
                .collect();
            let _ = writeln!(out, "Tags: {}", tags.join(", "));
        }
        if self.problems.is_empty() {
            let _ = writeln!(out, "\nChecked {} cases: no problems.", self.cases);
        } else {
            let _ = writeln!(out, "\nFound {} problem(s):", self.problems.len());
            for located in &self.problems {
                let _ = writeln!(out, "  {located}");
            }
        }
        out
    }
}

/// What the set-wide checks need to remember about a case that was read.
struct Case {
    id: String,
    label: Label,
    /// The document's real SHA-256, if it could be computed.
    sha256: Option<String>,
}

/// Checks the gold set in `root`.
pub fn check_set(root: &Path) -> Result<CheckReport, CheckError> {
    let unreadable = |error: io::Error| CheckError::Unreadable(error.kind());

    let mut case_ids = Vec::new();
    let mut other_folders = 0usize;
    let mut loose_files = 0usize;
    for entry in fs::read_dir(root).map_err(unreadable)? {
        let entry = entry.map_err(unreadable)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        // `file_type` does not follow links, so a link to a folder counts as a loose file.
        if entry.file_type().map_err(unreadable)?.is_dir() {
            if is_case_id(&name) {
                case_ids.push(name);
            } else {
                other_folders += 1;
            }
        } else {
            loose_files += 1;
        }
    }
    case_ids.sort();

    let mut problems = Vec::new();
    let mut set_level = |problem| problems.push(Located { case: None, problem });
    if loose_files > 0 {
        set_level(Problem::UnexpectedRootFiles { count: loose_files });
    }
    for index in 1..=other_folders {
        set_level(Problem::FolderNotACaseId { index });
    }
    if case_ids.is_empty() {
        set_level(Problem::NoCases);
    }

    let mut cases = Vec::new();
    for id in &case_ids {
        let (found, read) = check_case(&root.join(id), id);
        problems.extend(found.into_iter().map(|problem| Located {
            case: Some(id.clone()),
            problem,
        }));
        cases.extend(read);
    }
    problems.extend(check_across_cases(&cases));

    // Set-level problems first, then each case in name order. The sort is
    // stable, so a case's own problems keep the order they were found in.
    problems.sort_by(|a, b| {
        a.case
            .is_some()
            .cmp(&b.case.is_some())
            .then_with(|| a.case.cmp(&b.case))
    });

    Ok(CheckReport {
        cases: case_ids.len(),
        problems,
        coverage: coverage_of(&cases),
    })
}

/// Checks one case folder. Returns the problems found and, if the label could
/// be read, what the set-wide checks need to know about it.
fn check_case(folder: &Path, id: &str) -> (Vec<Problem>, Option<Case>) {
    let mut problems = Vec::new();

    let bytes = match read_label(&folder.join("label.json")) {
        Ok(bytes) => bytes,
        Err(problem) => return (vec![problem], None),
    };
    let label = match Label::parse(&bytes) {
        Ok(label) => label,
        Err(error) => return (vec![Problem::Parse(error)], None),
    };

    let label_problems = label.problems();
    let name_is_usable = !label_problems.contains(&LabelProblem::BadFileName);
    problems.extend(label_problems.into_iter().map(Problem::Label));
    if label.case != id {
        problems.push(Problem::CaseFolderMismatch);
    }

    // The document is only opened when the label names it safely.
    let mut sha256 = None;
    if name_is_usable {
        let wanted = &label.document.file;
        match hash_document(&folder.join(wanted)) {
            Ok(Some((hash, length))) => {
                if length == 0 {
                    problems.push(Problem::DocumentEmpty);
                } else if hash != label.document.sha256 {
                    problems.push(Problem::HashMismatch);
                }
                sha256 = Some(hash);
            }
            Ok(None) => problems.push(Problem::DocumentNotAFile),
            Err(error) if error.kind() == io::ErrorKind::NotFound => problems.push(Problem::DocumentMissing),
            Err(error) => problems.push(Problem::DocumentUnreadable(error.kind())),
        }
        let extra = unexpected_entries(folder, wanted);
        if extra > 0 {
            problems.push(Problem::UnexpectedEntries { count: extra });
        }
    }

    match label
        .truth
        .to_extraction(&label.case, &label.provenance.labelled_by)
    {
        Err(error) => problems.push(Problem::Truth(error)),
        Ok(extraction) => {
            let actual = finding_keys(&validate(&extraction).findings);
            // Only well-formed keys are compared (and so ever printed); a malformed
            // one has already been reported by position as a label problem.
            let expected: Vec<String> = label
                .expect
                .findings
                .iter()
                .filter(|key| is_finding_key(key))
                .cloned()
                .collect();
            let difference = compare(&expected, &actual);
            if !difference.is_empty() {
                problems.push(Problem::FindingsDiffer(difference));
            }
        }
    }

    for (field, kind) in personal_data_in(&label.truth) {
        problems.push(Problem::Personal { field, kind });
    }

    (
        problems,
        Some(Case {
            id: id.to_string(),
            label,
            sha256,
        }),
    )
}

fn read_label(path: &Path) -> Result<Vec<u8>, Problem> {
    let file = fs::File::open(path).map_err(|error| match error.kind() {
        io::ErrorKind::NotFound => Problem::LabelMissing,
        kind => Problem::LabelUnreadable(kind),
    })?;
    // Read one byte more than allowed, to tell "exactly at the limit" from "over".
    let mut bytes = Vec::new();
    file.take(MAX_LABEL_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| Problem::LabelUnreadable(error.kind()))?;
    if bytes.len() as u64 > MAX_LABEL_BYTES {
        return Err(Problem::LabelTooLarge);
    }
    Ok(bytes)
}

/// The hash and length of the document, or `None` if the path is not a plain
/// file (a folder, or a link that could lead outside the case folder).
fn hash_document(path: &Path) -> io::Result<Option<(String, u64)>> {
    if !fs::symlink_metadata(path)?.is_file() {
        return Ok(None);
    }
    sha256_hex_of_reader(fs::File::open(path)?).map(Some)
}

/// Whether two file names are the same file. Windows names ignore case; on
/// other systems `LABEL.JSON` is a different (and unexpected) file.
fn same_file_name(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

/// How many entries other than `label.json` and the document the folder holds.
fn unexpected_entries(folder: &Path, document: &str) -> usize {
    let Ok(entries) = fs::read_dir(folder) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            !same_file_name(&name, "label.json") && !same_file_name(&name, document)
        })
        .count()
}

/// How a supplier is recognised across cases: by VAT number and by name, so a
/// supplier cannot hide from the split rule by changing one of them.
fn supplier_keys(label: &Label) -> Vec<String> {
    let Some(supplier) = &label.truth.supplier else {
        return Vec::new();
    };
    let mut keys = Vec::new();
    if let Some(vat) = supplier
        .value
        .vat_number
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        keys.push(format!("vat:{vat}"));
    }
    let name = supplier
        .value
        .name
        .split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ");
    if !name.is_empty() {
        keys.push(format!("name:{name}"));
    }
    keys
}

/// Problems that only show when cases are compared: one document filed twice,
/// and one supplier on both sides of the dev/test split.
fn check_across_cases(cases: &[Case]) -> Vec<Located> {
    let mut out = Vec::new();
    let mut by_hash: BTreeMap<&str, &str> = BTreeMap::new();
    let mut by_supplier: BTreeMap<String, Vec<(Split, &str)>> = BTreeMap::new();

    for case in cases {
        let mut report = |problem| {
            out.push(Located {
                case: Some(case.id.clone()),
                problem,
            })
        };

        if let Some(hash) = case.sha256.as_deref() {
            match by_hash.get(hash) {
                Some(first) => report(Problem::DuplicateDocument {
                    other: (*first).to_string(),
                }),
                None => {
                    by_hash.insert(hash, &case.id);
                }
            }
        }

        let split = case.label.split;
        let mut conflict = None;
        for key in supplier_keys(&case.label) {
            let seen = by_supplier.entry(key).or_default();
            if conflict.is_none() {
                conflict = seen
                    .iter()
                    .find(|(other, _)| *other != split)
                    .map(|(_, id)| (*id).to_string());
            }
            seen.push((split, &case.id));
        }
        if let Some(other) = conflict {
            report(Problem::SupplierInBothSplits { other });
        }
    }
    out
}

fn coverage_of(cases: &[Case]) -> Coverage {
    let mut coverage = Coverage {
        cases: cases.len(),
        ..Coverage::default()
    };
    let mut suppliers = BTreeSet::new();
    for case in cases {
        let label = &case.label;
        match label.split {
            Split::Dev => coverage.dev += 1,
            Split::Test => coverage.test += 1,
        }
        match label.provenance.tier {
            Tier::Own => coverage.own += 1,
            Tier::Consented => coverage.consented += 1,
            Tier::Synthetic => coverage.synthetic += 1,
        }
        if !label.expect.findings.is_empty() {
            coverage.with_expected_findings += 1;
        }
        coverage.pages += label.document.pages as usize;
        // Only well-formed tags are counted (and so printed): a malformed one is
        // reported as a problem, but its text could be anything.
        for tag in label.tags.iter().filter(|tag| is_tag(tag)) {
            *coverage.tags.entry(tag.clone()).or_default() += 1;
        }
        // The first key is the VAT number when there is one, else the name.
        if let Some(key) = supplier_keys(label).into_iter().next() {
            suppliers.insert(key);
        }
    }
    coverage.suppliers = suppliers.len();
    coverage
}

#[cfg(test)]
mod tests;
