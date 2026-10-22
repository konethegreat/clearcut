// Written by Kone Tshivhinda & Claude
use super::*;
use std::collections::HashMap;
use std::fs;

const KEY: &str =
    "sk-ant-api03-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789_-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789_-wxyz";

/// A fake environment, so no test touches the real one (it is shared by the whole process).
fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
    let map: HashMap<String, String> = pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect();
    move |name| map.get(name).map(OsString::from)
}

// ---- loading ----

#[test]
fn a_key_in_the_environment_is_loaded() {
    let key = load_from(env(&[(KEY_ENV, KEY)])).unwrap();
    assert_eq!(key.expose(), KEY);
}

#[test]
fn a_key_in_a_file_is_loaded_and_its_trailing_newline_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secret");
    fs::write(&path, format!("{KEY}\n")).unwrap();
    let key = load_from(env(&[(KEY_FILE_ENV, path.to_str().unwrap())])).unwrap();
    assert_eq!(key.expose(), KEY);
    fs::write(&path, format!("{KEY}\r\n")).unwrap();
    assert_eq!(
        load_from(env(&[(KEY_FILE_ENV, path.to_str().unwrap())]))
            .unwrap()
            .expose(),
        KEY
    );
}

#[test]
fn nothing_configured_is_its_own_error() {
    assert_eq!(load_from(env(&[])).unwrap_err(), LoadError::NotConfigured);
}

#[test]
fn an_empty_variable_counts_as_not_set() {
    // Containers often define variables with empty values.
    assert_eq!(
        load_from(env(&[(KEY_ENV, "")])).unwrap_err(),
        LoadError::NotConfigured
    );
    assert_eq!(
        load_from(env(&[(KEY_FILE_ENV, "")])).unwrap_err(),
        LoadError::NotConfigured
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secret");
    fs::write(&path, KEY).unwrap();
    let key = load_from(env(&[(KEY_ENV, ""), (KEY_FILE_ENV, path.to_str().unwrap())])).unwrap();
    assert_eq!(key.expose(), KEY);
}

#[test]
fn both_ways_at_once_is_refused_rather_than_guessed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secret");
    fs::write(&path, KEY).unwrap();
    let error = load_from(env(&[(KEY_ENV, KEY), (KEY_FILE_ENV, path.to_str().unwrap())])).unwrap_err();
    assert_eq!(error, LoadError::Ambiguous);
}

#[test]
fn a_file_that_is_missing_unreadable_or_odd_is_reported_by_kind() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nope");
    assert_eq!(
        load_from(env(&[(KEY_FILE_ENV, missing.to_str().unwrap())])).unwrap_err(),
        LoadError::FileUnreadable(std::io::ErrorKind::NotFound)
    );
    assert_eq!(
        load_from(env(&[(KEY_FILE_ENV, dir.path().to_str().unwrap())])).unwrap_err(),
        LoadError::FileNotAPlainFile
    );
}

#[test]
fn a_file_of_exactly_the_limit_is_read_and_one_byte_more_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secret");
    let pad = |total: usize| format!("{KEY}{}", " ".repeat(total - KEY.len()));
    fs::write(&path, pad(MAX_KEY_FILE_BYTES)).unwrap();
    assert!(load_from(env(&[(KEY_FILE_ENV, path.to_str().unwrap())])).is_ok());
    fs::write(&path, pad(MAX_KEY_FILE_BYTES + 1)).unwrap();
    assert_eq!(
        load_from(env(&[(KEY_FILE_ENV, path.to_str().unwrap())])).unwrap_err(),
        LoadError::FileTooLarge
    );
}

#[cfg(unix)]
#[test]
fn a_link_to_the_file_is_followed_as_secret_mounts_are_links() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real");
    fs::write(&real, KEY).unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    assert!(load_from(env(&[(KEY_FILE_ENV, link.to_str().unwrap())])).is_ok());
}

#[test]
fn a_bad_key_is_reported_as_a_key_problem_without_its_text() {
    let error = load_from(env(&[(KEY_ENV, "SECRETTEXT not a key")])).unwrap_err();
    assert_eq!(error, LoadError::Key(KeyError::Whitespace));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secret");
    fs::write(&path, "sk-proj-SECRETTEXT").unwrap();
    let error = load_from(env(&[(KEY_FILE_ENV, path.to_str().unwrap())])).unwrap_err();
    assert_eq!(error, LoadError::Key(KeyError::NotAnApiKey));
}

