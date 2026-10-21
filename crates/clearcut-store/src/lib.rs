// Written by Kone Tshivhinda & Claude
//! Postgres storage for ClearCut (ADR-0006): tenants, documents, the
//! append-only event log and the audit log.
//!
//! This crate does I/O; `clearcut-core` does not. Business rules stay in the
//! core. The store's job is to keep what the core produces, return it
//! unchanged, and refuse anything that would break the guarantees the schema
//! makes:
//!
//! - **Events are stored as the core's canonical JSON** and re-validated by
//!   `Event`'s own rules every time they are read.
//! - **Every call takes the tenant** and runs in a transaction that has told
//!   Postgres which tenant it acts for. The service connects as the restricted
//!   role `clearcut_app`, whose row-level security policies (ADR-0007) match
//!   only that tenant's rows, and nothing at all if no tenant is set, so a
//!   forgotten `where tenant_id = ...` cannot leak another tenant's data. A
//!   document of another tenant looks like it does not exist.
//! - **Appending is atomic and idempotent.** A batch is stored whole or not at
//!   all, and storing an event again with the same ID and the same content is
//!   a no-op (so a retried job is safe), while the same ID with different
//!   content is refused.
//!
//! Queries use bound parameters throughout, never string-built SQL.

use clearcut_core::auth::{Principal, Scope};
use clearcut_core::event::{DocumentId as CoreDocumentId, Event};

pub mod keys;
use sqlx::postgres::PgPool;
use std::num::NonZeroU32;
use time::OffsetDateTime;
use uuid::Uuid;

/// The migrations, embedded in the binary at compile time.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("migration error: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
    /// The document does not exist, or belongs to another tenant.
    #[error("document not found")]
    DocumentNotFound,
    /// An event with this ID is already stored with different content.
    #[error("event {0} already exists with different content")]
    EventIdConflict(String),
    /// An event cites a different document than the one it is being stored under.
    #[error("event {0} cites a different document")]
    EventForOtherDocument(String),
    /// A stored event no longer passes the core's rules.
    #[error("a stored event is invalid: {0}")]
    InvalidStoredEvent(String),
    #[error("could not serialise event: {0}")]
    Serialize(serde_json::Error),
    #[error("unknown document status in the database: {0}")]
    UnknownStatus(String),
    /// The key is missing, malformed, unknown, wrong, revoked or expired. The
    /// text is the same for all of them on purpose.
    #[error("authentication failed")]
    Unauthenticated,
    #[error("API key not found")]
    ApiKeyNotFound,
    #[error("an API key needs a name")]
    BlankName,
    #[error("an API key needs at least one scope")]
    NoScopes,
    /// A stored key carries a scope this version does not know. Never ignored.
    #[error("a stored API key has an unknown scope: {0}")]
    InvalidStoredScopes(String),
    #[error("could not get random bytes for a key: {0}")]
    Random(String),
    /// The caller lacks a scope (from `clearcut-core::auth`).
    #[error(transparent)]
    Auth(#[from] clearcut_core::Error),
}

/// Identifies a tenant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TenantId(pub Uuid);

/// Identifies a stored document. Citations in events use it as text; see
/// [`DocId::as_core_id`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DocId(pub Uuid);

impl DocId {
    /// The core's document ID for this document: its UUID as text. Events about
    /// the document must cite exactly this.
    pub fn as_core_id(&self) -> CoreDocumentId {
        CoreDocumentId::new(self.0.to_string())
    }
}

/// Where a document is in the pipeline. There is no failed state: a document
/// ends `Done` or `NeedsReview` (ADR-0005).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentStatus {
    Received,
    Processing,
    NeedsReview,
    Done,
}

impl DocumentStatus {
    fn as_str(self) -> &'static str {
        match self {
            DocumentStatus::Received => "received",
            DocumentStatus::Processing => "processing",
            DocumentStatus::NeedsReview => "needs_review",
            DocumentStatus::Done => "done",
        }
    }

    fn parse(text: &str) -> Result<Self> {
        Ok(match text {
            "received" => DocumentStatus::Received,
            "processing" => DocumentStatus::Processing,
            "needs_review" => DocumentStatus::NeedsReview,
            "done" => DocumentStatus::Done,
            other => return Err(Error::UnknownStatus(other.to_string())),
        })
    }
}

/// What is known about an uploaded file.
#[derive(Debug, Clone)]
pub struct NewDocument {
    pub original_filename: String,
    pub content_type: String,
    pub byte_size: i64,
    pub sha256: [u8; 32],
    /// Key in object storage (P1-02), never a file path.
    pub storage_key: String,
    pub page_count: Option<i32>,
}

