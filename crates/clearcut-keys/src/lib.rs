// Written by Kone Tshivhinda & Claude
//! A tenant's own Anthropic key (ADR-0019, ADR-0011 mode B): the rules for setting, checking,
//! sealing, using and removing it. Transport-independent, so the REST endpoints, the MCP tools and
//! the job runner cannot drift apart.
//!
//! - **Setting** needs `keys:admin`. The text must be a regular API key (an admin key or a login
//!   token is refused), Anthropic must accept it (the free `GET /v1/models` check, no tokens
//!   spent), and a master key must be configured. Only then is it sealed (AES-256-GCM, the tenant
//!   bound in) and stored, and the action audited without the value.
//! - **Using** is [`TenantKeys::resolve`]: the tenant's key if it has one that works; otherwise the
//!   server's own key, **only** in [`KeyMode::Shared`]; otherwise there is no key and the job waits.
//!   The project has no key of its own and never pools users' keys: `TenantOnly` is the
//!   deployment setting that makes sure of it.
//! - **Nothing here ever prints, logs or returns a key.** Errors are fixed sentences.

use clearcut_anthropic::{
    check_key_with, open, seal, AnthropicConfig, ApiKey, KeyCheck, KeyCheckError, KeyError, MasterKey,
    Sealed, VaultError,
};
use clearcut_core::auth::{Principal, Scope};
use clearcut_store::tenant_keys::SealedKey;
use clearcut_store::{AuditEntry, Store, TenantId};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use time::OffsetDateTime;

/// Whether the server's own key may be used for tenants that have none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyMode {
    /// A tenant's key wins; without one the server's key is used (your own computer).
    Shared,
    /// The server's key is never used: every tenant brings its own (a deployment).
    TenantOnly,
}

impl KeyMode {
    /// `CLEARCUT_KEY_MODE`: `shared` (also when unset) or `tenant-only`. Anything else is an error.
    pub fn parse(text: Option<&str>) -> Result<KeyMode, &'static str> {
        match text.map(str::trim) {
            None | Some("") | Some("shared") => Ok(KeyMode::Shared),
            Some("tenant-only") => Ok(KeyMode::TenantOnly),
            Some(_) => Err("CLEARCUT_KEY_MODE must be shared or tenant-only"),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            KeyMode::Shared => "shared",
            KeyMode::TenantOnly => "tenant-only",
        }
    }
}

/// Where the key a job would use comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySource {
    Tenant,
    Server,
    None,
}

impl KeySource {
    pub fn as_str(self) -> &'static str {
        match self {
            KeySource::Tenant => "tenant",
            KeySource::Server => "server",
            KeySource::None => "none",
        }
    }
}

pub type CheckFuture<'a> = Pin<Box<dyn Future<Output = Result<KeyCheck, KeyCheckError>> + Send + 'a>>;

/// Asks Anthropic what a key can do, for free. The real one calls the Models API; tests use a fake.
pub trait KeyChecker: Send + Sync + 'static {
    fn check<'a>(&'a self, key: &'a ApiKey) -> CheckFuture<'a>;
}

/// The real check, against the configured Anthropic address.
pub struct AnthropicChecker(pub AnthropicConfig);

impl KeyChecker for AnthropicChecker {
    fn check<'a>(&'a self, key: &'a ApiKey) -> CheckFuture<'a> {
        Box::pin(check_key_with(&self.0, key))
    }
}

