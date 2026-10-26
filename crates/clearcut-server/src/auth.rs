// Written by Kone Tshivhinda & Claude
//! Authentication for the HTTP API (ADR-0007).
//!
//! A handler that takes an [`Authed`] argument can only run for a caller who
//! presented a valid API key (`Authorization: Bearer ccut_...`); it then asks
//! `auth.require(Scope::...)` for the scope it needs. Rules:
//!
//! - **Every authentication failure looks identical**: missing, malformed or
//!   wrong credentials, an unknown, revoked or expired key all give the same
//!   `401` with `WWW-Authenticate: Bearer` and the same body, so a caller
//!   learns nothing about which keys exist.
//! - **An outage is not a 401.** If the database cannot be reached the answer
//!   is a generic `500`, never "your key is wrong", and never any detail.
//! - **Authentication comes first.** `Authed` is extracted before the request
//!   body, so an anonymous caller cannot probe how the API parses input.
//! - **A scoped route answers 403 only to an authenticated caller.** An
//!   anonymous caller gets 401, which does not reveal what the route needs.
//! - **The key is never logged**, and the header is marked sensitive so even a
//!   `Debug` print of the request headers shows it redacted.

use crate::documents::Ingest;
use crate::jobs::JobQueue;
use crate::results::Results;
use axum::extract::FromRequestParts;
use axum::http::{header, request::Parts, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use clearcut_core::auth::{Principal, Scope};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Why a key could not be turned into a [`Principal`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthFailure {
    /// The key is not valid (for any reason; the caller is not told which).
    Rejected,
    /// The check itself could not be made, for example the database is down.
    Unavailable,
}

/// Something that can turn a presented API key into a [`Principal`]. The real
/// one is the store; tests use a fake.
pub trait Authenticator: Send + Sync + 'static {
    fn authenticate<'a>(
        &'a self,
        presented: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Principal, AuthFailure>> + Send + 'a>>;
}

impl Authenticator for clearcut_store::Store {
    fn authenticate<'a>(
        &'a self,
        presented: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Principal, AuthFailure>> + Send + 'a>> {
        Box::pin(async move {
            match clearcut_store::Store::authenticate(self, presented).await {
                Ok(principal) => Ok(principal),
                Err(clearcut_store::Error::Unauthenticated) => Err(AuthFailure::Rejected),
                Err(error) => {
                    // The key is not part of this error. The detail stays in the log.
                    tracing::error!(%error, "the authentication check failed");
                    Err(AuthFailure::Unavailable)
                }
            }
        })
    }
}

/// Shared state for the routes.
#[derive(Clone)]
pub struct AppState {
    auth: Arc<dyn Authenticator>,
    ingest: Option<Arc<dyn Ingest>>,
    jobs: Option<Arc<dyn JobQueue>>,
    results: Option<Arc<dyn Results>>,
}

impl AppState {
    pub fn new(auth: impl Authenticator) -> Self {
        Self {
            auth: Arc::new(auth),
            ingest: None,
            jobs: None,
            results: None,
        }
    }

    pub fn shared(auth: Arc<dyn Authenticator>) -> Self {
        Self {
            auth,
            ingest: None,
            jobs: None,
            results: None,
        }
    }

    /// Turns on `POST /v1/documents`. Without it that route answers 503.
    pub fn with_ingest(mut self, ingest: impl Ingest) -> Self {
        self.ingest = Some(Arc::new(ingest));
        self
    }

    /// Turns on the job endpoints. Without it they answer 503.
    pub fn with_jobs(mut self, jobs: impl JobQueue) -> Self {
        self.jobs = Some(Arc::new(jobs));
        self
    }

    /// Turns on reading a document back (status, events, report, export). Without it they answer 503.
    pub fn with_results(mut self, results: impl Results) -> Self {
        self.results = Some(Arc::new(results));
        self
    }

    pub(crate) fn results(&self) -> Option<&dyn Results> {
        self.results.as_deref()
    }

    pub(crate) fn jobs(&self) -> Option<&dyn JobQueue> {
        self.jobs.as_deref()
    }

    pub(crate) fn ingest(&self) -> Option<&dyn Ingest> {
        self.ingest.as_deref()
    }
}

/// What an API handler answers when it cannot go ahead.
#[derive(Debug, PartialEq, Eq)]
pub enum ApiProblem {
    /// No valid credentials. Always the same response.
    Unauthorized,
    /// Authenticated, but the key lacks this scope.
    Forbidden(Scope),
    /// Something went wrong on our side. Nothing more is said.
    Internal,
}

impl IntoResponse for ApiProblem {
    fn into_response(self) -> Response {
        match self {
            ApiProblem::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                [(header::WWW_AUTHENTICATE, "Bearer")],
                Json(serde_json::json!({ "error": "authentication failed" })),
            )
                .into_response(),
            ApiProblem::Forbidden(scope) => (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({ "error": "forbidden", "required_scope": scope.as_str() })),
            )
                .into_response(),
            ApiProblem::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "internal error" })),
            )
                .into_response(),
        }
    }
}

/// A caller who has presented a valid API key.
pub struct Authed(pub Principal);

impl Authed {
    /// Passes if the caller's key holds `scope`; otherwise a 403 naming it.
    pub fn require(&self, scope: Scope) -> Result<&Principal, ApiProblem> {
        self.0
            .require(scope)
            .map(|()| &self.0)
            .map_err(|_| ApiProblem::Forbidden(scope))
    }

    pub fn principal(&self) -> &Principal {
        &self.0
    }
}

impl FromRequestParts<AppState> for Authed {
    type Rejection = ApiProblem;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiProblem> {
        let Some(token) = bearer_token(&parts.headers) else {
            tracing::warn!(
                reason = "missing or malformed credentials",
                "authentication failed"
            );
            return Err(ApiProblem::Unauthorized);
        };
        match state.auth.authenticate(token).await {
            Ok(principal) => Ok(Authed(principal)),
            Err(AuthFailure::Rejected) => {
                tracing::warn!(reason = "credentials rejected", "authentication failed");
                Err(ApiProblem::Unauthorized)
            }
            Err(AuthFailure::Unavailable) => {
                tracing::error!("authentication unavailable");
                Err(ApiProblem::Internal)
            }
        }
    }
}

/// The token from `Authorization: Bearer <token>`, or `None` if the header is
/// absent, repeated, not plain text, not the Bearer scheme, or the token is
/// missing or contains whitespace. The scheme name is case-insensitive (RFC
/// 7235); nothing else is lenient.
fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let value = values.next()?;
    if values.next().is_some() {
        return None;
    }
    let (scheme, token) = value.to_str().ok()?.split_once(' ')?;
    let well_formed = scheme.eq_ignore_ascii_case("bearer")
        && !token.is_empty()
        && !token.bytes().any(|byte| byte.is_ascii_whitespace());
    well_formed.then_some(token)
}
