//! The one file `humpyard up` may keep provider keys in (ADR 0039).
//!
//! A directory of mode `0700` holds one file of mode `0600`, `keys.env`, with `NAME=value` lines. Only `up` reads it,
//! and it hands the values to the gateway it starts as environment variables the configuration already names; the
//! gateway's own loader is unchanged (ADR 0004). A value is a [`Secret`], which prints nothing of itself, and no
//! function here returns one except [`KeyStore::load`], whose only caller builds a child's environment from it.

use std::collections::BTreeMap;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// The file's name inside the store's directory.
pub const FILE: &str = "keys.env";

/// A value that cannot be printed by accident: formatting it shows nothing of it.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// A value to hold; the caller has already checked it is one worth holding.
    #[must_use]
    pub fn new(value: &str) -> Self {
        Self(value.to_string())
    }

    /// The value, for the one place that must hand it on.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

/// Why the store could not be used. No variant quotes a value.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum KeyStoreError {
    #[error("{path} is readable by other users (mode {mode:o}); run `chmod {fix} {path}`")]
    TooOpen {
        path: String,
        mode: u32,
        fix: &'static str,
    },
    #[error("{path} is not a {what}")]
    WrongKind { path: String, what: &'static str },
    #[error("cannot use {path}: {reason}")]
    Io { path: String, reason: String },
    #[error("line {line} of {path} is not NAME=value")]
    Malformed { path: String, line: usize },
    #[error(
        "`{0}` is not a variable name (letters, digits and underscores, not starting with a digit)"
    )]
    BadName(String),
    #[error("a key cannot be empty or contain a line break or control character")]
    BadValue,
}

/// Where a provider's key comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Environment,
    File,
    /// Entered for this run only and held in memory.
    Session,
    None,
}

/// The store: a directory and the file in it.
#[derive(Debug)]
pub struct KeyStore {
    dir: PathBuf,
}

fn io_error(path: &Path, e: &std::io::Error) -> KeyStoreError {
    KeyStoreError::Io {
        path: path.display().to_string(),
        reason: e.kind().to_string(),
    }
}

fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