/// Why a key could not be set or removed. Fixed sentences.
#[derive(Debug, thiserror::Error)]
pub enum KeysError {
    #[error("missing required scope keys:admin")]
    Forbidden,
    /// No master key is configured, so tenant keys cannot be stored.
    #[error("tenant keys are not available: the server has no master key configured")]
    NotConfigured,
    #[error("{0}")]
    BadKey(KeyError),
    /// Anthropic rejected the key (or its owner must act: billing, permission, spend cap).
    #[error("Anthropic did not accept this key: {0}")]
    Refused(String),
    /// The check could not be made (Anthropic unreachable, a timeout): nothing was stored.
    #[error("the key could not be checked just now: {0}")]
    CheckUnavailable(String),
    #[error("could not store the key: {0}")]
    Store(#[from] clearcut_store::Error),
    #[error("could not seal the key: {0}")]
    Vault(VaultError),
}

/// What setting a key found out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeySet {
    pub last4: String,
    /// The models the key can call.
    pub models: Vec<String>,
    pub checked_at: OffsetDateTime,
}

/// What may be shown about a tenant's key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyView {
    pub configured: bool,
    pub last4: Option<String>,
    pub set_at: Option<OffsetDateTime>,
    pub checked_at: Option<OffsetDateTime>,
    /// `Some(false)` once Anthropic has rejected the stored key.
    pub check_ok: Option<bool>,
    /// Where the next job's key would come from.
    pub source: KeySource,
    /// Whether this server can store tenant keys at all (a master key is configured).
    pub tenant_keys_enabled: bool,
    pub mode: KeyMode,
}

/// The key a job should use, or why there is none.
pub enum Resolved {
    Key(ApiKey, KeySource),
    /// No key anywhere this tenant may use: the job waits until one is set.
    NoKey,
    /// The tenant's stored key was rejected by Anthropic, or cannot be opened: it must be set again.
    Rejected,
}

/// The tenant keys of one running server.
#[derive(Clone)]
pub struct TenantKeys {
    store: Store,
    master: Option<Arc<MasterKey>>,
    mode: KeyMode,
    server_key: Option<ApiKey>,
    checker: Arc<dyn KeyChecker>,
}

impl TenantKeys {
    pub fn new(
        store: Store,
        master: Option<MasterKey>,
        mode: KeyMode,
        server_key: Option<ApiKey>,
        checker: impl KeyChecker,
    ) -> Self {
        Self {
            store,
            master: master.map(Arc::new),
            mode,
            server_key,
            checker: Arc::new(checker),
        }
    }

    pub fn mode(&self) -> KeyMode {
        self.mode
    }

    /// Where the key for a tenant with no key of its own would come from.
    fn fallback(&self) -> KeySource {
        match (self.mode, &self.server_key) {
            (KeyMode::Shared, Some(_)) => KeySource::Server,
            _ => KeySource::None,
        }
    }

    /// What may be shown about the tenant's key.
    pub async fn status(&self, tenant: TenantId) -> Result<KeyView, KeysError> {
        let stored = if self.master.is_some() {
            self.store.sealed_key_status(tenant).await?
        } else {
            None
        };
        let source = match &stored {
            Some(status) if status.check_ok != Some(false) => KeySource::Tenant,
            _ => self.fallback(),
        };
        Ok(KeyView {
            configured: stored.is_some(),
            last4: stored.as_ref().map(|s| s.last4.clone()),
            set_at: stored.as_ref().map(|s| s.updated_at),
            checked_at: stored.as_ref().and_then(|s| s.checked_at),
            check_ok: stored.as_ref().and_then(|s| s.check_ok),
            source,
            tenant_keys_enabled: self.master.is_some(),
            mode: self.mode,
        })
    }

