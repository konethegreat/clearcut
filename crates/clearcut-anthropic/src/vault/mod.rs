// Written by Kone Tshivhinda & Claude
//! A tenant's own Anthropic key, sealed for storage (ADR-0019, ADR-0011 mode B).
//!
//! The service encrypts a key **before** it reaches the database, so the database only ever holds
//! ciphertext. AES-256-GCM, a fresh random nonce every time, and the **tenant id bound in as
//! associated data**: a ciphertext copied to another tenant's row does not decrypt.
//!
//! The master key comes from `CLEARCUT_MASTER_KEY` (32 random bytes, base64) or
//! `CLEARCUT_MASTER_KEY_FILE`, never from the database. It is held in a type that cannot be
//! printed and is wiped when dropped. Errors say what is wrong and never repeat a key, a master
//! key, a path or any plaintext.

use crate::key::{ApiKey, KeyError};
use aes_gcm::aead::{Aead, Generate, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::Path;
use uuid::Uuid;
use zeroize::Zeroizing;

pub const MASTER_KEY_ENV: &str = "CLEARCUT_MASTER_KEY";
pub const MASTER_KEY_FILE_ENV: &str = "CLEARCUT_MASTER_KEY_FILE";
pub const MASTER_KEY_ID_ENV: &str = "CLEARCUT_MASTER_KEY_ID";
/// The name given to the master key when none is set. It is stored with each sealed key so a later
/// master key can be told apart.
pub const DEFAULT_MASTER_KEY_ID: &str = "k1";

const NONCE_LEN: usize = 12;
const MASTER_KEY_LEN: usize = 32;
/// What is bound in with the tenant id, so a ciphertext means this and nothing else.
const LABEL: &[u8] = b"clearcut/tenant-anthropic-key/v1";

/// Why a master key could not be loaded, or a key could not be sealed or opened. Fixed sentences.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum VaultError {
    #[error("no master key is configured; set CLEARCUT_MASTER_KEY or CLEARCUT_MASTER_KEY_FILE")]
    NotConfigured,
    #[error("both CLEARCUT_MASTER_KEY and CLEARCUT_MASTER_KEY_FILE are set; set only one")]
    Ambiguous,
    #[error("the master key file cannot be read ({0})")]
    FileUnreadable(io::ErrorKind),
    #[error("the master key is not 32 bytes of base64")]
    BadMasterKey,
    #[error("the master key id is empty, too long or has characters other than letters, digits, dashes and underscores")]
    BadMasterKeyId,
    #[error("the stored key could not be decrypted (wrong master key, or it was changed or moved)")]
    CannotOpen,
    #[error("the key could not be sealed")]
    CannotSeal,
    #[error("{0}")]
    Key(KeyError),
}

/// The key that encrypts every tenant's key. Cannot be printed; wiped on drop.
pub struct MasterKey {
    bytes: Zeroizing<[u8; MASTER_KEY_LEN]>,
    id: String,
}

impl fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MasterKey({}, [redacted])", self.id)
    }
}

impl MasterKey {
    /// From base64 text (surrounding whitespace is ignored) and the name to record with it.
    pub fn from_base64(text: &str, id: &str) -> Result<MasterKey, VaultError> {
        let valid_id = !id.is_empty()
            && id.len() <= 64
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        if !valid_id {
            return Err(VaultError::BadMasterKeyId);
        }
        let decoded = Zeroizing::new(
            STANDARD
                .decode(text.trim())
                .map_err(|_| VaultError::BadMasterKey)?,
        );
        let bytes: [u8; MASTER_KEY_LEN] = decoded
            .as_slice()
            .try_into()
            .map_err(|_| VaultError::BadMasterKey)?;
        Ok(MasterKey {
            bytes: Zeroizing::new(bytes),
            id: id.to_string(),
        })
    }

    /// The name recorded with every key this master key seals.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Loads the master key from the process environment: `Ok(None)` when none is configured (mode B is
    /// then off), an error when one is configured badly.
    pub fn load() -> Result<Option<MasterKey>, VaultError> {
        MasterKey::load_from(|name| std::env::var_os(name))
    }

