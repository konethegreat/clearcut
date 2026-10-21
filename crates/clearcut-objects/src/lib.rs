// Written by Kone Tshivhinda & Claude
//! Object storage for uploaded documents (P1-02): where the bytes of an
//! invoice live, as opposed to the database rows about it (`clearcut-store`).
//!
//! Design rules, all there to keep untrusted uploads from causing harm:
//!
//! - **Content-addressed and tenant-scoped.** An object's key is built from
//!   the tenant and the SHA-256 of its bytes: `tenants/<uuid>/documents/<hex>`.
//!   Uploading the same file twice stores it once, and two tenants never share
//!   an object (so neither can learn what the other uploaded).
//! - **Keys never come from user input.** There is no way to build a key from
//!   a filename. [`ObjectKey::parse`] (for keys read back from the database)
//!   accepts exactly that one shape, so `..`, absolute paths and backslashes
//!   cannot reach the filesystem.
//! - **Streaming with a hard size limit.** Bytes are read in chunks, hashed as
//!   they are written to a temporary file, and rejected the moment they exceed
//!   the limit, whatever the sender claimed. Nothing is held in memory.
//! - **All or nothing.** The temporary file is renamed into place only after
//!   the whole upload has arrived. A failed, oversized or empty upload leaves
//!   nothing behind.
//!
//! [`LocalDisk`] is the first implementation. An S3-compatible one (MinIO in
//! the dev stack, P1-07) will implement the same [`ObjectStore`] trait.

use sha2::{Digest, Sha256};
use std::future::Future;
use std::path::PathBuf;
use tokio::fs;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("object not found")]
    NotFound,
    /// The upload exceeded `limit` bytes.
    #[error("upload is larger than the {limit}-byte limit")]
    TooLarge { limit: u64 },
    /// An empty file cannot be an invoice.
    #[error("upload is empty")]
    Empty,
    #[error("invalid object key: {0}")]
    InvalidKey(&'static str),
    #[error("storage I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// A SHA-256 hash, written as 64 lowercase hex characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Sha256Digest([u8; 32]);

impl Sha256Digest {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        self.0.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// Parses exactly 64 lowercase hex characters; anything else is `None`.
    pub fn from_hex(text: &str) -> Option<Self> {
        if text.len() != 64 || !text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return None;
        }
        let mut bytes = [0u8; 32];
        for (slot, pair) in bytes.iter_mut().zip(text.as_bytes().chunks(2)) {
            // The characters were checked above, so these are valid hex digits.
            let pair = std::str::from_utf8(pair).ok()?;
            *slot = u8::from_str_radix(pair, 16).ok()?;
        }
        Some(Self(bytes))
    }
}

/// Where an object lives: `tenants/<tenant uuid>/documents/<sha256 hex>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ObjectKey(String);

impl ObjectKey {
    /// The key for a document with this content, owned by this tenant.
    pub fn for_document(tenant: Uuid, digest: &Sha256Digest) -> Self {
        Self(format!("tenants/{tenant}/documents/{}", digest.to_hex()))
    }

