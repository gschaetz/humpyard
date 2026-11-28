//! The key file `humpyard up` keeps (ADR 0039): modes, atomic replace, refusal of a loose file, no value in an error.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // integration tests fail loudly on purpose

mod common;

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;

use humpyard::keystore::{KeyStore, KeyStoreError, Source, overlay, source};

fn dir(name: &str) -> std::path::PathBuf {
    let d = common::scratch().join(format!("keystore-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn mode(path: &std::path::Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn a_saved_key_is_in_a_private_file_in_a_private_directory() {
    let d = dir("modes");
    let store = KeyStore::open(&d).unwrap();
    store.set("GROQ_API_KEY", "gsk-secret-value").unwrap();
    assert_eq!(mode(&d), 0o700);
    assert_eq!(mode(&d.join("keys.env")), 0o600);
    // Nothing else holds it: the directory has the one file.
    let names: Vec<_> = std::fs::read_dir(&d)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, vec!["keys.env".to_string()]);
}

#[test]
fn a_key_round_trips_and_is_replaced_and_removed() {
    let store = KeyStore::open(&dir("roundtrip")).unwrap();
    store.set("A_KEY", "one").unwrap();
    store.set("B_KEY", "two").unwrap();
    store.set("A_KEY", "three").unwrap();
    let keys = store.load().unwrap();
    assert_eq!(keys["A_KEY"].expose(), "three");
    assert_eq!(keys["B_KEY"].expose(), "two");
    store.remove("A_KEY").unwrap();
    store.remove("NOT_THERE").unwrap();
    let keys = store.load().unwrap();
    assert!(!keys.contains_key("A_KEY"));
    assert_eq!(keys.len(), 1);
}

#[test]
fn replacing_never_widens_the_mode_and_leaves_no_temporary_file() {
    let d = dir("replace");
    let store = KeyStore::open(&d).unwrap();
    store.set("A_KEY", "one").unwrap();
    store.set("A_KEY", "two").unwrap();
    assert_eq!(mode(&d.join("keys.env")), 0o600);
    assert!(!d.join("keys.env.new").exists());
}

#[test]
fn a_missing_file_is_an_empty_store() {
    let store = KeyStore::open(&dir("empty")).unwrap();
    assert!(store.load().unwrap().is_empty());
}

#[test]
fn a_file_others_can_read_is_refused_and_says_how_to_fix_it() {
    let d = dir("loose-file");
    let store = KeyStore::open(&d).unwrap();
    store.set("A_KEY", "super-secret").unwrap();
    std::fs::set_permissions(d.join("keys.env"), std::fs::Permissions::from_mode(0o644)).unwrap();
    let error = store.load().unwrap_err();
    assert!(matches!(error, KeyStoreError::TooOpen { fix: "600", .. }));
    let text = error.to_string();
    assert!(text.contains("chmod 600"), "{text}");
    assert!(!text.contains("super-secret"), "{text}");
}

#[test]
fn a_directory_others_can_use_is_refused() {
    let d = dir("loose-dir");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o755)).unwrap();
    let error = KeyStore::open(&d).unwrap_err();
    assert!(matches!(error, KeyStoreError::TooOpen { fix: "700", .. }));
}

#[test]
fn a_symbolic_link_in_place_of_the_file_is_refused() {
    let d = dir("symlink");
    let store = KeyStore::open(&d).unwrap();
    let elsewhere = common::scratch().join("keystore-symlink-target");
    std::fs::write(&elsewhere, "A_KEY=x\n").unwrap();
    std::os::unix::fs::symlink(&elsewhere, d.join("keys.env")).unwrap();
    assert!(matches!(
        store.load().unwrap_err(),
        KeyStoreError::WrongKind { .. }
    ));
}

