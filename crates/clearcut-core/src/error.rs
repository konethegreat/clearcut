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
}
