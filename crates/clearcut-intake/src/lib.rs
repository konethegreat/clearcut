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
//! 4. is refused if it would take the account past its storage quota, unless
//!    it may be a repeat of a document already stored (a repeat costs no room);
//!    either way it is cut off at the account's remaining room;
//! 5. is streamed into the object store (content-addressed, tenant-scoped, all
//!    or nothing), then recorded as a document, then audited.
//!
//! The file is stored *before* the row is written. If the row then fails, the
//! file is deliberately left in place: it is harmless (named by its content and
//! owned by one tenant), a retry finds it already there, and deleting it could
//! race with an identical upload that has just found it. A sweep for files no
//! document refers to can come later. The one exception is a quota refusal
//! after the file was written (another upload took the last room first): that
//! content is new, so its file is removed. An identical upload racing that
//! removal at the same instant can find the file gone, leaving the row it
//! writes pointing at a missing file (reported as such, fixed by uploading
//! again): the same narrow race as erasing a document while uploading it.
//!
//! Uploaded files are untrusted bytes. Nothing here opens or parses them beyond
//! checking the first few bytes against the declared media type (a PDF must
//! start `%PDF-`, an image with its signature, XML with `<`); the file name is
//! kept only as cleaned metadata and never forms a path.
//!
//! Deletion lives here too: a person's delete and the server's retention sweep
//! share one erasure, which cancels a waiting job, refuses a running one, and
//! removes the file once no document of the account uses those bytes.

use clearcut_core::auth::{Principal, Scope};
use clearcut_core::document::DocumentStatus;
use clearcut_core::jobs::JobKind;
use clearcut_objects::{ObjectKey, ObjectStore, PutLimits};
use clearcut_store::{AuditEntry, DocId, NewDocument, NewJob, Store, TenantId};
use std::io::ErrorKind;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncReadExt, ReadBuf};

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

/// How many leading bytes are looked at to tell the type: enough for a byte-order mark, some
/// whitespace and the first tag of an XML file.
const HEAD_BYTES: usize = 64;

/// A stream whose first `filled` bytes are already read: the head is handed out again, then
/// the rest comes from the sender.
struct Prefixed<R> {
    head: [u8; HEAD_BYTES],
    filled: usize,
    offset: usize,
    inner: R,
}

impl<R: AsyncRead + Unpin> AsyncRead for Prefixed<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = &mut *self;
        if this.offset < this.filled {
            let take = (this.filled - this.offset).min(buf.remaining());
            buf.put_slice(&this.head[this.offset..this.offset + take]);
            this.offset += take;
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut this.inner).poll_read(cx, buf)
    }
}

/// Whether the leading bytes match what the media type promises. XML is the lenient one: an
/// optional byte-order mark, then whitespace, then the first tag, which may be UTF-16.
fn signature_matches(media_type: MediaType, head: &[u8]) -> bool {
    match media_type {
        MediaType::Pdf => head.starts_with(b"%PDF-"),
        MediaType::Png => head.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]),
        MediaType::Jpeg => head.starts_with(&[0xFF, 0xD8, 0xFF]),
        MediaType::Tiff => head.starts_with(b"II*\0") || head.starts_with(b"MM\0*"),
        MediaType::Xml => xml_starts(head),
    }
}

fn xml_starts(head: &[u8]) -> bool {
    if let Some(rest) = head.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return first_is_tag(rest);
    }
    if let Some(rest) = head.strip_prefix(&[0xFF, 0xFE]) {
        return utf16_first_is_tag(rest, true);
    }
    if let Some(rest) = head.strip_prefix(&[0xFE, 0xFF]) {
        return utf16_first_is_tag(rest, false);
    }
    first_is_tag(head)
}

fn first_is_tag(bytes: &[u8]) -> bool {
    bytes
        .iter()
        .find(|byte| !byte.is_ascii_whitespace())
        .is_some_and(|byte| *byte == b'<')
}