#[test]
fn a_malformed_line_names_the_line_and_not_its_content() {
    let d = dir("malformed");
    let store = KeyStore::open(&d).unwrap();
    std::fs::write(
        d.join("keys.env"),
        "A_KEY=fine\nthis-has-no-equals-sign-or-secret\n",
    )
    .unwrap();
    std::fs::set_permissions(d.join("keys.env"), std::fs::Permissions::from_mode(0o600)).unwrap();
    let error = store.load().unwrap_err();
    assert!(matches!(error, KeyStoreError::Malformed { line: 2, .. }));
    assert!(!error.to_string().contains("this-has-no-equals"));
}

#[test]
fn a_bad_name_or_value_is_refused_before_anything_is_written() {
    let d = dir("bad-input");
    let store = KeyStore::open(&d).unwrap();
    assert!(matches!(
        store.set("not a name", "v"),
        Err(KeyStoreError::BadName(_))
    ));
    assert_eq!(
        store.set("A_KEY", "line\nbreak"),
        Err(KeyStoreError::BadValue)
    );
    assert_eq!(store.set("A_KEY", ""), Err(KeyStoreError::BadValue));
    assert!(!d.join("keys.env").exists());
}

#[test]
fn no_value_appears_in_debug_output() {
    let store = KeyStore::open(&dir("debug")).unwrap();
    store.set("A_KEY", "never-print-this").unwrap();
    let keys = store.load().unwrap();
    assert!(!format!("{keys:?}").contains("never-print-this"));
    assert!(!format!("{store:?}").contains("never-print-this"));
}

#[test]
fn the_environment_wins_over_the_file() {
    let store = KeyStore::open(&dir("overlay")).unwrap();
    store.set("FROM_FILE", "file-value").unwrap();
    store.set("IN_BOTH", "file-value").unwrap();
    let saved = store.load().unwrap();
    let env = |name: &str| match name {
        "IN_BOTH" => Some("env-value".to_string()),
        "EMPTY_IN_ENV" => Some(String::new()),
        _ => None,
    };
    let added: BTreeMap<String, String> = overlay(&saved, &env)
        .into_iter()
        .map(|(n, v)| (n, v.expose().to_string()))
        .collect();
    assert_eq!(added.len(), 1);
    assert_eq!(added["FROM_FILE"], "file-value");
    let none = BTreeMap::new();
    assert_eq!(source("IN_BOTH", &saved, &none, &env), Source::Environment);
    assert_eq!(source("FROM_FILE", &saved, &none, &env), Source::File);
    assert_eq!(source("NOWHERE", &saved, &none, &env), Source::None);
    // An empty variable is not a key.
    assert_eq!(source("EMPTY_IN_ENV", &saved, &none, &env), Source::None);
    // A key entered for this run wins over the file and loses to the environment.
    let mut session = BTreeMap::new();
    session.insert("FROM_FILE".to_string(), saved["FROM_FILE"].clone());
    session.insert("IN_BOTH".to_string(), saved["IN_BOTH"].clone());
    assert_eq!(source("FROM_FILE", &saved, &session, &env), Source::Session);
    assert_eq!(
        source("IN_BOTH", &saved, &session, &env),
        Source::Environment
    );
}

/// ADR 0004 is unchanged: the loader reads the environment it is given, never the key file, so a variable only the
/// file holds is still a missing-variable failure for `serve`.
#[test]
fn serve_does_not_read_the_file() {
    let store = KeyStore::open(&dir("serve")).unwrap();
    store.set("ONLY_IN_FILE", "file-value").unwrap();
    let text = "listen = \"127.0.0.1:0\"\n\
                [providers.p]\nbase_url = \"http://127.0.0.1:1\"\napi_key_env = \"ONLY_IN_FILE\"\n\
                [targets.t]\nendpoints = [{ provider = \"p\", model = \"m\" }]\n";
    let error = humpyard::config::Config::from_toml(text, |_| None).unwrap_err();
    assert!(
        error.to_string().contains("ONLY_IN_FILE") && error.to_string().contains("is not set"),
        "{error}"
    );
}
