// Written by Kone Tshivhinda & Claude
//! Reading a gold set and a folder of results from disk, and scoring them.
//!
//! The gold set is checked first ([`check_set`]) and **refused if it has any
//! problem**: a score against labels that are malformed, tampered with, or not
//! matching their documents would mean nothing. Results are read from
//! `<results>/<case>.json`. A missing file is not a note, because "no result"
//! is exactly what the completion figure measures; a file that cannot be read
//! or understood is noted (by case id and position, never content) and counts
//! as no result.

use super::report::ResultProblem;
use super::result::RunResult;
use super::{score, CaseInput, Gate, Report};
use crate::check::check_set;
use crate::label::{is_case_id, Label};
use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Read};
use std::path::Path;

/// A result file larger than this is refused unread.
pub const MAX_RESULT_BYTES: u64 = 8 * 1024 * 1024;

/// Why scoring could not run at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ScoreError {
    #[error("the gold set has {count} problem(s); run clearcut-gold check on it and fix them first")]
    GoldSetHasProblems { count: usize },
    #[error("a folder cannot be read ({0})")]
    Unreadable(io::ErrorKind),
}

impl From<io::Error> for ScoreError {
    fn from(error: io::Error) -> Self {
        ScoreError::Unreadable(error.kind())
    }
}

/// Scores the results in `results` against the gold set in `gold`, over the
/// split the gate selects.
pub fn score_folders(gold: &Path, results: &Path, gate: &Gate) -> Result<Report, ScoreError> {
    let report =
        check_set(gold).map_err(|crate::check::CheckError::Unreadable(kind)| ScoreError::Unreadable(kind))?;
    if !report.is_ok() {
        return Err(ScoreError::GoldSetHasProblems {
            count: report.problems.len(),
        });
    }

    let mut ids = Vec::new();
    for entry in fs::read_dir(gold)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type()?.is_dir() && is_case_id(&name) {
            ids.push(name);
        }
    }
    ids.sort();
    // Fails early, and with the right error, if the results folder is missing.
    let present = fs::read_dir(results)?.count();

    let mut cases = Vec::with_capacity(ids.len());
    let mut problems = Vec::new();
    for id in &ids {
        let label = Label::parse(&fs::read(gold.join(id).join("label.json"))?)
            .expect("check_set already accepted this label");
        let truth = label
            .truth
            .to_extraction(&label.case, &label.provenance.labelled_by)
            .expect("check_set already accepted this label");
        let result = if gate.split.includes(label.split) {
            read_result(&results.join(format!("{id}.json")), id, &mut problems)
        } else {
            None
        };
        cases.push(CaseInput {
            id: id.clone(),
            split: label.split,
            tags: label.tags,
            expected_findings: label.expect.findings,
            truth,
            result,
        });
    }

    // Anything in the results folder that is not `<a gold case>.json`.
    let known: BTreeSet<String> = ids.iter().map(|id| format!("{id}.json")).collect();
    let mut orphans = 0;
    if present > 0 {
        for entry in fs::read_dir(results)? {
            if !known.contains(&entry?.file_name().to_string_lossy().into_owned()) {
                orphans += 1;
            }
        }
    }
    if orphans > 0 {
        problems.push(ResultProblem::OrphanResults { count: orphans });
    }

    Ok(score(&cases, gate, problems))
}

/// Reads `<results>/<id>.json`. `None` if there is no usable result; a note is
/// added to `problems` unless the file simply does not exist.
fn read_result(path: &Path, id: &str, problems: &mut Vec<ResultProblem>) -> Option<RunResult> {
    let bytes = match read_limited(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return None,
        Err(_) => {
            problems.push(ResultProblem::Unreadable { case: id.to_string() });
            return None;
        }
    };
    match RunResult::parse(&bytes) {
        Ok(result) if result.case == id => Some(result),
        Ok(_) => {
            problems.push(ResultProblem::CaseMismatch { case: id.to_string() });
            None
        }
        Err(error) => {
            problems.push(ResultProblem::Parse {
                case: id.to_string(),
                error,
            });
            None
        }
    }
}

/// Reads a plain file of at most [`MAX_RESULT_BYTES`]. A link, a folder or an
/// oversized file is an error, not something to follow or buffer.
fn read_limited(path: &Path) -> io::Result<Vec<u8>> {
    if !fs::symlink_metadata(path)?.is_file() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a plain file"));
    }
    let mut bytes = Vec::new();
    // One byte more than allowed, to tell "exactly at the limit" from "over".
    fs::File::open(path)?
        .take(MAX_RESULT_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_RESULT_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "too large"));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests;
