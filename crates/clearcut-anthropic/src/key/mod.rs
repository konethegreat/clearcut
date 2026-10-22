// Written by Kone Tshivhinda & Claude
//! The user's own Anthropic API key, held as a secret (ADR-0011).
//!
//! ClearCut has no API key of its own: whoever runs it supplies theirs, and
//! every call is billed to them. So a key is treated like a password:
//!
//! - it can be **printed nowhere**: `ApiKey` has no `Display`, no `Serialize`,
//!   and its `Debug` shows `[redacted]`;
//! - it is **wiped from memory** when the last copy is dropped;
//! - errors about a key say what is wrong with it (too short, has a space,
//!   wrong kind) and **never repeat any of its text**;
//! - only a regular API key (`sk-ant-api...`) is accepted. An **admin key**
//!   can manage a whole organisation, and a subscription **login token** is not
//!   meant for third-party programs: both are refused with a clear message.

use std::fmt;
use std::sync::Arc;
use zeroize::Zeroizing;

/// The shortest and longest key accepted. A real key is about 100 characters;
/// the bounds only catch obvious mistakes.
const MIN_LENGTH: usize = 30;
const MAX_LENGTH: usize = 300;

/// Why a piece of text is not usable as an API key. Fixed sentences only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    #[error("the API key is empty")]
    Empty,
    #[error("the API key has spaces or line breaks inside it")]
    Whitespace,
    #[error("the API key has characters that a key never has (it should be letters, digits, dashes and underscores)")]
    BadCharacters,
    #[error("this is not an Anthropic API key (it should start with sk-ant-api)")]
    NotAnApiKey,
    #[error("this looks like an Anthropic admin key; ClearCut needs a regular API key, which cannot manage your organisation")]
    AdminKey,
    #[error("this looks like a subscription login token, not an API key; create an API key in the Claude Console instead")]
    LoginToken,
    #[error("the API key has an implausible length")]
    BadLength,
}

/// An Anthropic API key. Cloning shares the one protected copy.
#[derive(Clone)]
pub struct ApiKey {
    secret: Arc<Zeroizing<String>>,
}

impl ApiKey {
    /// Checks the shape of `text` (surrounding whitespace is ignored) and keeps
    /// a protected copy. The caller's own copy of the text is the caller's to wipe.
    pub fn parse(text: &str) -> Result<ApiKey, KeyError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(KeyError::Empty);
        }
        if text.chars().any(char::is_whitespace) {
            return Err(KeyError::Whitespace);
        }
        if !text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(KeyError::BadCharacters);
        }
        if text.starts_with("sk-ant-admin") {
            return Err(KeyError::AdminKey);
        }
        if text.starts_with("sk-ant-oat") {
            return Err(KeyError::LoginToken);
        }
        if !text.starts_with("sk-ant-api") {
            return Err(KeyError::NotAnApiKey);
        }
        if !(MIN_LENGTH..=MAX_LENGTH).contains(&text.len()) {
            return Err(KeyError::BadLength);
        }
        Ok(ApiKey {
            secret: Arc::new(Zeroizing::new(text.to_string())),
        })
    }

    /// The last four characters, for telling two keys apart on a screen.
    pub fn fingerprint(&self) -> String {
        let text = self.expose();
        format!("…{}", &text[text.len() - 4..])
    }

    /// The secret itself. Crate-private: only code that builds a request header
    /// ever needs it.
    pub(crate) fn expose(&self) -> &str {
        self.secret.as_str()
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey([redacted])")
    }
}

/// Compares in time that does not depend on where two keys first differ.
impl PartialEq for ApiKey {
    fn eq(&self, other: &Self) -> bool {
        let (a, b) = (self.expose().as_bytes(), other.expose().as_bytes());
        a.len() == b.len()
            && a.iter()
                .zip(b)
                .fold(0u8, |difference, (x, y)| difference | (x ^ y))
                == 0
    }
}

impl Eq for ApiKey {}

#[cfg(test)]
mod tests;
