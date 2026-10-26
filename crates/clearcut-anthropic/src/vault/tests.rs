// Written by Kone Tshivhinda & Claude
use super::*;

/// 32 bytes of 0x01 and of 0x02, as base64.
const MASTER_A: &str = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=";
const MASTER_B: &str = "AgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgI=";
const SENTINEL: &str = "sk-ant-api03-SENTINEL-do-not-leak-0123456789abcdefghij";

fn master(text: &str) -> MasterKey {
    MasterKey::from_base64(text, "k1").unwrap()
}

fn key() -> ApiKey {
    ApiKey::parse(SENTINEL).unwrap()
}

fn tenant(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

#[test]
fn a_sealed_key_opens_for_its_tenant_and_the_database_would_see_no_plaintext() {
    let sealed = seal(&master(MASTER_A), tenant(1), &key()).unwrap();
    assert!(!sealed
        .ciphertext
        .windows(8)
        .any(|w| SENTINEL.as_bytes().windows(8).any(|p| p == w)));
    assert_eq!(sealed.last4, "ghij");
    assert_eq!(sealed.master_key_id, "k1");
    assert_eq!(open(&master(MASTER_A), tenant(1), &sealed).unwrap(), key());
    // A fresh nonce every time: the same key sealed twice differs.
    assert_ne!(sealed, seal(&master(MASTER_A), tenant(1), &key()).unwrap());
}

#[test]
fn a_sealed_key_does_not_open_for_another_tenant_another_master_key_or_when_changed() {
    let sealed = seal(&master(MASTER_A), tenant(1), &key()).unwrap();
    // Copied to another tenant's row.
    assert_eq!(
        open(&master(MASTER_A), tenant(2), &sealed),
        Err(VaultError::CannotOpen)
    );
    // The wrong master key (same id, different bytes).
    assert_eq!(
        open(&master(MASTER_B), tenant(1), &sealed),
        Err(VaultError::CannotOpen)
    );
    // A different master key id is refused before any decryption.
    let other = MasterKey::from_base64(MASTER_A, "k2").unwrap();
    assert_eq!(open(&other, tenant(1), &sealed), Err(VaultError::CannotOpen));
    // One flipped bit.
    let mut changed = sealed.clone();
    changed.ciphertext[0] ^= 1;
    assert_eq!(
        open(&master(MASTER_A), tenant(1), &changed),
        Err(VaultError::CannotOpen)
    );
}

#[test]
fn nothing_prints_a_key_and_errors_never_repeat_one() {
    let sealed = seal(&master(MASTER_A), tenant(1), &key()).unwrap();
    for shown in [
        format!("{:?}", master(MASTER_A)),
        format!("{sealed:?}"),
        VaultError::CannotOpen.to_string(),
        MasterKey::from_base64(SENTINEL, "k1").unwrap_err().to_string(),
    ] {
        assert!(
            !shown.contains("SENTINEL") && !shown.contains(MASTER_A),
            "{shown}"
        );
    }
}

#[test]
fn a_master_key_must_be_32_bytes_of_base64_and_comes_from_the_environment_or_a_file() {
    for bad in ["", "not base64!", "AQEB", &format!("{MASTER_A}AA==")] {
        assert_eq!(
            MasterKey::from_base64(bad, "k1").unwrap_err(),
            VaultError::BadMasterKey,
            "{bad}"
        );
    }
    assert_eq!(
        MasterKey::from_base64(MASTER_A, "").unwrap_err(),
        VaultError::BadMasterKeyId
    );
    assert_eq!(
        MasterKey::from_base64(MASTER_A, "a b").unwrap_err(),
        VaultError::BadMasterKeyId
    );

    let env = |pairs: &'static [(&'static str, &'static str)]| {
        move |name: &str| {
            pairs
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| OsString::from(v))
        }
    };
    assert!(MasterKey::load_from(env(&[])).unwrap().is_none());
    let loaded = MasterKey::load_from(env(&[(MASTER_KEY_ENV, MASTER_A), (MASTER_KEY_ID_ENV, "prod-1")]))
        .unwrap()
        .unwrap();
    assert_eq!(loaded.id(), "prod-1");

    let dir = std::env::temp_dir().join(format!("clearcut-vault-test-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("master");
    fs::write(&file, format!("{MASTER_A}\n")).unwrap();
    let path: &'static str = Box::leak(file.to_string_lossy().into_owned().into_boxed_str());
    let from_file = MasterKey::load_from(env(Box::leak(Box::new([(MASTER_KEY_FILE_ENV, path)]))))
        .unwrap()
        .unwrap();
    assert_eq!(from_file.id(), "k1");
    let both = MasterKey::load_from(env(Box::leak(Box::new([
        (MASTER_KEY_ENV, MASTER_A),
        (MASTER_KEY_FILE_ENV, path),
    ]))));
    assert_eq!(both.unwrap_err(), VaultError::Ambiguous);
    fs::remove_dir_all(dir).unwrap();
}
