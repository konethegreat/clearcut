// Written by Kone Tshivhinda & Claude
//! Document intake (P1-02): what happens when someone uploads a file.
//!
//! This crate is the single place that decides how an upload is accepted, so
//! the REST endpoint now and the MCP `upload_document` tool later (P2-11) cannot
//! drift apart. It knows nothing about HTTP. In order, an upload:
//!
//! 1. needs the `documents:write` scope, checked **before anything is read**;
//! 2. must be an accepted media type (PDF, PNG, JPEG, TIFF or XML), also checked
//!    before the body is read, so a rejected request costs nothing;
//! 3. is refused up front if its declared length is over the limit, and in any
//!    case is cut off the moment it actually exceeds the limit, whatever the
//!    sender claimed;
//! 4. is streamed into the object store (content-addressed, tenant-scoped, all
//!    or nothing), then recorded as a document, then audited.
//!
//! The file is stored *before* the row is written. If the row then fails, the
//! file is deliberately left in place: it is harmless (named by its content and
//! owned by one tenant), a retry finds it already there, and deleting it could
//! race with an identical upload that has just found it. A sweep for files no
//! document refers to can come later.
//!
//! Uploaded files are untrusted bytes. Nothing here opens or parses them; the
//! file name is kept only as cleaned metadata and never forms a path. (Checking
//! that the bytes really are what the media type claims is P2-02.)

use clearcut_core::auth::{Principal, Scope};
use clearcut_core::document::DocumentStatus;
use clearcut_core::jobs::JobKind;
use clearcut_objects::{ObjectKey, ObjectStore, PutLimits};
use clearcut_store::{AuditEntry, DocId, NewDocument, NewJob, Store, TenantId};
use std::io::ErrorKind;
use tokio::io::AsyncRead;

/// The kinds of file ClearCut accepts as an invoice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaType {
    Pdf,
    Png,
    Jpeg,
    Tiff,
    Xml,
}

impl MediaType {
    /// Reads a `Content-Type` value: the type is case-insensitive and any
    /// parameters (`; charset=...`) are ignored. Anything not exactly one of the
    /// accepted types, including lists and wildcards, is `None`.
    pub fn parse(content_type: &str) -> Option<MediaType> {
        let base = content_type.split(';').next()?.trim().to_ascii_lowercase();
        match base.as_str() {
            "application/pdf" => Some(MediaType::Pdf),
            "image/png" => Some(MediaType::Png),
            "image/jpeg" => Some(MediaType::Jpeg),
            "image/tiff" => Some(MediaType::Tiff),
            "application/xml" | "text/xml" => Some(MediaType::Xml),
            _ => None,
        }
    }

    /// The one canonical name stored for this type.
    pub fn as_str(self) -> &'static str {
        match self {
            MediaType::Pdf => "application/pdf",
            MediaType::Png => "image/png",
            MediaType::Jpeg => "image/jpeg",
            MediaType::Tiff => "image/tiff",
            MediaType::Xml => "application/xml",
        }
    }
}

/// The longest file name kept, in characters.
const MAX_FILENAME_CHARS: usize = 255;

/// Turns whatever the sender called the file into safe metadata: only the last
/// path component, no control or invisible direction-changing characters (an
/// "invoice\u{202e}fdp.exe" trick), runs of spaces collapsed, no leading or
/// trailing spaces or dots, at most 255 characters, and `upload` if nothing
/// usable is left. The result is only ever displayed or recorded; it never
/// becomes a file path.
pub fn clean_filename(raw: Option<&str>) -> String {
    let last_component = raw.unwrap_or("").rsplit(['/', '\\']).next().unwrap_or("");
    let mut cleaned = String::new();
    let mut last_was_space = false;
    for c in last_component.chars() {
        if is_invisible(c) {
            continue;
        }
        let c = if c == '\t' { ' ' } else { c };
        if c.is_control() {
            continue;
        }
        if c.is_whitespace() {
            if !last_was_space {
                cleaned.push(' ');
            }
            last_was_space = true;
        } else {
            cleaned.push(c);
            last_was_space = false;
        }
    }
    let trimmed = |s: &str| s.trim_matches(|c: char| c == ' ' || c == '.').to_string();
    let short: String = trimmed(&cleaned).chars().take(MAX_FILENAME_CHARS).collect();
    let short = trimmed(&short);
    if short.is_empty() {
        "upload".to_string()
    } else {
        short
    }
}