/// The result of [`Store::add_document`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddedDocument {
    pub id: DocId,
    /// True when this tenant had already uploaded the same content, so `id` is
    /// the existing document.
    pub duplicate: bool,
}

/// One line of the audit log.
#[derive(Debug, Clone)]
pub struct AuditEntry {
    /// Who did it, e.g. `{"type":"system"}` or `{"type":"api_key","id":"..."}`.
    pub actor: serde_json::Value,
    /// What happened, e.g. `document.uploaded`.
    pub action: String,
    pub subject_type: String,
    pub subject_id: String,
    pub detail: serde_json::Value,
}

/// A key lasts this many days unless a shorter life is asked for. There is no
/// way to ask for a longer one or for none (ADR-0007).
pub const DEFAULT_KEY_LIFETIME_DAYS: u32 = 365;

/// A request for a new API key.
#[derive(Debug, Clone)]
pub struct NewApiKey {
    pub name: String,
    pub scopes: Vec<Scope>,
    /// Days until it expires; `None` means [`DEFAULT_KEY_LIFETIME_DAYS`]. It can
    /// only be made shorter than the default, never longer.
    pub lifetime_days: Option<NonZeroU32>,
}

/// A newly created key. The token is the only copy of the secret there will
/// ever be: show it to the owner once and drop it.
#[derive(Debug)]
pub struct CreatedApiKey {
    pub id: Uuid,
    pub token: keys::ApiKeyToken,
    pub expires_at: OffsetDateTime,
}

/// What may be shown about an existing key. Never its secret or its hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiKeyInfo {
    pub id: Uuid,
    pub name: String,
    pub scopes: Vec<Scope>,
    pub created_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
    pub last_used_at: Option<OffsetDateTime>,
    pub revoked_at: Option<OffsetDateTime>,
}

/// One row of the key listing, in the order of the select.
type ApiKeyRow = (
    Uuid,
    String,
    Vec<String>,
    OffsetDateTime,
    OffsetDateTime,
    Option<OffsetDateTime>,
    Option<OffsetDateTime>,
);

/// A handle to the database.
#[derive(Debug, Clone)]
pub struct Store {
    pool: PgPool,
}

