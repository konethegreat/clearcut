// Written by Kone Tshivhinda & Claude
//! # clearcut-anthropic
//!
//! ClearCut's side of the Anthropic API, built around one rule: **the key is the
//! user's own** (ADR-0011). This crate holds the key as a protected secret,
//! loads it from the environment or a file, refuses it on a command line, and
//! talks to `api.anthropic.com` through a deliberately narrow HTTPS client. The
//! first thing it does with a key is check it, for free, against the Models API.

pub mod config;
pub mod failure;
pub mod http;
pub mod key;
pub mod models;
pub mod provider;
pub mod request;
pub mod response;
pub mod schema;
pub mod source;

#[cfg(test)]
mod testutil;

pub use config::{AnthropicConfig, ConfigError, ANTHROPIC_VERSION};
pub use http::{AnthropicHttp, Reply, TransportError};
pub use key::{ApiKey, KeyError};
pub use models::{check_key, check_key_with, KeyCheck, KeyCheckError, KeyCheckKind, Lifecycle, ModelInfo};
pub use source::{load, load_from, reject_key_in_arguments, ArgsError, LoadError, KEY_ENV, KEY_FILE_ENV};
