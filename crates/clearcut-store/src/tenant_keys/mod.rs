// Written by Kone Tshivhinda & Claude
//! A tenant's own Anthropic key, as the database holds it (ADR-0019): **ciphertext only**. The
//! service seals the key before it gets here (`clearcut_anthropic::vault`) and opens it after it
//! comes back; the store neither knows nor needs the master key.
//!
//! The application role has no privilege on the table. Everything goes through five
//! security-definer functions that act on the current tenant only. The API shows only
//! [`KeyStatus`]; the ciphertext is read back by [`Store::load_sealed_key`], which the job runner
//! uses.

use crate::{Error, Result, Store, TenantId};
use time::OffsetDateTime;
use uuid::Uuid;

/// A key as stored: ciphertext, its nonce, which master key sealed it and the last four characters.
#[derive(Clone, PartialEq, Eq)]
pub struct SealedKey {
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
    pub master_key_id: String,
    pub last4: String,
}

impl std::fmt::Debug for SealedKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "SealedKey({}, {}, [ciphertext])",
            self.master_key_id, self.last4
        )
    }
}

/// What may be shown about a tenant's stored key. Never the ciphertext.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyStatus {
    pub last4: String,
    /// The ClearCut API key that set it (an id, never a secret).
    pub set_by: Option<Uuid>,
    pub updated_at: OffsetDateTime,
    pub checked_at: Option<OffsetDateTime>,
    /// `Some(false)` once Anthropic has rejected it: it is no longer used.
    pub check_ok: Option<bool>,
}

type StatusRow = (
    String,
    Option<Uuid>,
    OffsetDateTime,
    Option<OffsetDateTime>,
    Option<bool>,
);

impl Store {
    /// Stores (or replaces) the tenant's key. `check_ok` is the outcome of the free key check just made.
    pub async fn set_sealed_key(
        &self,
        tenant: TenantId,
        sealed: &SealedKey,
        set_by: Uuid,
        check_ok: bool,
    ) -> Result<()> {
        if sealed.nonce.len() != 12
            || !(17..=512).contains(&sealed.ciphertext.len())
            || sealed.last4.chars().count() != 4
            || sealed.master_key_id.is_empty()
            || sealed.master_key_id.len() > 64
        {
            return Err(Error::InvalidTenantKey(
                "a sealed key is a 12-byte nonce, 17 to 512 bytes, a 4-character tail and a master key id",
            ));
        }
        let mut tx = self.begin(tenant).await?;
        sqlx::query("select set_tenant_anthropic_key($1, $2, $3, $4, $5, $6, $7)")
            .bind(tenant.0)
            .bind(&sealed.nonce)
            .bind(&sealed.ciphertext)
            .bind(&sealed.master_key_id)
            .bind(&sealed.last4)
            .bind(set_by)
            .bind(check_ok)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// The tenant's sealed key, or `None` if there is none or Anthropic has rejected it. This is the
    /// only read of the ciphertext.
    pub async fn load_sealed_key(&self, tenant: TenantId) -> Result<Option<SealedKey>> {
        let mut tx = self.begin(tenant).await?;
        let row: Option<(Vec<u8>, Vec<u8>, String, String)> = sqlx::query_as(
            "select k.out_nonce, k.out_ciphertext, k.out_master_key_id, s.out_last4 \
             from load_tenant_anthropic_key($1) k, tenant_anthropic_key_status($1) s",
        )
        .bind(tenant.0)
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row.map(|(nonce, ciphertext, master_key_id, last4)| SealedKey {
            nonce,
            ciphertext,
            master_key_id,
            last4,
        }))
    }

    /// What may be shown about the tenant's key, or `None` if it has none.
    pub async fn sealed_key_status(&self, tenant: TenantId) -> Result<Option<KeyStatus>> {
        let mut tx = self.begin(tenant).await?;
        let row: Option<StatusRow> = sqlx::query_as(
            "select out_last4, out_set_by, out_updated_at, out_checked_at, out_check_ok \
                 from tenant_anthropic_key_status($1)",
        )
        .bind(tenant.0)
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(
            row.map(|(last4, set_by, updated_at, checked_at, check_ok)| KeyStatus {
                last4,
                set_by,
                updated_at,
                checked_at,
                check_ok,
            }),
        )
    }

    /// Says Anthropic rejected the tenant's key, so the runner stops using it.
    pub async fn mark_sealed_key_rejected(&self, tenant: TenantId) -> Result<()> {
        let mut tx = self.begin(tenant).await?;
        sqlx::query("select mark_tenant_anthropic_key_rejected($1)")
            .bind(tenant.0)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Removes the tenant's key; says whether there was one.
    pub async fn delete_sealed_key(&self, tenant: TenantId) -> Result<bool> {
        let mut tx = self.begin(tenant).await?;
        let removed: bool = sqlx::query_scalar("select delete_tenant_anthropic_key($1)")
            .bind(tenant.0)
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(removed)
    }
}

#[cfg(test)]
mod tests;
