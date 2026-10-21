// Written by Kone Tshivhinda & Claude
//! Authorization (ADR-0007): what an authenticated caller may do.
//!
//! This is a business rule, so it lives in the core. Authentication (turning
//! an API key into a [`Principal`]) is I/O and lives in `clearcut-store`; the
//! REST and MCP layers only ask `principal.require(Scope::DocumentsWrite)`.
//!
//! The model is deliberately small:
//! - A [`Scope`] names one permission. **No scope implies another**: holding
//!   `keys:admin` lets you manage keys, not read invoices.
//! - A [`Principal`] is a tenant, the key that was presented, and the scopes
//!   that key carries. A principal with no scopes can do nothing.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;

/// One permission. The text forms are stored in the database and sent in API
/// calls, so renaming one is a breaking change.
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Scope {
    /// List and read documents and their status.
    #[serde(rename = "documents:read")]
    DocumentsRead,
    /// Upload documents.
    #[serde(rename = "documents:write")]
    DocumentsWrite,
    /// Read events, narration and exports.
    #[serde(rename = "events:read")]
    EventsRead,
    /// Confirm or correct extractions (creates human events).
    #[serde(rename = "review:write")]
    ReviewWrite,
    /// Create and revoke API keys for the tenant.
    #[serde(rename = "keys:admin")]
    KeysAdmin,
}

impl Scope {
    /// Every scope, in a fixed order.
    pub const ALL: [Scope; 5] = [
        Scope::DocumentsRead,
        Scope::DocumentsWrite,
        Scope::EventsRead,
        Scope::ReviewWrite,
        Scope::KeysAdmin,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Scope::DocumentsRead => "documents:read",
            Scope::DocumentsWrite => "documents:write",
            Scope::EventsRead => "events:read",
            Scope::ReviewWrite => "review:write",
            Scope::KeysAdmin => "keys:admin",
        }
    }

    /// Parses exactly one scope name. Matching is exact: no trimming, no
    /// case folding, no wildcards, no comma lists.
    pub fn parse(text: &str) -> Result<Scope> {
        Scope::ALL
            .into_iter()
            .find(|scope| scope.as_str() == text)
            .ok_or_else(|| Error::UnknownScope(text.to_string()))
    }

    /// Parses a list of names, failing on the first unknown one. A stored
    /// list containing an unknown scope is an error to be looked at, not
    /// something to trim quietly.
    pub fn parse_all<I, S>(names: I) -> Result<Vec<Scope>>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        names
            .into_iter()
            .map(|name| Scope::parse(name.as_ref()))
            .collect()
    }
}

/// Who is calling: a tenant, the key they presented, and what that key may do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    tenant: Uuid,
    key_id: Uuid,
    scopes: BTreeSet<Scope>,
}

impl Principal {
    pub fn new(tenant: Uuid, key_id: Uuid, scopes: impl IntoIterator<Item = Scope>) -> Self {
        Self {
            tenant,
            key_id,
            scopes: scopes.into_iter().collect(),
        }
    }

    pub fn tenant(&self) -> Uuid {
        self.tenant
    }

    pub fn key_id(&self) -> Uuid {
        self.key_id
    }

    pub fn has(&self, scope: Scope) -> bool {
        self.scopes.contains(&scope)
    }

    /// Passes if the principal holds `scope`; otherwise names the missing one.
    pub fn require(&self, scope: Scope) -> Result<()> {
        if self.has(scope) {
            Ok(())
        } else {
            Err(Error::MissingScope(scope))
        }
    }

    /// The scopes held, without duplicates, in a fixed order.
    pub fn scopes(&self) -> Vec<Scope> {
        self.scopes.iter().copied().collect()
    }
}

#[cfg(test)]
mod tests;
