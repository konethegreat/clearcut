// Written by Kone Tshivhinda & Claude
//! # clearcut-core
//!
//! Pure, I/O-free invoicing domain logic: money, tax, invoices, lifecycle and
//! South African VAT compliance. Everything else (REST API, MCP host, SDKs)
//! is a thin layer over this crate.

pub mod assemble;
pub mod auth;
pub mod compliance_za;
pub mod cost;
pub mod decimal;
pub mod document;
pub mod error;
pub mod escalation;
pub mod event;
pub mod extraction;
pub mod invoice;
pub mod jobs;
pub mod money;
pub mod narration;
pub mod page_read;
pub mod reading;
pub mod redact;
pub mod status;
pub mod tax;
pub mod text_check;
pub mod text_quality;
pub mod untrusted;

pub use error::{Error, Result};
pub use invoice::{Invoice, InvoiceTotals, LineItem};
pub use money::{Currency, Money};
pub use status::InvoiceStatus;
pub use tax::TaxCategory;
