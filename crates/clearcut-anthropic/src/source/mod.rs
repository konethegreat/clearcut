// Written by Kone Tshivhinda & Claude
//! Where a key comes from (ADR-0011, mode A: one owner running ClearCut).
//!
//! - `ANTHROPIC_API_KEY`: the key itself, in the environment; or
//! - `ANTHROPIC_API_KEY_FILE`: the path of a file holding it (for container and
//!   Kubernetes secrets, which are mounted as files, often through a link).
//!
//! Both set is an error rather than a guess. An **empty** variable counts as not
//! set (containers often define empty ones). A key given on the **command line**
//! is refused outright: other users of the computer can read a process's
//! arguments.
//!
//! Errors never contain the path of the file or anything it held.

use crate::key::{ApiKey, KeyError};
use std::ffi::OsString;
use std::fs;
use std::io::{self, Read};
use std::path::Path;
use zeroize::Zeroizing;

pub const KEY_ENV: &str = "ANTHROPIC_API_KEY";
pub const KEY_FILE_ENV: &str = "ANTHROPIC_API_KEY_FILE";

/// A key file bigger than this is not a key file.
pub const MAX_KEY_FILE_BYTES: usize = 4096;

/// Why no key could be loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LoadError {
    #[error("no API key is configured; set ANTHROPIC_API_KEY or ANTHROPIC_API_KEY_FILE")]
    NotConfigured,
    #[error("both ANTHROPIC_API_KEY and ANTHROPIC_API_KEY_FILE are set; set only one")]
    Ambiguous,
    #[error("the API key file cannot be read ({0})")]
    FileUnreadable(io::ErrorKind),
    #[error("the API key file is not a plain file")]
    FileNotAPlainFile,
    #[error("the API key file is larger than {MAX_KEY_FILE_BYTES} bytes")]
    FileTooLarge,
    #[error("{0}")]
    Key(KeyError),
}

/// Loads the key from the process environment.
pub fn load() -> Result<ApiKey, LoadError> {
    load_from(|name| std::env::var_os(name))
}

/// Loads the key using `env` to look up variables (so tests need not touch the
/// real environment, which the whole process shares).
pub fn load_from(env: impl Fn(&str) -> Option<OsString>) -> Result<ApiKey, LoadError> {
    let set = |name: &str| env(name).filter(|value| !value.is_empty());
    match (set(KEY_ENV), set(KEY_FILE_ENV)) {
        (None, None) => Err(LoadError::NotConfigured),
        (Some(_), Some(_)) => Err(LoadError::Ambiguous),
        (Some(value), None) => {
            // `into_string` hands back the same allocation, so the copy we hold is the one we wipe.
            let text = Zeroizing::new(
                value
                    .into_string()
                    .map_err(|_| LoadError::Key(KeyError::BadCharacters))?,
            );
            ApiKey::parse(&text).map_err(LoadError::Key)
        }
        (None, Some(path)) => load_file(Path::new(&path)),
    }
}

fn load_file(path: &Path) -> Result<ApiKey, LoadError> {
    let unreadable = |error: io::Error| LoadError::FileUnreadable(error.kind());
    // `metadata` follows links: a mounted secret is usually one.
    if !fs::metadata(path).map_err(unreadable)?.is_file() {
        return Err(LoadError::FileNotAPlainFile);
    }
    // One byte more than allowed, to tell "exactly at the limit" from "over".
    let mut bytes = Zeroizing::new(Vec::with_capacity(MAX_KEY_FILE_BYTES + 1));
    fs::File::open(path)
        .map_err(unreadable)?
        .take(MAX_KEY_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(unreadable)?;
    if bytes.len() > MAX_KEY_FILE_BYTES {
        return Err(LoadError::FileTooLarge);
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| LoadError::Key(KeyError::BadCharacters))?;
    ApiKey::parse(text).map_err(LoadError::Key)
}

/// A key was found where it must not be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ArgsError {
    #[error(
        "an API key was given on the command line, where other users of this computer can read it; \
         set ANTHROPIC_API_KEY or ANTHROPIC_API_KEY_FILE instead"
    )]
    KeyOnCommandLine,
}

/// Fails if any argument is, or contains, an API key, or is a flag that exists
/// to carry one. Call it with the program's arguments (without the program
/// name) before doing anything else.
pub fn reject_key_in_arguments<I>(args: I) -> Result<(), ArgsError>
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    for arg in args {
        let arg = arg.as_ref();
        if arg.contains("sk-ant-") || is_key_flag(arg) {
            return Err(ArgsError::KeyOnCommandLine);
        }
    }
    Ok(())
}

/// `--api-key`, `--anthropic-api-key=...`, `--ANTHROPIC_KEY` and the like.
fn is_key_flag(arg: &str) -> bool {
    let Some(flag) = arg.strip_prefix('-') else {
        return false;
    };
    let name = flag.trim_start_matches('-').split('=').next().unwrap_or("");
    let normalised: String = name
        .chars()
        .filter(|c| *c != '-' && *c != '_')
        .flat_map(char::to_lowercase)
        .collect();
    matches!(normalised.as_str(), "apikey" | "anthropicapikey" | "anthropickey")
}

#[cfg(test)]
mod tests;