    /// As [`MasterKey::load`], looking variables up with `env`.
    pub fn load_from(env: impl Fn(&str) -> Option<OsString>) -> Result<Option<MasterKey>, VaultError> {
        let set = |name: &str| env(name).filter(|value| !value.is_empty());
        let id = match set(MASTER_KEY_ID_ENV) {
            Some(value) => value.into_string().map_err(|_| VaultError::BadMasterKeyId)?,
            None => DEFAULT_MASTER_KEY_ID.to_string(),
        };
        match (set(MASTER_KEY_ENV), set(MASTER_KEY_FILE_ENV)) {
            (None, None) => Ok(None),
            (Some(_), Some(_)) => Err(VaultError::Ambiguous),
            (Some(value), None) => {
                let text = Zeroizing::new(value.into_string().map_err(|_| VaultError::BadMasterKey)?);
                MasterKey::from_base64(&text, &id).map(Some)
            }
            (None, Some(path)) => {
                let unreadable = |error: io::Error| VaultError::FileUnreadable(error.kind());
                let path = Path::new(&path);
                if !fs::metadata(path).map_err(unreadable)?.is_file() {
                    return Err(VaultError::BadMasterKey);
                }
                // Room for the key and a line ending, no more.
                let mut bytes = Zeroizing::new(Vec::with_capacity(256));
                fs::File::open(path)
                    .map_err(unreadable)?
                    .take(257)
                    .read_to_end(&mut bytes)
                    .map_err(unreadable)?;
                let text =
                    Zeroizing::new(String::from_utf8(bytes.to_vec()).map_err(|_| VaultError::BadMasterKey)?);
                MasterKey::from_base64(&text, &id).map(Some)
            }
        }
    }

    fn cipher(&self) -> Aes256Gcm {
        // The length is checked where the key is made, so this cannot fail.
        Aes256Gcm::new_from_slice(self.bytes.as_slice()).expect("a 32-byte key")
    }
}

/// A key as it is stored: ciphertext, the nonce, which master key sealed it, and the last four
/// characters (for telling keys apart on a screen; they do not reveal the key).
#[derive(Clone, PartialEq, Eq)]
pub struct Sealed {
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
    pub master_key_id: String,
    pub last4: String,
}

impl fmt::Debug for Sealed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Sealed({}, {}, [ciphertext])", self.master_key_id, self.last4)
    }
}

/// Associated data: the label, then the tenant id. Anything sealed for one tenant fails to open for another.
fn associated_data(tenant: Uuid) -> Vec<u8> {
    let mut data = LABEL.to_vec();
    data.extend_from_slice(tenant.as_bytes());
    data
}

/// Seals `key` for `tenant` with a fresh random nonce.
pub fn seal(master: &MasterKey, tenant: Uuid, key: &ApiKey) -> Result<Sealed, VaultError> {
    let nonce = Nonce::generate();
    let ciphertext = master
        .cipher()
        .encrypt(
            &nonce,
            Payload {
                msg: key.expose().as_bytes(),
                aad: &associated_data(tenant),
            },
        )
        .map_err(|_| VaultError::CannotSeal)?;
    let text = key.expose();
    Ok(Sealed {
        nonce: nonce.to_vec(),
        ciphertext,
        master_key_id: master.id.clone(),
        last4: text[text.len() - 4..].to_string(),
    })
}

/// Opens a sealed key for `tenant`. Fails if it was sealed for another tenant, with another master
/// key, or was changed.
pub fn open(master: &MasterKey, tenant: Uuid, sealed: &Sealed) -> Result<ApiKey, VaultError> {
    if sealed.nonce.len() != NONCE_LEN || sealed.master_key_id != master.id {
        return Err(VaultError::CannotOpen);
    }
    let nonce = Nonce::try_from(sealed.nonce.as_slice()).map_err(|_| VaultError::CannotOpen)?;
    let plain = Zeroizing::new(
        master
            .cipher()
            .decrypt(
                &nonce,
                Payload {
                    msg: &sealed.ciphertext,
                    aad: &associated_data(tenant),
                },
            )
            .map_err(|_| VaultError::CannotOpen)?,
    );
    let text = Zeroizing::new(String::from_utf8(plain.to_vec()).map_err(|_| VaultError::CannotOpen)?);
    ApiKey::parse(&text).map_err(VaultError::Key)
}

#[cfg(test)]
mod tests;
