// Written by Kone Tshivhinda & Claude
use crate::status::InvoiceStatus;

/// Crate-wide result type.
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("currency mismatch: expected {expected}, got {got}")]
    CurrencyMismatch {
        expected: &'static str,
        got: &'static str,
    },
    #[error("arithmetic overflow")]
    Overflow,
    #[error("invalid transition from {from:?} to {to:?}")]
    InvalidTransition { from: InvoiceStatus, to: InvoiceStatus },
    #[error("invoice has no line items")]
    EmptyInvoice,
    #[error("quantity must be positive")]
    InvalidQuantity,
    #[error("invalid event: {0}")]
    InvalidEvent(&'static str),
    #[error("invalid narration: {0}")]
    InvalidNarration(&'static str),
    #[error("unknown scope {0:?}")]
    UnknownScope(String),
    #[error("missing required scope {}", .0.as_str())]
    MissingScope(crate::auth::Scope),
    #[error("unknown job kind {0:?}")]
    UnknownJobKind(String),
    #[error("unknown job state {0:?}")]
    UnknownJobState(String),
    #[error("invalid job transition from {from:?} to {to:?}")]
    InvalidJobTransition {
        from: crate::jobs::JobState,
        to: crate::jobs::JobState,
    },
    #[error("invalid job progress: {0}")]
    InvalidJobProgress(&'static str),
    #[error("invalid job policy: {0}")]
    InvalidJobPolicy(&'static str),
    #[error("invalid page read: {0}")]
    InvalidPageRead(&'static str),
    #[error("invalid router policy: {0}")]
    InvalidRouterPolicy(&'static str),
}
