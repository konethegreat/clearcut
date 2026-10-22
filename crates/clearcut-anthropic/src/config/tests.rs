// Written by Kone Tshivhinda & Claude
use super::*;
use std::collections::HashMap;

fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
    let map: HashMap<String, String> = pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect();
    move |name| map.get(name).map(OsString::from)
}

#[test]
fn the_default_is_anthropics_own_address_over_https() {
    let config = AnthropicConfig::production();
    assert_eq!(config.base_url().as_str(), "https://api.anthropic.com/");
    assert_eq!(config.workspace_id(), None);
    assert_eq!(config.timeout(), DEFAULT_TIMEOUT);
    assert_eq!(ANTHROPIC_VERSION, "2023-06-01");
}

#[test]
fn https_addresses_are_accepted_with_or_without_a_trailing_slash() {
    for url in [
        "https://api.anthropic.com",
        "https://api.anthropic.com/",
        "https://gateway.example.com:8443",
    ] {
        assert!(AnthropicConfig::new(url).is_ok(), "{url}");
    }
}

#[test]
fn plain_http_is_allowed_only_to_this_computer() {
    for url in [
        "http://127.0.0.1:8080",
        "http://localhost:9000",
        "http://[::1]:8080",
        "http://127.0.0.2",
    ] {
        assert!(AnthropicConfig::new(url).is_ok(), "{url}");
    }
    for url in [
        "http://api.anthropic.com",
        "http://192.168.1.10",
        "http://10.0.0.1:80",
        "http://example.com",
        "http://127.0.0.1.example.com",
    ] {
        assert_eq!(
            AnthropicConfig::new(url).unwrap_err(),
            ConfigError::InsecureAddress,
            "{url}"
        );
    }
}

#[test]
fn other_schemes_and_junk_are_refused() {
    for url in [
        "ftp://api.anthropic.com",
        "file:///etc/passwd",
        "ws://127.0.0.1",
        "api.anthropic.com",
        "",
        "https://",
        "://x",
        "javascript:alert(1)",
    ] {
        assert!(
            matches!(
                AnthropicConfig::new(url).unwrap_err(),
                ConfigError::BadUrl | ConfigError::InsecureAddress
            ),
            "{url}"
        );
    }
}

#[test]
fn an_address_may_not_carry_credentials_a_path_a_query_or_a_fragment() {
    assert_eq!(
        AnthropicConfig::new("https://user:pass@api.anthropic.com").unwrap_err(),
        ConfigError::HasCredentials
    );
    assert_eq!(
        AnthropicConfig::new("https://user@api.anthropic.com").unwrap_err(),
        ConfigError::HasCredentials
    );
    assert_eq!(
        AnthropicConfig::new("https://api.anthropic.com/v1").unwrap_err(),
        ConfigError::HasPathQueryOrFragment
    );
    assert_eq!(
        AnthropicConfig::new("https://api.anthropic.com?x=1").unwrap_err(),
        ConfigError::HasPathQueryOrFragment
    );
    assert_eq!(
        AnthropicConfig::new("https://api.anthropic.com/#x").unwrap_err(),
        ConfigError::HasPathQueryOrFragment
    );
}

#[test]
fn a_workspace_id_is_wrkspc_and_letters_and_digits() {
    for ok in ["wrkspc_011CZkZaBF1tNoB5wlCeusgy", "wrkspc_0123456789"] {
        let config = AnthropicConfig::production().with_workspace_id(ok).unwrap();
        assert_eq!(config.workspace_id(), Some(ok));
    }
    for bad in [
        "",
        "wrkspc_",
        "wrkspc_short",
        "workspace_011CZkZaBF1tNoB5wlCeusgy",
        "wrkspc_011CZk ZaBF1tNoB5wlCeusgy",
        "wrkspc_011CZkZaBF1tNoB5wlCeusgy\n",
        &format!("wrkspc_{}", "a".repeat(65)),
        "wrkspc_011CZkZaBF1tNoB5wlCeus-y",
    ] {
        assert_eq!(
            AnthropicConfig::production().with_workspace_id(bad).unwrap_err(),
            ConfigError::BadWorkspaceId,
            "{bad:?}"
        );
    }
}