impl KeyStore {
    /// Opens the store in `dir`, creating the directory with mode `0700` when it is not there.
    ///
    /// # Errors
    /// The directory exists with a mode that others can use, is not a directory, or cannot be made.
    pub fn open(dir: &Path) -> Result<Self, KeyStoreError> {
        match std::fs::metadata(dir) {
            Ok(meta) if !meta.is_dir() => {
                return Err(KeyStoreError::WrongKind {
                    path: dir.display().to_string(),
                    what: "directory",
                });
            }
            Ok(meta) => {
                let mode = meta.permissions().mode() & 0o777;
                if mode & 0o077 != 0 {
                    return Err(KeyStoreError::TooOpen {
                        path: dir.display().to_string(),
                        mode,
                        fix: "700",
                    });
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                std::fs::DirBuilder::new()
                    .recursive(true)
                    .mode(0o700)
                    .create(dir)
                    .map_err(|e| io_error(dir, &e))?;
            }
            Err(e) => return Err(io_error(dir, &e)),
        }
        Ok(Self {
            dir: dir.to_path_buf(),
        })
    }

    fn file(&self) -> PathBuf {
        self.dir.join(FILE)
    }

    /// Every saved key, by variable name. A missing file is an empty store.
    ///
    /// # Errors
    /// The file is readable by others, is not a regular file, or has a line that is not `NAME=value`.
    pub fn load(&self) -> Result<BTreeMap<String, Secret>, KeyStoreError> {
        let path = self.file();
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
            Err(e) => return Err(io_error(&path, &e)),
        };
        if !meta.is_file() {
            return Err(KeyStoreError::WrongKind {
                path: path.display().to_string(),
                what: "regular file",
            });
        }
        let mode = meta.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            return Err(KeyStoreError::TooOpen {
                path: path.display().to_string(),
                mode,
                fix: "600",
            });
        }
        let text = std::fs::read_to_string(&path).map_err(|e| io_error(&path, &e))?;
        let mut keys = BTreeMap::new();
        for (index, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let malformed = || KeyStoreError::Malformed {
                path: path.display().to_string(),
                line: index + 1,
            };
            let (name, value) = line.split_once('=').ok_or_else(malformed)?;
            if !valid_name(name) || value.is_empty() {
                return Err(malformed());
            }
            keys.insert(name.to_string(), Secret(value.to_string()));
        }
        Ok(keys)
    }

    /// Replaces the file with `keys`: a new file created `0600`, written, synced and renamed over the old one, so a
    /// reader sees the old content or the new and the mode is never wider than `0600`.
    ///
    /// # Errors
    /// A name or value that is not allowed, or a file that cannot be written.
    pub fn save(&self, keys: &BTreeMap<String, Secret>) -> Result<(), KeyStoreError> {
        let mut text = String::from("# Written by `humpyard up`. Read only by `humpyard up`.\n");
        for (name, value) in keys {
            if !valid_name(name) {
                return Err(KeyStoreError::BadName(name.clone()));
            }
            if value.0.is_empty() || value.0.chars().any(char::is_control) {
                return Err(KeyStoreError::BadValue);
            }
            text.push_str(name);
            text.push('=');
            text.push_str(&value.0);
            text.push('\n');
        }
        let target = self.file();
        let temp = self.dir.join(format!("{FILE}.new"));
        // A leftover from a crash would make `create_new` fail; it holds nothing the store still needs.
        let _ = std::fs::remove_file(&temp);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)
            .map_err(|e| io_error(&temp, &e))?;
        let written = file
            .write_all(text.as_bytes())
            .and_then(|()| file.sync_all());
        if let Err(e) = written {
            let _ = std::fs::remove_file(&temp);
            return Err(io_error(&temp, &e));
        }
        std::fs::rename(&temp, &target).map_err(|e| {
            let _ = std::fs::remove_file(&temp);
            io_error(&target, &e)
        })
    }

    /// Adds or replaces one key and saves.
    ///
    /// # Errors
    /// As [`KeyStore::load`] and [`KeyStore::save`].
    pub fn set(&self, name: &str, value: &str) -> Result<(), KeyStoreError> {
        let mut keys = self.load()?;
        keys.insert(name.to_string(), Secret(value.to_string()));
        self.save(&keys)
    }

    /// Removes one key and saves. A name that is not there is not an error.
    ///
    /// # Errors
    /// As [`KeyStore::load`] and [`KeyStore::save`].
    pub fn remove(&self, name: &str) -> Result<(), KeyStoreError> {
        let mut keys = self.load()?;
        if keys.remove(name).is_some() {
            self.save(&keys)?;
        }
        Ok(())
    }
}

/// The variables to add to a child's environment: each saved key whose name the real environment does not already
/// set to something. The environment always wins, so a user with a secret manager is unaffected.
#[must_use]
pub fn overlay(
    saved: &BTreeMap<String, Secret>,
    env: &dyn Fn(&str) -> Option<String>,
) -> Vec<(String, Secret)> {
    saved
        .iter()
        .filter(|(name, _)| env(name).is_none_or(|v| v.is_empty()))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

/// Where the key for `name` comes from, for a status that says nothing else about it. The environment wins, then a
/// key entered for this run, then the file.
#[must_use]
pub fn source(
    name: &str,
    saved: &BTreeMap<String, Secret>,
    session: &BTreeMap<String, Secret>,
    env: &dyn Fn(&str) -> Option<String>,
) -> Source {
    if env(name).is_some_and(|v| !v.is_empty()) {
        Source::Environment
    } else if session.contains_key(name) {
        Source::Session
    } else if saved.contains_key(name) {
        Source::File
    } else {
        Source::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_checked() {
        assert!(valid_name("GROQ_API_KEY"));
        assert!(valid_name("_x1"));
        assert!(!valid_name(""));
        assert!(!valid_name("1X"));
        assert!(!valid_name("A-B"));
        assert!(!valid_name("A B"));
    }
}