impl Store {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// The underlying connection pool.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Starts a transaction that acts for `tenant`: the tenant is recorded as a
    /// transaction-local setting that the row-level security policies read.
    /// `set_config(.., true)` expires with the transaction, so it can never
    /// leak onto the next user of a pooled connection.
    async fn begin(&self, tenant: TenantId) -> Result<sqlx::Transaction<'static, sqlx::Postgres>> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("select set_config('clearcut.tenant', $1, true)")
            .bind(tenant.0.to_string())
            .execute(&mut *tx)
            .await?;
        Ok(tx)
    }

    /// Applies any migrations that have not been applied yet.
    pub async fn migrate(pool: &PgPool) -> Result<()> {
        MIGRATOR.run(pool).await?;
        Ok(())
    }

    /// Creates a tenant. This is an owner-level operation (the `clearcut-admin`
    /// command); the restricted application role is not allowed to do it.
    pub async fn create_tenant(&self, name: &str) -> Result<TenantId> {
        let id: Uuid = sqlx::query_scalar("insert into tenants (name) values ($1) returning id")
            .bind(name)
            .fetch_one(&self.pool)
            .await?;
        Ok(TenantId(id))
    }

    /// Records an uploaded document. Uploading the same content again for the
    /// same tenant returns the existing document with `duplicate` set.
    pub async fn add_document(&self, tenant: TenantId, document: &NewDocument) -> Result<AddedDocument> {
        let mut tx = self.begin(tenant).await?;
        let inserted: Option<Uuid> = sqlx::query_scalar(
            "insert into documents \
             (tenant_id, original_filename, content_type, byte_size, sha256, storage_key, page_count) \
             values ($1, $2, $3, $4, $5, $6, $7) \
             on conflict (tenant_id, sha256) do nothing \
             returning id",
        )
        .bind(tenant.0)
        .bind(&document.original_filename)
        .bind(&document.content_type)
        .bind(document.byte_size)
        .bind(&document.sha256[..])
        .bind(&document.storage_key)
        .bind(document.page_count)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(id) = inserted {
            tx.commit().await?;
            return Ok(AddedDocument {
                id: DocId(id),
                duplicate: false,
            });
        }
        let existing: Uuid =
            sqlx::query_scalar("select id from documents where tenant_id = $1 and sha256 = $2")
                .bind(tenant.0)
                .bind(&document.sha256[..])
                .fetch_one(&mut *tx)
                .await?;
        tx.commit().await?;
        Ok(AddedDocument {
            id: DocId(existing),
            duplicate: true,
        })
    }

    pub async fn document_status(&self, tenant: TenantId, document: DocId) -> Result<DocumentStatus> {
        let mut tx = self.begin(tenant).await?;
        let status: Option<String> =
            sqlx::query_scalar("select status from documents where tenant_id = $1 and id = $2")
                .bind(tenant.0)
                .bind(document.0)
                .fetch_optional(&mut *tx)
                .await?;
        tx.commit().await?;
        DocumentStatus::parse(&status.ok_or(Error::DocumentNotFound)?)
    }

    pub async fn set_document_status(
        &self,
        tenant: TenantId,
        document: DocId,
        status: DocumentStatus,
    ) -> Result<()> {
        let mut tx = self.begin(tenant).await?;
        let result = sqlx::query(
            "update documents set status = $3, updated_at = now() where tenant_id = $1 and id = $2",
        )
        .bind(tenant.0)
        .bind(document.0)
        .bind(status.as_str())
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() == 0 {
            return Err(Error::DocumentNotFound);
        }
        tx.commit().await?;
        Ok(())
    }

    /// Appends events to a document's log, all or nothing. Re-appending an
    /// event with the same ID and the same content does nothing; the same ID
    /// with different content is an [`Error::EventIdConflict`]. Every event
    /// must cite this document.
    pub async fn append_events(&self, tenant: TenantId, document: DocId, events: &[Event]) -> Result<()> {
        let mut tx = self.begin(tenant).await?;
        require_document(&mut *tx, tenant, document).await?;
        let cited = document.as_core_id();
        for event in events {
            if event.citation().document != cited {
                return Err(Error::EventForOtherDocument(event.id().as_str().to_string()));
            }
            let body = serde_json::to_value(event).map_err(Error::Serialize)?;
            let inserted: Option<i64> = sqlx::query_scalar(
                "insert into events (tenant_id, document_id, id, body) values ($1, $2, $3, $4) \
                 on conflict (document_id, id) do nothing returning seq",
            )
            .bind(tenant.0)
            .bind(document.0)
            .bind(event.id().as_str())
            .bind(&body)
            .fetch_optional(&mut *tx)
            .await?;
            if inserted.is_none() {
                let stored: serde_json::Value =
                    sqlx::query_scalar("select body from events where document_id = $1 and id = $2")
                        .bind(document.0)
                        .bind(event.id().as_str())
                        .fetch_one(&mut *tx)
                        .await?;
                if stored != body {
                    return Err(Error::EventIdConflict(event.id().as_str().to_string()));
                }
            }
        }
        tx.commit().await?;
        Ok(())
    }

    /// Every event stored for a document, oldest first. Each is re-validated by
    /// the core's rules as it is read.
    pub async fn load_events(&self, tenant: TenantId, document: DocId) -> Result<Vec<Event>> {
        let mut tx = self.begin(tenant).await?;
        require_document(&mut *tx, tenant, document).await?;
        let bodies: Vec<serde_json::Value> = sqlx::query_scalar(
            "select body from events where tenant_id = $1 and document_id = $2 order by seq",
        )
        .bind(tenant.0)
        .bind(document.0)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        bodies
            .into_iter()
            .map(|body| {
                serde_json::from_value::<Event>(body).map_err(|e| Error::InvalidStoredEvent(e.to_string()))
            })
            .collect()
    }

    /// Erases one document for a data-subject request (POPIA): its events and its
    /// row, plus one audit entry that names the document and holds no invoice
    /// content. The application cannot delete events itself; this calls the
    /// vetted `erase_document` database function (ADR-0007), which checks the
    /// tenant. The stored file is removed separately by the caller, and only if
    /// no other document still uses the same content.
    pub async fn erase_document(&self, tenant: TenantId, document: DocId) -> Result<()> {
        let mut tx = self.begin(tenant).await?;
        let result = sqlx::query("select erase_document($1, $2)")
            .bind(tenant.0)
            .bind(document.0)
            .execute(&mut *tx)
            .await;
        match result {
            Ok(_) => {
                tx.commit().await?;
                Ok(())
            }
            // The function raises "no data found" for a missing or foreign document.
            Err(sqlx::Error::Database(error)) if error.code().as_deref() == Some("P0002") => {
                Err(Error::DocumentNotFound)
            }
            Err(error) => Err(error.into()),
        }
    }

    pub async fn record_audit(&self, tenant: TenantId, entry: &AuditEntry) -> Result<()> {
        let mut tx = self.begin(tenant).await?;
        insert_audit(&mut tx, tenant, entry).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Checks a presented API key and says who it is. Every kind of failure
    /// (malformed, unknown, wrong secret, revoked, expired) is the same
    /// [`Error::Unauthenticated`], so a caller learns nothing about which keys
    /// exist. The check runs in a security-definer database function because
    /// the tenant is not known until the key has been checked.
    pub async fn authenticate(&self, presented: &str) -> Result<Principal> {
        let parsed = keys::parse(presented).ok_or(Error::Unauthenticated)?;
        let found: Option<(Uuid, Vec<String>)> =
            sqlx::query_as("select out_tenant_id, out_scopes from authenticate_api_key($1, $2)")
                .bind(parsed.id)
                .bind(&parsed.secret_hash[..])
                .fetch_optional(&self.pool)
                .await?;
        let (tenant, scope_names) = found.ok_or(Error::Unauthenticated)?;
        let scopes =
            Scope::parse_all(&scope_names).map_err(|error| Error::InvalidStoredScopes(error.to_string()))?;
        Ok(Principal::new(tenant, parsed.id, scopes))
    }

    /// Creates a key for the caller's own tenant. The caller needs `keys:admin`
    /// **and must hold every scope it hands out**: `keys:admin` lets you manage
    /// keys, it does not let you mint a key that can do what you cannot (no scope
    /// implies another, ADR-0007). One missing scope refuses the whole request.
    /// Only the operator path ([`Store::create_api_key_as_operator`]) can issue
    /// scopes nobody holds yet.
    pub async fn create_api_key(&self, by: &Principal, new: &NewApiKey) -> Result<CreatedApiKey> {
        by.require(Scope::KeysAdmin)?;
        let mut wanted = new.scopes.clone();
        wanted.sort();
        wanted.dedup();
        for scope in wanted {
            by.require(scope)?;
        }
        let actor = serde_json::json!({ "type": "api_key", "id": by.key_id().to_string() });
        self.insert_api_key(TenantId(by.tenant()), new, actor).await
    }

    /// Creates a key for any tenant, with no scope check. This is how the
    /// operator's `clearcut-admin` command makes a tenant's first key; the
    /// public API never calls it.
    pub async fn create_api_key_as_operator(
        &self,
        tenant: TenantId,
        new: &NewApiKey,
    ) -> Result<CreatedApiKey> {
        self.insert_api_key(tenant, new, serde_json::json!({ "type": "operator" }))
            .await
    }

    async fn insert_api_key(
        &self,
        tenant: TenantId,
        new: &NewApiKey,
        actor: serde_json::Value,
    ) -> Result<CreatedApiKey> {
        if new.name.trim().is_empty() {
            return Err(Error::BlankName);
        }
        if new.scopes.is_empty() {
            return Err(Error::NoScopes);
        }
        let mut scopes = new.scopes.clone();
        scopes.sort();
        scopes.dedup();
        let scope_names: Vec<&str> = scopes.iter().map(|scope| scope.as_str()).collect();

        let key = keys::generate().map_err(|error| Error::Random(error.to_string()))?;
        let days = new
            .lifetime_days
            .map_or(DEFAULT_KEY_LIFETIME_DAYS, NonZeroU32::get)
            .min(DEFAULT_KEY_LIFETIME_DAYS);

        let mut tx = self.begin(tenant).await?;
        let expires_at: OffsetDateTime = sqlx::query_scalar(
            "insert into api_keys (id, tenant_id, name, secret_hash, scopes, expires_at) \
             values ($1, $2, $3, $4, $5, now() + make_interval(days => $6)) \
             returning expires_at",
        )
        .bind(key.id)
        .bind(tenant.0)
        .bind(new.name.trim())
        .bind(&key.secret_hash[..])
        .bind(&scope_names)
        .bind(i32::try_from(days).unwrap_or(i32::MAX))
        .fetch_one(&mut *tx)
        .await?;
        let entry = AuditEntry {
            actor,
            action: "api_key.created".into(),
            subject_type: "api_key".into(),
            subject_id: key.id.to_string(),
            // The secret is never recorded, only what the key is for.
            detail: serde_json::json!({ "name": new.name.trim(), "scopes": scope_names }),
        };
        insert_audit(&mut tx, tenant, &entry).await?;
        tx.commit().await?;
        Ok(CreatedApiKey {
            id: key.id,
            token: key.token,
            expires_at,
        })
    }

    /// Revokes a key of the caller's own tenant, at once. Revoking a key that
    /// is already revoked is fine; one that does not exist (or belongs to
    /// another tenant) is [`Error::ApiKeyNotFound`]. Needs `keys:admin`.
    pub async fn revoke_api_key(&self, by: &Principal, key_id: Uuid) -> Result<()> {
        by.require(Scope::KeysAdmin)?;
        let actor = serde_json::json!({ "type": "api_key", "id": by.key_id().to_string() });
        self.revoke_inner(TenantId(by.tenant()), key_id, actor).await
    }

    /// Revokes any tenant's key with no scope check. This is the operator's
    /// response to a leaked key (`clearcut-admin revoke-key`), usable without
    /// holding an admin key; the public API never calls it. Same semantics as
    /// [`Store::revoke_api_key`], audited as the operator.
    pub async fn revoke_api_key_as_operator(&self, tenant: TenantId, key_id: Uuid) -> Result<()> {
        self.revoke_inner(tenant, key_id, serde_json::json!({ "type": "operator" }))
            .await
    }

    async fn revoke_inner(&self, tenant: TenantId, key_id: Uuid, actor: serde_json::Value) -> Result<()> {
        let mut tx = self.begin(tenant).await?;
        let revoked = sqlx::query(
            "update api_keys set revoked_at = now() \
             where id = $1 and tenant_id = $2 and revoked_at is null",
        )
        .bind(key_id)
        .bind(tenant.0)
        .execute(&mut *tx)
        .await?;
        if revoked.rows_affected() == 1 {
            let entry = AuditEntry {
                actor,
                action: "api_key.revoked".into(),
                subject_type: "api_key".into(),
                subject_id: key_id.to_string(),
                detail: serde_json::json!({}),
            };
            insert_audit(&mut tx, tenant, &entry).await?;
        } else {
            let exists: Option<i32> =
                sqlx::query_scalar("select 1 from api_keys where id = $1 and tenant_id = $2")
                    .bind(key_id)
                    .bind(tenant.0)
                    .fetch_optional(&mut *tx)
                    .await?;
            if exists.is_none() {
                return Err(Error::ApiKeyNotFound);
            }
        }
        tx.commit().await?;
        Ok(())
    }

    /// The caller's tenant's keys, oldest first, without any secrets. Needs
    /// `keys:admin`.
    pub async fn list_api_keys(&self, by: &Principal) -> Result<Vec<ApiKeyInfo>> {
        by.require(Scope::KeysAdmin)?;
        let mut tx = self.begin(TenantId(by.tenant())).await?;
        let rows: Vec<ApiKeyRow> = sqlx::query_as(
            "select id, name, scopes, created_at, expires_at, last_used_at, revoked_at \
             from api_keys order by created_at, id",
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(
                |(id, name, scope_names, created_at, expires_at, last_used_at, revoked_at)| {
                    Ok(ApiKeyInfo {
                        id,
                        name,
                        scopes: Scope::parse_all(&scope_names)
                            .map_err(|error| Error::InvalidStoredScopes(error.to_string()))?,
                        created_at,
                        expires_at,
                        last_used_at,
                        revoked_at,
                    })
                },
            )
            .collect()
    }
}

/// Writes one audit row inside an existing tenant-scoped transaction.
async fn insert_audit(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    tenant: TenantId,
    entry: &AuditEntry,
) -> Result<()> {
    sqlx::query(
        "insert into audit_log (tenant_id, actor, action, subject_type, subject_id, detail) \
         values ($1, $2, $3, $4, $5, $6)",
    )
    .bind(tenant.0)
    .bind(&entry.actor)
    .bind(&entry.action)
    .bind(&entry.subject_type)
    .bind(&entry.subject_id)
    .bind(&entry.detail)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Fails with [`Error::DocumentNotFound`] unless `document` exists and belongs
/// to `tenant`. This is what makes another tenant's document look absent.
async fn require_document<'e, E>(executor: E, tenant: TenantId, document: DocId) -> Result<()>
where
    E: sqlx::PgExecutor<'e>,
{
    let found: Option<i32> = sqlx::query_scalar("select 1 from documents where tenant_id = $1 and id = $2")
        .bind(tenant.0)
        .bind(document.0)
        .fetch_optional(executor)
        .await?;
    found.map(|_| ()).ok_or(Error::DocumentNotFound)
}

#[cfg(test)]
mod tests;
