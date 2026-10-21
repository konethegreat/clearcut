// Written by Kone Tshivhinda & Claude
use super::*;
use std::collections::HashSet;
use uuid::Uuid;

/// SHA-256 of 32 zero bytes (a standard test vector).
const HASH_OF_ZEROS: &str = "66687aadf862bd776c8fc18b8e9f8e20089714856ee233b3902a591d0d5f2925";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ---------- generating ----------

#[test]
fn a_key_has_the_documented_shape() {
    // ccut_<32 hex key id>_<64 hex secret>
    let key = generate().unwrap();
    let text = key.token.expose();
    assert!(text.starts_with("ccut_"));
    let parts: Vec<&str> = text["ccut_".len()..].split('_').collect();
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].len(), 32);
    assert_eq!(parts[1].len(), 64);
    assert!(text
        .bytes()
        .all(|b| b.is_ascii_digit() || b.is_ascii_lowercase() || b == b'_'));
    assert!(parts[0]
        .bytes()
        .chain(parts[1].bytes())
        .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
    assert_eq!(text.len(), 5 + 32 + 1 + 64);
    assert_eq!(parts[0], key.id.simple().to_string());
}

#[test]
fn every_generated_key_is_different() {
    let ids: HashSet<Uuid> = (0..200).map(|_| generate().unwrap().id).collect();
    assert_eq!(ids.len(), 200);
    let tokens: HashSet<String> = (0..200)
        .map(|_| generate().unwrap().token.expose().to_string())
        .collect();
    assert_eq!(tokens.len(), 200);
    let hashes: HashSet<[u8; 32]> = (0..200).map(|_| generate().unwrap().secret_hash).collect();
    assert_eq!(hashes.len(), 200);
}

#[test]
fn the_key_id_is_a_random_uuid_not_a_counter() {
    let key = generate().unwrap();
    assert_eq!(key.id.get_version_num(), 4);
}

// ---------- hashing ----------

#[test]
fn the_stored_hash_is_the_sha256_of_the_secret_bytes() {
    assert_eq!(hex(&hash_secret(&[0u8; 32])), HASH_OF_ZEROS);
}

#[test]
fn a_generated_key_carries_the_hash_of_its_own_secret() {
    let key = generate().unwrap();
    let parsed = parse(key.token.expose()).unwrap();
    assert_eq!(parsed.secret_hash, key.secret_hash);
    // ...and the hash is not the secret itself.
    let secret_hex = &key.token.expose()[5 + 32 + 1..];
    assert_ne!(hex(&key.secret_hash), secret_hex);
}

// ---------- parsing ----------

#[test]
fn a_generated_key_parses_back_to_its_id_and_hash() {
    let key = generate().unwrap();
    let parsed = parse(key.token.expose()).unwrap();
    assert_eq!(parsed.id, key.id);
    assert_eq!(parsed.secret_hash, key.secret_hash);
}

#[test]
fn a_known_key_parses_to_a_known_hash() {
    let text = format!("ccut_{}_{}", "0".repeat(32), "0".repeat(64));
    let parsed = parse(&text).unwrap();
    assert_eq!(parsed.id, Uuid::nil());
    assert_eq!(hex(&parsed.secret_hash), HASH_OF_ZEROS);
}

#[test]
fn anything_that_is_not_exactly_a_key_is_refused() {
    let id = "a".repeat(32);
    let secret = "b".repeat(64);
    let good = format!("ccut_{id}_{secret}");
    assert!(parse(&good).is_some());
    let bad = [
        String::new(),
        "ccut_".to_string(),
        format!("cc_{id}_{secret}"),
        format!("CCUT_{id}_{secret}"),
        format!("ccut_{id}{secret}"),
        format!("ccut_{id}_{}", &secret[..63]),
        format!("ccut_{}_{secret}", &id[..31]),
        format!("ccut_{id}_{secret}0"),
        format!("ccut_{id}_{secret}_extra"),
        format!("ccut_{}_{secret}", id.to_uppercase()),
        format!("ccut_{id}_{}", secret.to_uppercase()),
        format!("ccut_{id}_{}g", &secret[..63]),
        format!(" {good}"),
        format!("{good} "),
        format!("{good}\n"),
        format!("Bearer {good}"),
        format!("ccut_{id}_{secret}\0"),
        format!("ccut_{id}-{secret}"),
    ];
    for text in bad {
        assert!(parse(&text).is_none(), "should refuse {text:?}");
    }
}

// ---------- not leaking ----------

#[test]
fn debug_output_never_shows_the_secret() {
    let key = generate().unwrap();
    let secret_hex = key.token.expose()[5 + 32 + 1..].to_string();
    for shown in [
        format!("{:?}", key.token),
        format!("{key:?}"),
        format!("{:#?}", key),
    ] {
        assert!(!shown.contains(&secret_hex), "secret leaked in {shown}");
        assert!(!shown.contains(key.token.expose()), "token leaked in {shown}");
    }
    // The id is not secret and is useful in logs.
    assert!(format!("{:?}", key.token).contains(&key.id.simple().to_string()));
}
