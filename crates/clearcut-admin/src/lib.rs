// Written by Kone Tshivhinda & Claude
//! `clearcut-admin`: the operator's command (ADR-0007).
//!
//! The public API cannot bootstrap itself (a tenant needs its first key before
//! anyone can call it), and some operator work spans tenants (revoking a leaked
//! key, support). Both happen here, with the **owner** database connection,
//! never through the public API and never with the restricted service role
//! (which is refused with a clear message).
//!
//! Conventions that make it safe to script:
//! - **stdout carries only the machine-readable result**: a tenant id, or a new
//!   key's token, on a line of its own, so `clearcut-admin create-key ... >
//!   key.txt` captures exactly the secret and nothing else;
//! - **stderr carries the human notes**, and never contains a secret;
//! - the exit code is non-zero on any failure, and a failed command prints
//!   nothing on stdout.
//!
//! The logic lives here, apart from `main`, so it can be tested against a real
//! database.

use clap::{Parser, Subcommand};
use clearcut_core::auth::Scope;
use clearcut_store::{AuditEntry, NewApiKey, Store, TenantId};
use std::io::Write;
use std::num::NonZeroU32;
use uuid::Uuid;

/// The operator command.
#[derive(Debug, Parser)]
#[command(
    name = "clearcut-admin",
    version,
    about = "ClearCut operator command. Needs DATABASE_URL set to the OWNER database connection."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Apply any database migrations that have not been applied (safe to repeat).
    Migrate,
    /// Create a tenant and print its id.
    CreateTenant {
        /// The tenant's name.
        name: String,
    },
    /// Create an API key for a tenant and print it ONCE. Use this for a tenant's
    /// first key; after that its administrators can manage keys themselves.
    CreateKey {
        #[arg(long)]
        tenant: Uuid,
        /// A label for the key, such as "ci uploader".
        #[arg(long)]
        name: String,
        /// Comma-separated scopes, e.g. documents:write,events:read.
        #[arg(long, value_delimiter = ',', required = true, value_parser = parse_scope)]
        scopes: Vec<Scope>,
        /// Days until it expires (1 to 365; the default is 365).
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=365))]
        days: Option<u32>,
    },
    /// Make a master key for sealing each tenant's own Anthropic key (ADR-0019) and print it once.
    /// Needs no database. Put it in `.env` as CLEARCUT_MASTER_KEY, and keep a copy somewhere safe:
    /// without it every stored tenant key is unreadable.
    GenerateMasterKey,
    /// Revoke a tenant's key at once (for example one that has leaked).
    RevokeKey {
        #[arg(long)]
        tenant: Uuid,
        #[arg(long)]
        key: Uuid,
    },
}

/// Reads one scope name exactly, and says what the valid ones are if it is wrong.
fn parse_scope(text: &str) -> Result<Scope, String> {
    Scope::parse(text).map_err(|_| {
        let valid: Vec<&str> = Scope::ALL.iter().map(|scope| scope.as_str()).collect();
        format!("unknown scope {text:?}; valid scopes: {}", valid.join(", "))
    })
}

/// Why a command failed, in words an operator can act on.
#[derive(Debug, thiserror::Error)]
pub enum AdminError {
    #[error("a name is required")]
    BlankName,
    #[error("no such tenant")]
    TenantNotFound,
    #[error("no such key for that tenant")]
    KeyNotFound,
    #[error(
        "this command needs the owner database connection, not the service role; \
         check that DATABASE_URL is the owner's"
    )]
    NeedsOwnerConnection,
    #[error("the computer could not provide random bytes")]
    NoRandomness,
    #[error(transparent)]
    Store(#[from] clearcut_store::Error),
    #[error("could not write output: {0}")]
    Io(#[from] std::io::Error),
}

/// Postgres says "insufficient privilege" with this code.
const INSUFFICIENT_PRIVILEGE: &str = "42501";

/// Turns a store error into an operator-facing one where we can say more.
fn explain(error: clearcut_store::Error) -> AdminError {
    use clearcut_store::Error as StoreError;
    match &error {
        StoreError::BlankName => AdminError::BlankName,
        StoreError::ApiKeyNotFound => AdminError::KeyNotFound,
        StoreError::Database(sqlx::Error::Database(db)) => {
            if db.code().as_deref() == Some(INSUFFICIENT_PRIVILEGE) {
                AdminError::NeedsOwnerConnection
            } else if db.is_foreign_key_violation() {
                AdminError::TenantNotFound
            } else {
                AdminError::Store(error)
            }
        }
        _ => AdminError::Store(error),
    }
}

/// Writes a fresh master key (32 random bytes, base64) to `out` alone, with the notes to `err`.
pub fn generate_master_key(out: &mut impl Write, err: &mut impl Write) -> Result<(), AdminError> {
    use base64::Engine;
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| AdminError::NoRandomness)?;
    writeln!(out, "{}", base64::engine::general_purpose::STANDARD.encode(bytes))?;
    writeln!(
        err,
        "This is your master key. Put it in .env as CLEARCUT_MASTER_KEY=<the line above>.
         It is shown only now. Keep a copy somewhere safe: without it every tenant's stored Anthropic
         key is unreadable and has to be entered again. Anyone holding it AND the database can read them."
    )?;
    Ok(())
}

/// Runs one command. `out` gets only the machine-readable result; `err` gets
/// the human notes. Nothing is written to `out` unless the command succeeded.
pub async fn run(
    command: Command,
    store: &Store,
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<(), AdminError> {
    match command {
        Command::Migrate => {
            Store::migrate(store.pool()).await.map_err(explain)?;
            writeln!(err, "Migrations are up to date.")?;
        }
        Command::CreateTenant { name } => {
            let name = name.trim();
            if name.is_empty() {
                return Err(AdminError::BlankName);
            }
            let tenant = store.create_tenant(name).await.map_err(explain)?;
            let entry = AuditEntry {
                actor: serde_json::json!({ "type": "operator" }),
                action: "tenant.created".into(),
                subject_type: "tenant".into(),
                subject_id: tenant.0.to_string(),
                detail: serde_json::json!({ "name": name }),
            };
            store.record_audit(tenant, &entry).await.map_err(explain)?;
            writeln!(out, "{}", tenant.0)?;
            writeln!(err, "Created tenant {} ({name}).", tenant.0)?;
        }
        Command::CreateKey {
            tenant,
            name,
            scopes,
            days,
        } => {
            let request = NewApiKey {
                name,
                scopes,
                lifetime_days: days.and_then(NonZeroU32::new),
            };
            let created = store
                .create_api_key_as_operator(TenantId(tenant), &request)
                .await
                .map_err(explain)?;
            let mut scope_names: Vec<&str> = request.scopes.iter().map(|scope| scope.as_str()).collect();
            scope_names.sort_unstable();
            scope_names.dedup();
            let expires = created
                .expires_at
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_else(|_| created.expires_at.to_string());
            // The token goes to stdout alone; the notes never include it.
            writeln!(out, "{}", created.token.expose())?;
            writeln!(
                err,
                "Created API key {} for tenant {tenant}.\n  name:    {}\n  scopes:  {}\n  expires: {expires}\n\
                 This is the only time the key is shown. Store it somewhere safe.",
                created.id,
                request.name.trim(),
                scope_names.join(", "),
            )?;
        }
        Command::GenerateMasterKey => generate_master_key(out, err)?,
        Command::RevokeKey { tenant, key } => {
            store
                .revoke_api_key_as_operator(TenantId(tenant), key)
                .await
                .map_err(explain)?;
            writeln!(err, "Revoked key {key} of tenant {tenant}.")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