    /// Reads back a key that was stored earlier (for example from the
    /// database). Only the exact shape produced by [`ObjectKey::for_document`]
    /// is accepted, so a tampered or corrupt value cannot point elsewhere.
    pub fn parse(text: &str) -> Result<Self> {
        let parts: Vec<&str> = text.split('/').collect();
        let [tenants, tenant, documents, hash] = parts[..] else {
            return Err(Error::InvalidKey("expected tenants/<uuid>/documents/<hash>"));
        };
        let tenant_ok = Uuid::parse_str(tenant).is_ok_and(|id| id.to_string() == tenant);
        if tenants != "tenants"
            || documents != "documents"
            || !tenant_ok
            || Sha256Digest::from_hex(hash).is_none()
        {
            return Err(Error::InvalidKey("expected tenants/<uuid>/documents/<hash>"));
        }
        Ok(Self(text.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Limits applied while an upload is being read.
#[derive(Debug, Clone, Copy)]
pub struct PutLimits {
    /// The most bytes accepted. Enforced on what is actually read.
    pub max_bytes: u64,
}

/// The result of storing an upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredObject {
    pub key: ObjectKey,
    pub sha256: Sha256Digest,
    pub byte_size: u64,
    /// False when the same content was already stored for this tenant.
    pub created: bool,
}

/// Somewhere to keep the bytes of uploaded documents.
pub trait ObjectStore: Send + Sync {
    /// A stream of an object's bytes.
    type Reader: AsyncRead + Send + Unpin;

    /// Streams `body` into the store for `tenant`, hashing as it goes.
    fn put<R: AsyncRead + Send + Unpin>(
        &self,
        tenant: Uuid,
        body: R,
        limits: PutLimits,
    ) -> impl Future<Output = Result<StoredObject>> + Send;

    /// Opens an object for reading, or [`Error::NotFound`].
    fn open(&self, key: &ObjectKey) -> impl Future<Output = Result<Self::Reader>> + Send;

    fn exists(&self, key: &ObjectKey) -> impl Future<Output = Result<bool>> + Send;

    /// Removes an object. Removing one that is already gone is not an error.
    fn delete(&self, key: &ObjectKey) -> impl Future<Output = Result<()>> + Send;
}

/// Objects kept as files under a root directory.
#[derive(Debug, Clone)]
pub struct LocalDisk {
    root: PathBuf,
}

impl LocalDisk {
    /// Stores under `root`, which is created on first use.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Safe because an `ObjectKey` can only have the fixed shape.
    fn path_of(&self, key: &ObjectKey) -> PathBuf {
        self.root.join(key.as_str())
    }

    /// Reads `body` into `path`, returning its hash and size. On any error the
    /// caller removes the partial file.
    async fn write_temp<R: AsyncRead + Unpin>(
        path: &std::path::Path,
        mut body: R,
        limits: PutLimits,
    ) -> Result<(Sha256Digest, u64)> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .await?;
        let mut hasher = Sha256::new();
        let mut total: u64 = 0;
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            let read = body.read(&mut buffer).await?;
            if read == 0 {
                break;
            }
            total += u64::try_from(read).unwrap_or(u64::MAX);
            if total > limits.max_bytes {
                return Err(Error::TooLarge {
                    limit: limits.max_bytes,
                });
            }
            hasher.update(&buffer[..read]);
            file.write_all(&buffer[..read]).await?;
        }
        if total == 0 {
            return Err(Error::Empty);
        }
        file.flush().await?;
        file.sync_all().await?;
        let digest: [u8; 32] = hasher.finalize().into();
        Ok((Sha256Digest(digest), total))
    }
}

impl ObjectStore for LocalDisk {
    type Reader = fs::File;

    async fn put<R: AsyncRead + Send + Unpin>(
        &self,
        tenant: Uuid,
        body: R,
        limits: PutLimits,
    ) -> Result<StoredObject> {
        let tmp_dir = self.root.join("tmp");
        fs::create_dir_all(&tmp_dir).await?;
        let tmp_path = tmp_dir.join(format!("{}.part", Uuid::new_v4()));

        let (sha256, byte_size) = match Self::write_temp(&tmp_path, body, limits).await {
            Ok(done) => done,
            Err(error) => {
                let _ = fs::remove_file(&tmp_path).await; // best effort
                return Err(error);
            }
        };

        let key = ObjectKey::for_document(tenant, &sha256);
        let final_path = self.path_of(&key);
        let published = async {
            if let Some(parent) = final_path.parent() {
                fs::create_dir_all(parent).await?;
            }
            if fs::try_exists(&final_path).await? {
                return Ok(false);
            }
            match fs::rename(&tmp_path, &final_path).await {
                Ok(()) => Ok(true),
                // Lost a race with an identical upload: the content is there.
                Err(_) if fs::try_exists(&final_path).await.unwrap_or(false) => Ok(false),
                Err(error) => Err(Error::Io(error)),
            }
        }
        .await;

        // After a successful rename the temp file is gone; otherwise remove it.
        let _ = fs::remove_file(&tmp_path).await;
        Ok(StoredObject {
            key,
            sha256,
            byte_size,
            created: published?,
        })
    }

    async fn open(&self, key: &ObjectKey) -> Result<Self::Reader> {
        match fs::File::open(self.path_of(key)).await {
            Ok(file) => Ok(file),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(Error::NotFound),
            Err(error) => Err(Error::Io(error)),
        }
    }

    async fn exists(&self, key: &ObjectKey) -> Result<bool> {
        Ok(fs::try_exists(self.path_of(key)).await?)
    }

    async fn delete(&self, key: &ObjectKey) -> Result<()> {
        match fs::remove_file(self.path_of(key)).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(Error::Io(error)),
        }
    }
}

#[cfg(test)]
mod tests;
