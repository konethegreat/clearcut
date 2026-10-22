// Written by Kone Tshivhinda & Claude
//! The `clearcut-gold` command line. It lives in the library, taking the
//! output streams as arguments, so tests can run it without starting a
//! program.

use crate::check::check_set;
use crate::sample::results::write_sample_results;
use crate::sample::write_sample;
use crate::score::{score_folders, Gate, ScoreError, SplitChoice};
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

/// Everything is fine.
pub const EXIT_OK: u8 = 0;
/// The check or score ran and found problems (a gold set with problems, or a
/// run that fails the gate).
pub const EXIT_PROBLEMS: u8 = 1;
/// The command was used wrongly.
pub const EXIT_USAGE: u8 = 2;
/// The command was right but could not do its job (a folder it cannot read).
pub const EXIT_UNUSABLE: u8 = 3;

const USAGE: &str = "\
Usage:
  clearcut-gold check <gold-set-folder>
      check a gold set and show what it covers
  clearcut-gold score <gold-set-folder> <results-folder> [--split dev|test|all]
                      [--gate <gate.json>] [--json <report.json>]
      score a run against the gold set and apply the release gate
      (default: the test split and the research-05 targets)
  clearcut-gold sample <new-folder>
      write the synthetic sample gold set
  clearcut-gold sample-results <new-folder>
      write sample results for it, with deliberate flaws
  clearcut-gold --help

Exit codes: 0 fine, 1 problems found (a gold set with problems, or a run that
fails the gate), 2 wrong usage, 3 a folder or file could not be read or written.

Reports name cases by id and fields by position. They never print what a
label, a document or a result says.
";

/// A gate file is a handful of numbers; anything bigger is not one.
const MAX_GATE_BYTES: u64 = 64 * 1024;

/// Runs one command. `args` excludes the program name. Returns the exit code.
pub fn run(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["--help" | "-h" | "help"] => {
            let _ = write!(out, "{USAGE}");
            EXIT_OK
        }
        ["check", folder] => check(Path::new(folder), out, err),
        ["sample", folder] => sample(Path::new(folder), out, err),
        ["sample-results", folder] => sample_results(Path::new(folder), out, err),
        ["score", rest @ ..] => match ScoreArgs::parse(rest) {
            Some(parsed) => score(&parsed, out, err),
            None => usage(err),
        },
        _ => usage(err),
    }
}

fn usage(err: &mut dyn Write) -> u8 {
    let _ = write!(err, "{USAGE}");
    EXIT_USAGE
}

fn check(folder: &Path, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    match check_set(folder) {
        Ok(report) => {
            let _ = write!(out, "{}", report.render());
            if report.is_ok() {
                EXIT_OK
            } else {
                EXIT_PROBLEMS
            }
        }
        Err(error) => {
            // The error text carries only the kind of failure, not the path.
            let _ = writeln!(err, "{error}");
            EXIT_UNUSABLE
        }
    }
}

fn sample(folder: &Path, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    match write_sample(folder) {
        Ok(()) => {
            let _ = writeln!(
                out,
                "Wrote the synthetic sample set (15 cases). Check it with: clearcut-gold check <folder>"
            );
            EXIT_OK
        }
        Err(error) => {
            let _ = writeln!(err, "{error}");
            EXIT_UNUSABLE
        }
    }
}

fn sample_results(folder: &Path, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    match write_sample_results(folder) {
        Ok(()) => {
            let _ = writeln!(
                out,
                "Wrote sample results for 15 cases. Score them with: clearcut-gold score <gold folder> <folder>"
            );
            EXIT_OK
        }
        Err(error) => {
            let _ = writeln!(err, "{error}");
            EXIT_UNUSABLE
        }
    }
}

/// The arguments of `score`.
struct ScoreArgs<'a> {
    gold: &'a Path,
    results: &'a Path,
    split: Option<SplitChoice>,
    gate: Option<&'a Path>,
    json: Option<&'a Path>,
}

impl<'a> ScoreArgs<'a> {
    /// `None` if the arguments are not two folders and each flag at most once
    /// with a value.
    fn parse(args: &[&'a str]) -> Option<ScoreArgs<'a>> {
        let mut folders = Vec::new();
        let (mut split, mut gate, mut json) = (None, None, None);
        let mut rest = args.iter();
        while let Some(&arg) = rest.next() {
            match arg {
                "--split" => {
                    let value = match *rest.next()? {
                        "dev" => SplitChoice::Dev,
                        "test" => SplitChoice::Test,
                        "all" => SplitChoice::All,
                        _ => return None,
                    };
                    if split.replace(value).is_some() {
                        return None;
                    }
                }
                "--gate" => {
                    if gate.replace(Path::new(*rest.next()?)).is_some() {
                        return None;
                    }
                }
                "--json" => {
                    if json.replace(Path::new(*rest.next()?)).is_some() {
                        return None;
                    }
                }
                flag if flag.starts_with("--") => return None,
                folder => folders.push(Path::new(folder)),
            }
        }
        let [gold, results] = folders.as_slice() else {
            return None;
        };
        Some(ScoreArgs {
            gold,
            results,
            split,
            gate,
            json,
        })
    }
}

fn score(args: &ScoreArgs<'_>, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let mut gate = match args.gate {
        Some(path) => match read_gate(path) {
            Ok(gate) => gate,
            Err(message) => {
                let _ = writeln!(err, "{message}");
                return EXIT_UNUSABLE;
            }
        },
        None => Gate::default(),
    };
    if let Some(split) = args.split {
        gate.split = split;
    }

    // Open the report file first: if it cannot be created, fail before the work.
    let mut json_file = match args
        .json
        .map(|path| fs::OpenOptions::new().write(true).create_new(true).open(path))
    {
        None => None,
        Some(Ok(file)) => Some(file),
        Some(Err(error)) => {
            let message = if error.kind() == std::io::ErrorKind::AlreadyExists {
                "the report file already exists; it is never overwritten".to_string()
            } else {
                format!("the report file cannot be created ({})", error.kind())
            };
            let _ = writeln!(err, "{message}");
            return EXIT_UNUSABLE;
        }
    };

    let report = match score_folders(args.gold, args.results, &gate) {
        Ok(report) => report,
        Err(error) => {
            let _ = writeln!(err, "{error}");
            return match error {
                ScoreError::GoldSetHasProblems { .. } => EXIT_PROBLEMS,
                ScoreError::Unreadable(_) => EXIT_UNUSABLE,
            };
        }
    };

    let _ = write!(out, "{}", report.render_markdown());
    if let Some(file) = json_file.as_mut() {
        let written = serde_json::to_writer_pretty(&mut *file, &report)
            .map_err(std::io::Error::from)
            .and_then(|()| writeln!(file));
        if let Err(error) = written {
            let _ = writeln!(err, "the report file cannot be written ({})", error.kind());
            return EXIT_UNUSABLE;
        }
    }
    if report.passed() {
        EXIT_OK
    } else {
        EXIT_PROBLEMS
    }
}

/// Reads a gate file; the error is a sentence with no content from the file.
fn read_gate(path: &Path) -> Result<Gate, String> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .and_then(|file| file.take(MAX_GATE_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|error| format!("the gate file cannot be read ({})", error.kind()))?;
    if bytes.len() as u64 > MAX_GATE_BYTES {
        return Err("the gate file is too large".to_string());
    }
    Gate::parse(&bytes).map_err(|error| format!("the gate file is not valid: {error}"))
}

#[cfg(test)]
mod tests;