    /// Checks `text` with Anthropic and, if it works, seals and stores it as the tenant's key.
    pub async fn set(&self, by: &Principal, text: &str) -> Result<KeySet, KeysError> {
        by.require(Scope::KeysAdmin).map_err(|_| KeysError::Forbidden)?;
        let master = self.master.as_ref().ok_or(KeysError::NotConfigured)?;
        let key = ApiKey::parse(text).map_err(KeysError::BadKey)?;
        let check = self.checker.check(&key).await.map_err(|error| {
            let said = error.kind().to_string();
            if error.kind().owner_must_act() {
                KeysError::Refused(said)
            } else {
                KeysError::CheckUnavailable(said)
            }
        })?;
        let tenant = TenantId(by.tenant());
        let sealed = seal(master, by.tenant(), &key).map_err(KeysError::Vault)?;
        self.store
            .set_sealed_key(
                tenant,
                &SealedKey {
                    nonce: sealed.nonce,
                    ciphertext: sealed.ciphertext,
                    master_key_id: sealed.master_key_id,
                    last4: sealed.last4.clone(),
                },
                by.key_id(),
                true,
            )
            .await?;
        self.audit(
            by,
            "anthropic_key.set",
            serde_json::json!({ "last4": sealed.last4 }),
        )
        .await;
        Ok(KeySet {
            last4: sealed.last4,
            models: check
                .models
                .iter()
                .filter(|model| check.can_use(&model.id))
                .map(|model| model.id.clone())
                .collect(),
            checked_at: OffsetDateTime::now_utc(),
        })
    }

    /// Removes the tenant's key; says whether it had one.
    pub async fn remove(&self, by: &Principal) -> Result<bool, KeysError> {
        by.require(Scope::KeysAdmin).map_err(|_| KeysError::Forbidden)?;
        let removed = self.store.delete_sealed_key(TenantId(by.tenant())).await?;
        if removed {
            self.audit(by, "anthropic_key.removed", serde_json::json!({}))
                .await;
        }
        Ok(removed)
    }

    /// The key a job of this tenant should use (for the job runner).
    pub async fn resolve(&self, tenant: TenantId) -> Result<Resolved, KeysError> {
        if let Some(master) = &self.master {
            if let Some(stored) = self.store.load_sealed_key(tenant).await? {
                let opened = open(
                    master,
                    tenant.0,
                    &Sealed {
                        nonce: stored.nonce,
                        ciphertext: stored.ciphertext,
                        master_key_id: stored.master_key_id,
                        last4: stored.last4,
                    },
                );
                return match opened {
                    Ok(key) => Ok(Resolved::Key(key, KeySource::Tenant)),
                    Err(error) => {
                        tracing::error!(%error, "a tenant's stored key could not be opened; it must be set again");
                        Ok(Resolved::Rejected)
                    }
                };
            }
            // A stored key that Anthropic has rejected is not handed out, and is not silently
            // replaced by the server's: the tenant is told to set it again.
            if self
                .store
                .sealed_key_status(tenant)
                .await?
                .is_some_and(|status| status.check_ok == Some(false))
            {
                return Ok(Resolved::Rejected);
            }
        }
        match (self.mode, &self.server_key) {
            (KeyMode::Shared, Some(key)) => Ok(Resolved::Key(key.clone(), KeySource::Server)),
            _ => Ok(Resolved::NoKey),
        }
    }

    /// Anthropic rejected the tenant's key while a job was using it: stop using it, and say so.
    pub async fn mark_rejected(&self, tenant: TenantId) -> Result<(), KeysError> {
        self.store.mark_sealed_key_rejected(tenant).await?;
        let entry = AuditEntry {
            actor: serde_json::json!({ "type": "system" }),
            action: "anthropic_key.rejected".to_string(),
            subject_type: "tenant".to_string(),
            subject_id: tenant.0.to_string(),
            detail: serde_json::json!({}),
        };
        if let Err(error) = self.store.record_audit(tenant, &entry).await {
            tracing::warn!(%error, "could not audit a rejected tenant key");
        }
        Ok(())
    }

    async fn audit(&self, by: &Principal, action: &str, detail: serde_json::Value) {
        let entry = AuditEntry {
            actor: serde_json::json!({ "type": "api_key", "id": by.key_id().to_string() }),
            action: action.to_string(),
            subject_type: "tenant".to_string(),
            subject_id: by.tenant().to_string(),
            detail,
        };
        if let Err(error) = self.store.record_audit(TenantId(by.tenant()), &entry).await {
            tracing::warn!(%error, "could not audit a tenant key change");
        }
    }
}

#[cfg(test)]
mod tests;