/// Characters with no visible form that can disguise a name: zero-width and
/// direction marks, direction overrides and isolates, invisible operators, the
/// soft hyphen and the byte-order mark.
fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00ad}'
            | '\u{200b}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{2069}'
            | '\u{feff}'
    )
}

/// One upload, as handed over by a transport (HTTP, MCP).
pub struct Upload<R> {
    /// The sender's name for the file, if any. Cleaned before it is kept.
    pub filename: Option<String>,
    /// The `Content-Type` as sent. Must be one of the accepted [`MediaType`]s.
    pub content_type: String,
    /// The length the sender claims, if known. Only used to refuse early; the
    /// real size is measured as the bytes arrive.
    pub declared_length: Option<u64>,
    /// The bytes, streamed.
    pub body: R,
}

/// What a successful upload produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Uploaded {
    pub document: DocId,
    /// True when this tenant had already uploaded exactly this content, so
    /// `document` is the existing one.
    pub duplicate: bool,
    pub sha256_hex: String,
    pub byte_size: u64,
    /// The cleaned file name sent with this upload. For a repeat upload
    /// (`duplicate`) that is this request's name; the existing document keeps
    /// the name it was first uploaded with.
    pub filename: String,
    pub media_type: MediaType,
    /// The job that reads the document, queued by this upload (or the one already queued). `None`
    /// for a repeat upload of a document that has already been worked on.
    pub job: Option<uuid::Uuid>,
}

/// Why an upload was refused or failed.
#[derive(Debug, thiserror::Error)]
pub enum IntakeError {
    /// The caller's key lacks this scope.
    #[error("missing required scope {}", .0.as_str())]
    Forbidden(Scope),
    #[error("unsupported media type")]
    UnsupportedMediaType,
    #[error("the file is larger than the {max_bytes}-byte limit")]
    TooLarge { max_bytes: u64 },
    #[error("the file is empty")]
    Empty,
    /// The sender went away before the whole file arrived.
    #[error("the upload was interrupted")]
    Interrupted,
    #[error("could not store the file: {0}")]
    Storage(clearcut_objects::Error),
    #[error("could not record the document: {0}")]
    Database(#[from] clearcut_store::Error),
    /// A job is queued or running for the document, so it cannot be erased right now.
    #[error("the document is being worked on")]
    Busy,
}

/// Accepts uploads.
pub struct Intake<O> {
    store: Store,
    objects: O,
    max_bytes: u64,
}

impl<O: ObjectStore> Intake<O> {
    /// `max_bytes` is the largest file accepted.
    pub fn new(store: Store, objects: O, max_bytes: u64) -> Self {
        Self {
            store,
            objects,
            max_bytes,
        }
    }