#[test]
fn the_timeout_must_be_more_than_zero() {
    assert_eq!(
        AnthropicConfig::production()
            .with_timeout(Duration::ZERO)
            .unwrap_err(),
        ConfigError::BadTimeout
    );
    let config = AnthropicConfig::production()
        .with_timeout(Duration::from_secs(90))
        .unwrap();
    assert_eq!(config.timeout(), Duration::from_secs(90));
}

#[test]
fn the_environment_can_set_the_address_and_the_workspace() {
    let config = AnthropicConfig::from_env(env(&[
        (BASE_URL_ENV, "http://127.0.0.1:4000"),
        (WORKSPACE_ENV, "wrkspc_011CZkZaBF1tNoB5wlCeusgy"),
    ]))
    .unwrap();
    assert_eq!(config.base_url().as_str(), "http://127.0.0.1:4000/");
    assert_eq!(config.workspace_id(), Some("wrkspc_011CZkZaBF1tNoB5wlCeusgy"));
}

#[test]
fn nothing_set_means_production_and_empty_means_not_set() {
    assert_eq!(
        AnthropicConfig::from_env(env(&[])).unwrap(),
        AnthropicConfig::production()
    );
    assert_eq!(
        AnthropicConfig::from_env(env(&[(BASE_URL_ENV, ""), (WORKSPACE_ENV, "")])).unwrap(),
        AnthropicConfig::production()
    );
}

#[test]
fn a_bad_value_in_the_environment_is_an_error_not_a_silent_default() {
    assert!(AnthropicConfig::from_env(env(&[(BASE_URL_ENV, "http://evil.example")])).is_err());
    assert!(AnthropicConfig::from_env(env(&[(WORKSPACE_ENV, "nope")])).is_err());
}

#[test]
fn errors_never_repeat_the_value() {
    for result in [
        AnthropicConfig::new("http://secret-host.example"),
        AnthropicConfig::new("https://user:SECRETPASS@api.anthropic.com"),
        AnthropicConfig::new("https://api.anthropic.com/SECRETPATH"),
    ] {
        let shown = format!("{}", result.unwrap_err());
        assert!(!shown.contains("secret") && !shown.contains("SECRET"), "{shown}");
    }
    let shown = AnthropicConfig::production()
        .with_workspace_id("SECRETWORKSPACE")
        .unwrap_err()
        .to_string();
    assert!(!shown.contains("SECRET"), "{shown}");
}

#[test]
fn a_password_without_a_user_name_is_still_a_credential() {
    assert_eq!(
        AnthropicConfig::new("https://:pass@api.anthropic.com").unwrap_err(),
        ConfigError::HasCredentials
    );
}

#[test]
fn workspace_ids_have_ten_to_sixty_four_characters_after_the_prefix() {
    let with = |n: usize| format!("wrkspc_{}", "a".repeat(n));
    assert_eq!(
        AnthropicConfig::production()
            .with_workspace_id(&with(9))
            .unwrap_err(),
        ConfigError::BadWorkspaceId
    );
    assert!(AnthropicConfig::production().with_workspace_id(&with(10)).is_ok());
    assert!(AnthropicConfig::production().with_workspace_id(&with(64)).is_ok());
    assert_eq!(
        AnthropicConfig::production()
            .with_workspace_id(&with(65))
            .unwrap_err(),
        ConfigError::BadWorkspaceId
    );
}

#[test]
fn only_exactly_this_computer_may_be_reached_without_https() {
    for url in [
        "http://notlocalhost",
        "http://localhost.example.com",
        "http://evil-localhost",
        "http://[2001:db8::1]",
        "http://[fe80::1]",
        "http://8.8.8.8",
    ] {
        assert_eq!(
            AnthropicConfig::new(url).unwrap_err(),
            ConfigError::InsecureAddress,
            "{url}"
        );
    }
    assert!(AnthropicConfig::new("http://LOCALHOST:80").is_ok());
}

#[test]
fn the_default_timeout_is_sixty_seconds() {
    assert_eq!(DEFAULT_TIMEOUT, Duration::from_secs(60));
    assert_eq!(AnthropicConfig::production().timeout(), Duration::from_secs(60));
}
