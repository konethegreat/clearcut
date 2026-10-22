// Written by Kone Tshivhinda & Claude
//! The `clearcut-gold` command line. It lives in the library, taking the
//! output streams as arguments, so tests can run it without starting a
//! program.

use crate::check::check_set;
use crate::sample::write_sample;
use std::io::Write;
use std::path::Path;

/// Everything is fine.
pub const EXIT_OK: u8 = 0;
/// The check ran and found problems in the gold set.
pub const EXIT_PROBLEMS: u8 = 1;
/// The command was used wrongly.
pub const EXIT_USAGE: u8 = 2;
/// The command was right but could not do its job (a folder it cannot read).
pub const EXIT_UNUSABLE: u8 = 3;

const USAGE: &str = "\
Usage:
  clearcut-gold check <gold-set-folder>   check a gold set and show what it covers
  clearcut-gold sample <new-folder>       write the synthetic sample gold set
  clearcut-gold --help

Exit codes: 0 fine, 1 problems found in the gold set, 2 wrong usage,
3 the folder could not be read or written.

Reports name cases by id and fields by position. They never print what a
label or a document says.
";

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
        _ => {
            let _ = write!(err, "{USAGE}");
            EXIT_USAGE
        }
    }
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

#[cfg(test)]
mod tests;