    /// Accepts one upload for the caller's tenant. See the module docs for the
    /// order of the steps and what is left behind when one fails.
    pub async fn upload<R>(&self, by: &Principal, upload: Upload<R>) -> Result<Uploaded, IntakeError>
    where
        R: AsyncRead + Send + Unpin,
    {
        by.require(Scope::DocumentsWrite)
            .map_err(|_| IntakeError::Forbidden(Scope::DocumentsWrite))?;
        let media_type = MediaType::parse(&upload.content_type).ok_or(IntakeError::UnsupportedMediaType)?;
        if upload
            .declared_length
            .is_some_and(|length| length > self.max_bytes)
        {
            return Err(self.too_large());
        }
        let filename = clean_filename(upload.filename.as_deref());

        let stored = self
            .objects
            .put(
                by.tenant(),
                upload.body,
                PutLimits {
                    max_bytes: self.max_bytes,
                },
            )
            .await
            .map_err(|error| self.explain(error))?;
        let byte_size = i64::try_from(stored.byte_size).map_err(|_| self.too_large())?;

        let tenant = TenantId(by.tenant());
        let added = self
            .store
            .add_document(
                tenant,
                &NewDocument {
                    original_filename: filename.clone(),
                    content_type: media_type.as_str().to_string(),
                    byte_size,
                    sha256: *stored.sha256.as_bytes(),
                    storage_key: stored.key.as_str().to_string(),
                    page_count: None,
                },
            )
            .await?;

        // Recorded even for a repeat, so every attempt is visible. The entry
        // names the file and its size, never its content.
        let entry = AuditEntry {
            actor: serde_json::json!({ "type": "api_key", "id": by.key_id().to_string() }),
            action: "document.uploaded".into(),
            subject_type: "document".into(),
            subject_id: added.id.0.to_string(),
            detail: serde_json::json!({
                "filename": filename,
                "media_type": media_type.as_str(),
                "byte_size": stored.byte_size,
                "duplicate": added.duplicate,
            }),
        };
        self.store.record_audit(tenant, &entry).await?;

        // A new document is queued to be read. So is a repeat upload of one that was never
        // started (an earlier upload that stopped before its job was queued); a document that
        // has been worked on is left as it is. Asking again while a job is queued or running
        // returns that job.
        let job = if !added.duplicate
            || self.store.document_status(tenant, added.id).await? == DocumentStatus::Received
        {
            let queued = self
                .store
                .enqueue_job(
                    tenant,
                    &NewJob::new(added.id, JobKind::ProcessDocument),
                    &serde_json::json!({ "type": "api_key", "id": by.key_id().to_string() }),
                )
                .await?;
            Some(queued.id)
        } else {
            None
        };

        Ok(Uploaded {
            document: added.id,
            duplicate: added.duplicate,
            sha256_hex: stored.sha256.to_hex(),
            byte_size: stored.byte_size,
            filename,
            media_type,
            job,
        })
    }

    /// Erases a document for good (POPIA): its events, readings, jobs and row, through the vetted
    /// `erase_document` database function, and then its file. Needs `documents:write`.
    ///
    /// Refused while a job is queued or running on it ([`IntakeError::Busy`]): the job would be
    /// writing what is being removed. The file is removed **after** the rows, and only when no
    /// document of the tenant still points at it (a file is named by its content, and one upload
    /// of the same bytes reuses it). If removing the file fails the rows are already gone; that is
    /// reported, and the file is left for a sweep. What stays by design: the audit entry naming the
    /// document (no content) and the call log's costs (no content).
    pub async fn erase(&self, by: &Principal, document: DocId) -> Result<(), IntakeError> {
        by.require(Scope::DocumentsWrite)
            .map_err(|_| IntakeError::Forbidden(Scope::DocumentsWrite))?;
        let tenant = TenantId(by.tenant());
        let file = self.store.document_file(tenant, document).await?;
        let jobs = self.store.jobs_for_document(tenant, document).await?;
        if jobs.iter().any(|job| !job.state.is_terminal()) {
            return Err(IntakeError::Busy);
        }
        self.store.erase_document(tenant, document).await?;
        if self.store.storage_key_in_use(tenant, &file.storage_key).await? {
            return Ok(());
        }
        let key = ObjectKey::parse(&file.storage_key).map_err(IntakeError::Storage)?;
        self.objects.delete(&key).await.map_err(IntakeError::Storage)
    }

    fn too_large(&self) -> IntakeError {
        IntakeError::TooLarge {
            max_bytes: self.max_bytes,
        }
    }

    /// Sorts an object-store failure into "the client's fault" and "ours".
    fn explain(&self, error: clearcut_objects::Error) -> IntakeError {
        use clearcut_objects::Error as StoreError;
        match error {
            StoreError::TooLarge { .. } => self.too_large(),
            StoreError::Empty => IntakeError::Empty,
            StoreError::Io(io)
                if matches!(
                    io.kind(),
                    ErrorKind::ConnectionAborted
                        | ErrorKind::ConnectionReset
                        | ErrorKind::BrokenPipe
                        | ErrorKind::UnexpectedEof
                ) =>
            {
                IntakeError::Interrupted
            }
            other => IntakeError::Storage(other),
        }
    }
}

#[cfg(test)]
mod tests;
