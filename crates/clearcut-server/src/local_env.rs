// Written by Kone Tshivhinda & Claude
//! Settings from the environment, with a `.env` file as a fallback for running on one's own
//! computer.
//!
//! A real environment variable always wins over the file. The file is `KEY=VALUE` lines
//! (blank lines and `#` comments ignored, an optional `export ` and optional quotes around
//! the value allowed). It is looked for at the path in `CLEARCUT_ENV_FILE`, or `.env` in the
//! directory the server starts in, and it is git-ignored. **Values are never printed or logged
//! here**; the file may hold the Anthropic key.
//!
//! (The values are looked up, not copied into the process environment: setting environment
//! variables from code is `unsafe` in newer Rust and this workspace forbids unsafe code.)

use std::collections::HashMap;
use std::ffi::OsString;

pub struct LocalEnv {
    file: HashMap<String, String>,
}

/// Reads `KEY=VALUE` lines. Lines that are not of that shape are skipped.
pub fn parse(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for line in text.lines() {
        let line = line.trim().trim_start_matches('\u{feff}');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            continue;
        }
        let value = value.trim();
        let value = match (value.chars().next(), value.chars().last()) {
            (Some(open @ ('"' | '\'')), Some(close)) if open == close && value.len() >= 2 => {
                &value[1..value.len() - 1]
            }
            _ => value,
        };
        out.insert(key.to_string(), value.to_string());
    }
    out
}

impl LocalEnv {
    /// Loads the file named by `CLEARCUT_ENV_FILE`, or `.env` if there is one.
    pub fn load() -> LocalEnv {
        let path = std::env::var_os("CLEARCUT_ENV_FILE")
            .filter(|path| !path.is_empty())
            .unwrap_or_else(|| ".env".into());
        let file = std::fs::read_to_string(path)
            .map(|text| parse(&text))
            .unwrap_or_default();
        LocalEnv { file }
    }

    /// A setting: the environment variable if it is set and not empty, else the file's.
    pub fn get(&self, name: &str) -> Option<String> {
        std::env::var(name)
            .ok()
            .filter(|value| !value.is_empty())
            .or_else(|| self.file.get(name).filter(|value| !value.is_empty()).cloned())
    }

    /// The same, as the lookup the key loader takes.
    pub fn os(&self, name: &str) -> Option<OsString> {
        self.get(name).map(OsString::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_value_lines_are_read_and_the_rest_is_skipped() {
        let text = "\u{feff}# a comment\n\nA=1\nexport B = two \nC=\"quoted value\"\nD='single'\nnot a setting\n=novalue\nE F=bad key\nG=a=b\n";
        let map = parse(text);
        assert_eq!(map["A"], "1");
        assert_eq!(map["B"], "two");
        assert_eq!(map["C"], "quoted value");
        assert_eq!(map["D"], "single");
        assert_eq!(map["G"], "a=b");
        assert_eq!(map.len(), 5, "{map:?}");
    }

    #[test]
    fn windows_line_endings_do_not_end_up_in_the_value() {
        assert_eq!(parse("KEY=value\r\nOTHER=x\r\n")["KEY"], "value");
    }

    #[test]
    fn a_real_variable_wins_over_the_file_and_an_empty_one_does_not() {
        let env = LocalEnv {
            file: parse("PATH=from-the-file\nCLEARCUT_TEST_ONLY_IN_FILE=yes\nCLEARCUT_TEST_EMPTY=fallback\n"),
        };
        assert_ne!(env.get("PATH").as_deref(), Some("from-the-file"));
        assert_eq!(env.get("CLEARCUT_TEST_ONLY_IN_FILE").as_deref(), Some("yes"));
        assert_eq!(env.get("CLEARCUT_TEST_NOT_ANYWHERE"), None);
    }
}
