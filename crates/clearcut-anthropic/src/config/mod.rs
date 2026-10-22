// Written by Kone Tshivhinda & Claude
//! Where and how ClearCut talks to Anthropic. This is **operator
//! configuration, never tenant input** (ADR-0011): if a user of a hosted
//! ClearCut could choose the address, they could point their (or someone
//! else's) key at a server they control.
//!
//! So the address is strict: `https` (plain `http` only to this computer, for
//! tests and local gateways), no user name or password, no path, query or
//! fragment. Errors never repeat the value they refused.

use std::ffi::OsString;
use std::time::Duration;
use url::{Host, Url};

/// The address of the Claude API.
pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
/// The API version header every request carries (Anthropic keeps old versions working).
pub const ANTHROPIC_VERSION: &str = "2023-06-01";
/// How long one request may take before the client gives up.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// An operator may point ClearCut at a different address (a local gateway).
pub const BASE_URL_ENV: &str = "CLEARCUT_ANTHROPIC_BASE_URL";
/// A key that is not tied to one workspace must say which to use.
pub const WORKSPACE_ENV: &str = "ANTHROPIC_WORKSPACE_ID";

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    #[error("the Anthropic address is not a valid URL")]
    BadUrl,
    #[error("the Anthropic address must start with https:// (http:// is allowed only to this computer)")]
    InsecureAddress,
    #[error("the Anthropic address must not contain a user name or password")]
    HasCredentials,
    #[error("the Anthropic address must be just a host, with no path, query or fragment")]
    HasPathQueryOrFragment,
    #[error("the workspace id must look like wrkspc_ followed by letters and digits")]
    BadWorkspaceId,
    #[error("the timeout must be more than zero")]
    BadTimeout,
    #[error("the HTTP client could not be built")]
    Client,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnthropicConfig {
    base_url: Url,
    workspace_id: Option<String>,
    timeout: Duration,
}

impl AnthropicConfig {
    /// Anthropic's own address, no workspace, the default timeout.
    pub fn production() -> AnthropicConfig {
        AnthropicConfig::new(DEFAULT_BASE_URL).expect("the built-in address is valid")
    }

    pub fn new(base_url: &str) -> Result<AnthropicConfig, ConfigError> {
        let url = Url::parse(base_url).map_err(|_| ConfigError::BadUrl)?;
        let host = url.host().ok_or(ConfigError::BadUrl)?;
        match url.scheme() {
            "https" => {}
            "http" if is_this_computer(&host) => {}
            "http" => return Err(ConfigError::InsecureAddress),
            _ => return Err(ConfigError::BadUrl),
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(ConfigError::HasCredentials);
        }
        if !matches!(url.path(), "" | "/") || url.query().is_some() || url.fragment().is_some() {
            return Err(ConfigError::HasPathQueryOrFragment);
        }
        Ok(AnthropicConfig {
            base_url: url,
            workspace_id: None,
            timeout: DEFAULT_TIMEOUT,
        })
    }

    pub fn with_workspace_id(mut self, id: &str) -> Result<AnthropicConfig, ConfigError> {
        let valid = id.strip_prefix("wrkspc_").is_some_and(|rest| {
            (10..=64).contains(&rest.len()) && rest.bytes().all(|b| b.is_ascii_alphanumeric())
        });
        if !valid {
            return Err(ConfigError::BadWorkspaceId);
        }
        self.workspace_id = Some(id.to_string());
        Ok(self)
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Result<AnthropicConfig, ConfigError> {
        if timeout.is_zero() {
            return Err(ConfigError::BadTimeout);
        }
        self.timeout = timeout;
        Ok(self)
    }

    /// Reads `CLEARCUT_ANTHROPIC_BASE_URL` and `ANTHROPIC_WORKSPACE_ID` (an empty
    /// value counts as not set). A bad value is an error, never a quiet default.
    pub fn from_env(env: impl Fn(&str) -> Option<OsString>) -> Result<AnthropicConfig, ConfigError> {
        let set = |name: &str| -> Result<Option<String>, ConfigError> {
            match env(name).filter(|value| !value.is_empty()) {
                None => Ok(None),
                Some(value) => value.into_string().map(Some).map_err(|_| ConfigError::BadUrl),
            }
        };
        let mut config = match set(BASE_URL_ENV)? {
            Some(url) => AnthropicConfig::new(&url)?,
            None => AnthropicConfig::production(),
        };
        if let Some(id) = set(WORKSPACE_ENV)? {
            config = config.with_workspace_id(&id)?;
        }
        Ok(config)
    }

    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    pub fn workspace_id(&self) -> Option<&str> {
        self.workspace_id.as_deref()
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }
}

/// `localhost` or a loopback address (`127.x.x.x`, `::1`). A name such as
/// `127.0.0.1.example.com` is a different host and does not count.
fn is_this_computer(host: &Host<&str>) -> bool {
    match host {
        Host::Domain(name) => name.eq_ignore_ascii_case("localhost"),
        Host::Ipv4(ip) => ip.is_loopback(),
        Host::Ipv6(ip) => ip.is_loopback(),
    }
}

#[cfg(test)]
mod tests;
