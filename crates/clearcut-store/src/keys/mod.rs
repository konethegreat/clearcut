// Written by Kone Tshivhinda & Claude
//! API key format (ADR-0007): `ccut_<32 hex key id>_<64 hex secret>`.
//!
//! - The **key id** is a random UUID; it finds the row. It is not secret.
//! - The **secret** is 256 bits from the operating system's random source, so a
//!   plain SHA-256 is the right way to store it: slow password hashes protect
//!   low-entropy human passwords and add nothing for a random 256-bit value.
//!   Only the hash is ever stored.
//! - The `ccut_` prefix lets secret scanners recognise a leaked key.
//!
//! This module is pure apart from asking the OS for random bytes. A key's text
//! form appears exactly once, when it is created ([`ApiKeyToken`]); its
//! `Debug` output never contains the secret, so it cannot end up in a log by
//! accident.

use sha2::{Digest, Sha256};
use std::fmt;
use uuid::Uuid;

/// Every key starts with this.
pub const KEY_PREFIX: &str = "ccut_";

const ID_HEX_LEN: usize = 32;
const SECRET_HEX_LEN: usize = 64;

/// The full text of a key. Shown to its owner once; never stored.
pub struct ApiKeyToken {
    id: Uuid,
    text: String,
}

impl ApiKeyToken {
    /// The key as the owner must present it (`Authorization: Bearer ...`).
    pub fn expose(&self) -> &str {
        &self.text
    }
}

impl fmt::Debug for ApiKeyToken {
    /// Shows the key id (useful in logs) and hides the secret.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ApiKeyToken({KEY_PREFIX}{}_***)", self.id.simple())
    }
}

/// A freshly generated key and what the database needs to store for it.
#[derive(Debug)]
pub struct GeneratedKey {
    pub id: Uuid,
    pub token: ApiKeyToken,
    /// SHA-256 of the secret: the only form that is stored.
    pub secret_hash: [u8; 32],
}

/// What can be read out of a presented key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedKey {
    pub id: Uuid,
    pub secret_hash: [u8; 32],
}

/// SHA-256 of the 32 secret bytes.
pub fn hash_secret(secret: &[u8; 32]) -> [u8; 32] {
    Sha256::digest(secret).into()
}

/// Generates a new random key.
pub fn generate() -> Result<GeneratedKey, getrandom::Error> {
    let mut id_bytes = [0u8; 16];
    let mut secret = [0u8; 32];
    getrandom::fill(&mut id_bytes)?;
    getrandom::fill(&mut secret)?;
    // Sets the UUID version and variant bits; the rest stays random.
    let id = uuid::Builder::from_random_bytes(id_bytes).into_uuid();
    let text = format!("{KEY_PREFIX}{}_{}", id.simple(), to_hex(&secret));
    Ok(GeneratedKey {
        id,
        token: ApiKeyToken { id, text },
        secret_hash: hash_secret(&secret),
    })
}

/// Reads a presented key. Anything that is not exactly the documented shape
/// (right prefix, lowercase hex of exactly the right lengths, nothing extra)
/// is `None`: no trimming, no case folding, no `Bearer ` prefix.
pub fn parse(text: &str) -> Option<ParsedKey> {
    let rest = text.strip_prefix(KEY_PREFIX)?;
    if rest.len() != ID_HEX_LEN + 1 + SECRET_HEX_LEN || rest.as_bytes()[ID_HEX_LEN] != b'_' {
        return None;
    }
    let id: [u8; 16] = from_hex(&rest[..ID_HEX_LEN])?.try_into().ok()?;
    let secret: [u8; 32] = from_hex(&rest[ID_HEX_LEN + 1..])?.try_into().ok()?;
    Some(ParsedKey {
        id: Uuid::from_bytes(id),
        secret_hash: hash_secret(&secret),
    })
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Lowercase hex only; anything else is `None`.
fn from_hex(text: &str) -> Option<Vec<u8>> {
    if text.len() & 1 == 1 || !text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return None;
    }
    text.as_bytes()
        .chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests;
