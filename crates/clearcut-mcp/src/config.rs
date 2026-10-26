// Written by Kone Tshivhinda & Claude
//! What the MCP server reads from its environment.
//!
//! - `CLEARCUT_URL`: where the ClearCut server is (default `http://127.0.0.1:8080`).
//! - `CLEARCUT_API_KEY` or `CLEARCUT_API_KEY_FILE`: **a ClearCut API key** (never an Anthropic
//!   key). Without one the document tools say so and the calculation tools still work.
//! - `CLEARCUT_MCP_UPLOAD_DIR`: the **one folder** uploads may be read from. Without it
//!   `upload_document` can only take file content handed to it, never a path: a document an agent is
//!   reading could try to make it send some other file on this computer, so a path outside this folder
//!   is refused.
//! - `CLEARCUT_MCP_OUTPUT_DIR`: where exported PDF, Excel and Word files are saved.
//!
//! The key is held in a type that cannot be printed and goes only to `CLEARCUT_URL`.

use std::fmt;
use std::path::PathBuf;

/// A ClearCut API key. `Debug` shows `[redacted]`.
#[derive(Clone)]
pub struct SecretKey(String);

impl SecretKey {
    pub fn new(text: &str) -> Option<SecretKey> {
        let text = text.trim();
        (!text.is_empty() && !text.contains(char::is_whitespace)).then(|| SecretKey(text.to_string()))
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretKey([redacted])")
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub base_url: String,
    pub api_key: Option<SecretKey>,
    pub upload_dir: Option<PathBuf>,
    pub output_dir: Option<PathBuf>,
}

impl Config {
    /// Reads the settings. `Err` is a sentence for the person starting the server (never a secret).
    pub fn from_env(env: impl Fn(&str) -> Option<String>) -> Result<Config, String> {
        let set = |name: &str| env(name).filter(|value| !value.trim().is_empty());
        let base_url = set("CLEARCUT_URL").unwrap_or_else(|| "http://127.0.0.1:8080".to_string());
        let base_url = base_url.trim().trim_end_matches('/').to_string();
        if !(base_url.starts_with("http://") || base_url.starts_with("https://")) || base_url.contains('@') {
            return Err("CLEARCUT_URL must be an http:// or https:// address with no user name in it".into());
        }
        let api_key = match (set("CLEARCUT_API_KEY"), set("CLEARCUT_API_KEY_FILE")) {
            (Some(_), Some(_)) => {
                return Err("set only one of CLEARCUT_API_KEY and CLEARCUT_API_KEY_FILE".into())
            }
            (Some(text), None) => Some(SecretKey::new(&text).ok_or("CLEARCUT_API_KEY is not a usable key")?),
            (None, Some(path)) => {
                let text =
                    std::fs::read_to_string(&path).map_err(|_| "CLEARCUT_API_KEY_FILE cannot be read")?;
                Some(SecretKey::new(&text).ok_or("CLEARCUT_API_KEY_FILE does not hold a usable key")?)
            }
            (None, None) => None,
        };
        let dir = |name: &str| set(name).map(PathBuf::from);
        Ok(Config {
            base_url,
            api_key,
            upload_dir: dir("CLEARCUT_MCP_UPLOAD_DIR"),
            output_dir: dir("CLEARCUT_MCP_OUTPUT_DIR"),
        })
    }
}