fn utf16_first_is_tag(bytes: &[u8], little_endian: bool) -> bool {
    for pair in bytes.as_chunks::<2>().0 {
        let unit = if little_endian {
            u16::from_le_bytes([pair[0], pair[1]])
        } else {
            u16::from_be_bytes([pair[0], pair[1]])
        };
        match unit {
            0x20 | 0x09 | 0x0A | 0x0D => continue,
            _ => return unit == u16::from(b'<'),
        }
    }
    false
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
    /// Storing the file would put the account over its storage quota.
    #[error("the account's storage is full: {used_bytes} of {quota_bytes} bytes used")]
    QuotaExceeded { used_bytes: u64, quota_bytes: u64 },
    #[error("the file is empty")]
    Empty,
    /// The first bytes are not what the declared media type says they are.
    #[error("the file is not the declared type")]
    ContentMismatch,
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

/// The size limits an upload is held to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The largest single file accepted.
    pub max_upload_bytes: u64,
    /// The most bytes one account may have stored at once.
    pub tenant_quota_bytes: u64,
}

/// Accepts uploads.
pub struct Intake<O> {
    store: Store,
    objects: O,
    limits: Limits,
}

impl<O: ObjectStore> Intake<O> {
    pub fn new(store: Store, objects: O, limits: Limits) -> Self {
        Self {
            store,
            objects,
            limits,
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
            .is_some_and(|length| length > self.limits.max_upload_bytes)
        {
            return Err(self.too_large());
        }
        let tenant = TenantId(by.tenant());
        let quota = self.limits.tenant_quota_bytes;
        // If the room cannot be read, carry on: the insert checks the quota again,
        // and a database failure must not change whether the file is kept for a
        // retry (the empty room below also means "no early quota check").
        let used = self.store.tenant_storage_used(tenant).await.ok();
        let room = used.map(|used| quota.saturating_sub(used));
        if let (Some(used), Some(declared)) = (used, upload.declared_length) {
            if declared > quota.saturating_sub(used) {
                // A repeat of a document already stored costs no room, and its
                // size is the only thing known before it is read. Anything else
                // of that size cannot fit, so it is refused unread.
                let maybe_a_repeat = self
                    .store
                    .tenant_has_document_of_size(tenant, declared)
                    .await
                    .unwrap_or(true);
                if !maybe_a_repeat {
                    return Err(IntakeError::QuotaExceeded {
                        used_bytes: used,
                        quota_bytes: quota,
                    });
                }
            }
        }
        let filename = clean_filename(upload.filename.as_deref());

        // Cut the stream off at the account's remaining room, so an upload
        // that claims to be small cannot fill the disk before the check. A
        // possible repeat may take the size it declared, and no more.
        let max_bytes = match (room, upload.declared_length) {
            (Some(room), Some(declared)) if declared > room => declared,
            (Some(room), _) => self.limits.max_upload_bytes.min(room),
            (None, _) => self.limits.max_upload_bytes,
        };
        let room_limited = max_bytes < self.limits.max_upload_bytes;
        // The first bytes are read before anything is stored, so a file that lies about its
        // type costs one small read. They are put back in front of the stream for the store.
        let mut body = upload.body;
        let mut head = [0u8; HEAD_BYTES];
        let mut filled = 0;
        while filled < head.len() {
            match body.read(&mut head[filled..]).await {
                Ok(0) => break,
                Ok(read) => filled += read,
                Err(_) => return Err(IntakeError::Interrupted),
            }
        }
        if filled == 0 {
            return Err(IntakeError::Empty);
        }
        if !signature_matches(media_type, &head[..filled]) {
            return Err(IntakeError::ContentMismatch);
        }
        let body = Prefixed {
            head,
            filled,
            offset: 0,
            inner: body,
        };
        let stored = self
            .objects
            .put(by.tenant(), body, PutLimits { max_bytes })
            .await
            .map_err(|error| match error {
                clearcut_objects::Error::TooLarge { .. } if room_limited => {
                    // The cap was the account's room, not the file limit.
                    IntakeError::QuotaExceeded {
                        used_bytes: used.unwrap_or(quota),
                        quota_bytes: quota,
                    }
                }
                other => self.explain(other),
            })?;
        let byte_size = i64::try_from(stored.byte_size).map_err(|_| self.too_large())?;

        let added = match self
            .store
            .add_document_within(
                tenant,
                &NewDocument {
                    original_filename: filename.clone(),
                    content_type: media_type.as_str().to_string(),
                    byte_size,
                    sha256: *stored.sha256.as_bytes(),
                    storage_key: stored.key.as_str().to_string(),
                    page_count: None,
                },
                quota,
            )
            .await
        {
            Ok(added) => added,
            Err(clearcut_store::Error::StorageQuotaExceeded {
                used_bytes,
                quota_bytes,
            }) => {
                // Another upload for this account filled the last room first.
                // Its content is new (a repeat would have been recognised as a
                // duplicate), so this file belongs to nothing: remove it.
                if stored.created {
                    if let Err(error) = self.objects.delete(&stored.key).await {
                        tracing::warn!(%error, "a refused upload's file could not be removed");
                    }
                }
                return Err(IntakeError::QuotaExceeded {
                    used_bytes,
                    quota_bytes,
                });
            }
            Err(error) => return Err(error.into()),
        };

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
    pub async fn erase(&self, by: &Principal, document: DocId) -> Result<(), IntakeError> {
        by.require(Scope::DocumentsWrite)
            .map_err(|_| IntakeError::Forbidden(Scope::DocumentsWrite))?;
        let requested = AuditEntry {
            actor: serde_json::json!({ "type": "api_key", "id": by.key_id().to_string() }),
            action: "document.erase_requested".into(),
            subject_type: "document".into(),
            subject_id: document.0.to_string(),
            detail: serde_json::json!({}),
        };
        self.erase_as(TenantId(by.tenant()), document, requested).await
    }

    /// Erases a document whose retention period has passed: the same erasure as a person's
    /// deletion, recorded as asked for by the retention sweep with the period as the reason.
    /// No key is involved: the server decides this, not a caller. A document still being read
    /// is [`IntakeError::Busy`] and is left for a later sweep.
    pub async fn erase_expired(
        &self,
        tenant: TenantId,
        document: DocId,
        days: u16,
    ) -> Result<(), IntakeError> {
        let requested = AuditEntry {
            actor: serde_json::json!({ "type": "retention" }),
            action: "document.erase_requested".into(),
            subject_type: "document".into(),
            subject_id: document.0.to_string(),
            detail: serde_json::json!({ "retention_days": days }),
        };
        self.erase_as(tenant, document, requested).await
    }

    /// The erasure both above share.
    ///
    /// A job that is only waiting is cancelled first; one that is **running** refuses
    /// ([`IntakeError::Busy`]): it would be writing what is being removed. The file is removed **after** the rows, and only when no
    /// document of the tenant still points at it (a file is named by its content, and one upload
    /// of the same bytes reuses it). If removing the file fails the rows are already gone; that is
    /// reported, and the file is left for a sweep. What stays by design: the audit entry naming the
    /// document (no content) and the call log's costs (no content).
    async fn erase_as(
        &self,
        tenant: TenantId,
        document: DocId,
        requested: AuditEntry,
    ) -> Result<(), IntakeError> {
        let file = self.store.document_file(tenant, document).await?;
        // A job that is only waiting (for a key, a retry, its turn) is stopped first; one that is
        // running is not interrupted: the person is told to try again when it has finished.
        let actor = requested.actor.clone();
        let jobs = self.store.jobs_for_document(tenant, document).await?;
        for job in jobs.iter().filter(|job| !job.state.is_terminal()) {
            if job.state == clearcut_core::jobs::JobState::Running {
                return Err(IntakeError::Busy);
            }
            if !self
                .store
                .request_cancel(tenant, job.id, &actor)
                .await?
                .is_terminal()
            {
                return Err(IntakeError::Busy);
            }
        }
        // Who asked and why is recorded first (the database function can only say "system"). No
        // content.
        self.store.record_audit(tenant, &requested).await?;
        self.store
            .erase_document(tenant, document)
            .await
            .map_err(|error| match error {
                clearcut_store::Error::DocumentBusy => IntakeError::Busy,
                other => other.into(),
            })?;
        if self.store.storage_key_in_use(tenant, &file.storage_key).await? {
            return Ok(());
        }
        let key = ObjectKey::parse(&file.storage_key).map_err(IntakeError::Storage)?;
        self.objects.delete(&key).await.map_err(IntakeError::Storage)
    }

    fn too_large(&self) -> IntakeError {
        IntakeError::TooLarge {
            max_bytes: self.limits.max_upload_bytes,
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