#[test]
fn a_value_that_is_not_text_is_a_bad_key() {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        let read = |name: &str| (name == KEY_ENV).then(|| OsString::from_vec(vec![0xff, 0xfe, 0xfd]));
        assert_eq!(
            load_from(read).unwrap_err(),
            LoadError::Key(KeyError::BadCharacters)
        );
    }
}

#[test]
fn no_error_names_the_file_or_repeats_what_it_held() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secretfilename");
    fs::write(&path, "SECRETTEXT").unwrap();
    for error in [
        load_from(env(&[(KEY_FILE_ENV, path.to_str().unwrap())])).unwrap_err(),
        load_from(env(&[(
            KEY_FILE_ENV,
            dir.path().join("secretfilename2").to_str().unwrap(),
        )]))
        .unwrap_err(),
        load_from(env(&[(KEY_FILE_ENV, dir.path().to_str().unwrap())])).unwrap_err(),
        load_from(env(&[(KEY_ENV, "SECRETTEXT")])).unwrap_err(),
    ] {
        let shown = format!("{error} {error:?}");
        assert!(
            !shown.contains("SECRET") && !shown.contains("secretfilename"),
            "{shown}"
        );
    }
}

// ---- refusing a key on the command line ----

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|a| (*a).to_string()).collect()
}

#[test]
fn ordinary_arguments_are_fine() {
    for list in [
        args(&[]),
        args(&["serve"]),
        args(&["--port", "8080", "--host=127.0.0.1"]),
        args(&["--help"]),
        args(&["check", "D:\\gold"]),
        args(&["--key-file", "/run/secrets/anthropic"]),
        args(&["--name", "ant-farm", "sk"]),
    ] {
        assert_eq!(reject_key_in_arguments(&list), Ok(()), "{list:?}");
    }
}

#[test]
fn a_key_anywhere_on_the_command_line_is_refused() {
    for list in [
        args(&[KEY]),
        args(&["serve", KEY]),
        args(&["--note", &format!("hello {KEY} world")]),
        args(&[&format!("--something={KEY}")]),
        args(&["sk-ant-api03-short"]),
        args(&["sk-ant-whatever"]),
    ] {
        assert_eq!(
            reject_key_in_arguments(&list),
            Err(ArgsError::KeyOnCommandLine),
            "{list:?}"
        );
    }
}

#[test]
fn a_flag_that_would_take_a_key_is_refused_even_without_a_value() {
    for flag in [
        "--api-key",
        "--api-key=abc",
        "--anthropic-api-key",
        "--anthropic-key=x",
        "--apikey",
        "--anthropic_api_key",
        "--ANTHROPIC-API-KEY",
    ] {
        assert_eq!(
            reject_key_in_arguments(args(&[flag])),
            Err(ArgsError::KeyOnCommandLine),
            "{flag}"
        );
    }
}

#[test]
fn the_refusal_never_repeats_the_argument() {
    let error = reject_key_in_arguments(args(&["serve", &format!("--x={KEY}")])).unwrap_err();
    let shown = format!("{error} {error:?}");
    assert!(!shown.contains("sk-ant") && !shown.contains("AbCdEf"), "{shown}");
    assert!(shown.contains(KEY_ENV) && shown.contains(KEY_FILE_ENV), "{shown}");
}

#[test]
fn the_key_file_limit_is_four_kibibytes_exactly() {
    assert_eq!(MAX_KEY_FILE_BYTES, 4096);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secret");
    for (total, fine) in [(4096usize, true), (4097, false)] {
        fs::write(&path, format!("{KEY}{}", " ".repeat(total - KEY.len()))).unwrap();
        let loaded = load_from(env(&[(KEY_FILE_ENV, path.to_str().unwrap())]));
        assert_eq!(loaded.is_ok(), fine, "{total}");
    }
}

#[test]
fn a_flag_with_a_single_dash_is_refused_too() {
    for flag in ["-api-key", "-apikey=abc", "-anthropic-api-key"] {
        assert_eq!(
            reject_key_in_arguments(args(&[flag])),
            Err(ArgsError::KeyOnCommandLine),
            "{flag}"
        );
    }
    // A lone dash, and short options that are not key flags, are fine.
    for fine in ["-", "-v", "-p8080", "--"] {
        assert_eq!(reject_key_in_arguments(args(&[fine])), Ok(()), "{fine}");
    }
}
