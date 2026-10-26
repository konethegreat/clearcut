// Written by Kone Tshivhinda & Claude
use super::*;

/// A made-up key with the real shape: `sk-ant-api03-` and then URL-safe characters.
fn sample(body: &str) -> String {
    format!("sk-ant-api03-{body}")
}

const BODY: &str = "AbCdEfGhIjKlMnOpQrStUvWxYz0123456789_-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789_-wxyz";

#[test]
fn a_well_formed_key_is_accepted() {
    let key = ApiKey::parse(&sample(BODY)).unwrap();
    assert_eq!(key.expose(), sample(BODY));
}

#[test]
fn whitespace_around_a_key_is_trimmed_but_not_inside_it() {
    for padded in [
        format!("{}\n", sample(BODY)),
        format!("{}\r\n", sample(BODY)),
        format!("  {}  ", sample(BODY)),
        format!("\t{}\n\n", sample(BODY)),
    ] {
        assert_eq!(ApiKey::parse(&padded).unwrap().expose(), sample(BODY));
    }
    for broken in [
        sample("AbCd EfGh0123456789012345678901234567890123"),
        sample("AbCd\tEfGh0123456789012345678901234567890123"),
        sample("AbCd\nEfGh0123456789012345678901234567890123"),
    ] {
        assert_eq!(
            ApiKey::parse(&broken).unwrap_err(),
            KeyError::Whitespace,
            "{broken:?}"
        );
    }
}

#[test]
fn an_empty_key_is_refused() {
    for empty in ["", " ", "\n", " \r\n\t "] {
        assert_eq!(ApiKey::parse(empty).unwrap_err(), KeyError::Empty, "{empty:?}");
    }
}

#[test]
fn only_url_safe_characters_follow_the_prefix() {
    for bad in [
        "é", "/", "+", "=", "$", "%", "\"", "'", "<", "\u{0}", "\u{202e}", "🙂", ".",
    ] {
        let key = sample(&format!("{}{bad}{}", &BODY[..40], &BODY[40..]));
        assert_eq!(
            ApiKey::parse(&key).unwrap_err(),
            KeyError::BadCharacters,
            "{bad:?}"
        );
    }
}

#[test]
fn keys_of_the_wrong_kind_are_named_as_such() {
    let body = "0".repeat(60);
    assert_eq!(
        ApiKey::parse(&format!("sk-ant-admin01-{body}")).unwrap_err(),
        KeyError::AdminKey
    );
    assert_eq!(
        ApiKey::parse(&format!("sk-ant-oat01-{body}")).unwrap_err(),
        KeyError::LoginToken
    );
    for other in [
        "sk-proj-",
        "sk-",
        "pk-ant-api03-",
        "sk-ant-",
        "sk-ant-apple-",
        "sk-ant-ap",
        "xoxb-",
        "ghp_",
        "SK-ANT-API03-",
    ] {
        let error = ApiKey::parse(&format!("{other}{body}")).unwrap_err();
        assert!(matches!(error, KeyError::NotAnApiKey), "{other}: {error:?}");
    }
}

#[test]
fn a_personal_key_is_accepted_and_only_with_its_dash() {
    let body = "a".repeat(80);
    assert!(ApiKey::parse(&format!("sk-ant-usr-{body}")).is_ok());
    for other in ["sk-ant-usr", "sk-ant-user-", "sk-ant-usrx-"] {
        assert_eq!(
            ApiKey::parse(&format!("{other}{body}")).unwrap_err(),
            KeyError::NotAnApiKey,
            "{other}"
        );
    }
}

#[test]
fn the_length_must_be_plausible() {
    let at = |length: usize| format!("sk-ant-api03-{}", "a".repeat(length - "sk-ant-api03-".len()));
    assert_eq!(ApiKey::parse(&at(29)).unwrap_err(), KeyError::BadLength);
    assert!(ApiKey::parse(&at(30)).is_ok());
    assert!(ApiKey::parse(&at(300)).is_ok());
    assert_eq!(ApiKey::parse(&at(301)).unwrap_err(), KeyError::BadLength);
    assert_eq!(ApiKey::parse(&at(5_000)).unwrap_err(), KeyError::BadLength);
}

#[test]
fn the_fingerprint_shows_only_the_last_four_characters() {
    let key = ApiKey::parse(&sample(BODY)).unwrap();
    assert_eq!(key.fingerprint(), "…wxyz");
}

#[test]
fn a_key_never_appears_in_debug_output_in_whole_or_in_part() {
    let key = ApiKey::parse(&sample(BODY)).unwrap();
    let shown = format!("{key:?} {:?} {:#?}", Some(&key), vec![key.clone()]);
    assert!(shown.contains("redacted"), "{shown}");
    for fragment in ["sk-ant", "AbCdEf", "0123456789", "wxyz", &BODY[10..30], "api03"] {
        assert!(!shown.contains(fragment), "{fragment} leaked in {shown}");
    }
}

#[test]
fn a_clone_is_the_same_key_and_costs_no_extra_copy_of_the_secret() {
    let key = ApiKey::parse(&sample(BODY)).unwrap();
    let copy = key.clone();
    assert_eq!(copy.expose(), key.expose());
    assert!(std::ptr::eq(copy.expose().as_ptr(), key.expose().as_ptr()));
}

#[test]
fn a_key_does_not_compare_by_content_in_a_way_that_could_leak_timing() {
    // Equality is available (the tests need it) but goes through a constant-time comparison.
    let a = ApiKey::parse(&sample(BODY)).unwrap();
    let b = ApiKey::parse(&sample(BODY)).unwrap();
    let c = ApiKey::parse(&sample(&BODY.replace("wxyz", "wxyx"))).unwrap();
    assert!(a == b);
    assert!(a != c);
}

#[test]
fn errors_say_what_is_wrong_but_never_repeat_the_text() {
    let secret = "SECRETTEXT";
    let inputs = [
        format!("{secret} {secret}"),
        format!("sk-ant-api03-{secret}é"),
        format!("sk-ant-admin01-{secret}"),
        format!("sk-ant-oat01-{secret}"),
        format!("sk-proj-{secret}"),
        format!("sk-ant-api03-{secret}"),
    ];
    for input in inputs {
        let error = ApiKey::parse(&input).unwrap_err();
        let shown = format!("{error} {error:?}");
        assert!(!shown.contains(secret), "{shown}");
    }
}

#[test]
fn every_error_has_a_sentence_a_person_can_act_on() {
    for (error, words) in [
        (KeyError::Empty, "empty"),
        (KeyError::Whitespace, "spaces"),
        (KeyError::BadCharacters, "characters"),
        (KeyError::NotAnApiKey, "sk-ant-api"),
        (KeyError::AdminKey, "admin"),
        (KeyError::LoginToken, "login"),
        (KeyError::BadLength, "length"),
    ] {
        assert!(error.to_string().contains(words), "{error}");
    }
}

#[test]
fn a_key_that_merely_starts_like_another_is_a_different_key() {
    let short = ApiKey::parse(&sample(BODY)).unwrap();
    let long = ApiKey::parse(&sample(&format!("{BODY}x"))).unwrap();
    assert!(short != long);
    assert!(long != short);
}
