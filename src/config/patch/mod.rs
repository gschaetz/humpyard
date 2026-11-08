//! Typed edits to the configuration file's text, and the reviewable diff of the result (ADR 0034).
//!
//! An [`Edit`] names a place in the file by a path of table names, keys and array positions and says what to do
//! there. [`apply`] performs a list of them on the file's text with a format-preserving editor, so a line no edit
//! touches comes back byte for byte, and returns the new text. It does not decide whether that text is a valid
//! configuration (the caller loads it with the real loader) and it never writes anything.

mod diff;
mod edit;

use serde::Deserialize;

pub use diff::masked_diff;
pub use edit::apply;

/// The most edits one request may carry.
pub const MAX_EDITS: usize = 64;
/// The longest path an edit may name.
pub const MAX_PATH_DEPTH: usize = 8;
/// How deeply a value may nest (arrays and objects within arrays and objects).
pub const MAX_VALUE_DEPTH: usize = 4;

/// One step of a path: a table or key name, or a position in an array.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum Segment {
    Index(usize),
    Key(String),
}

/// One change to the file. The path always names the place inside the file; the value is JSON, mapped to TOML.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Edit {
    /// Put a value at a key or array position; the key may be new, its table must exist.
    Set {
        path: Vec<Segment>,
        value: serde_json::Value,
    },
    /// Take a key or array element away.
    Remove { path: Vec<Segment> },
    /// Add a new table (`value` is an object holding its keys).
    AddTable {
        path: Vec<Segment>,
        value: serde_json::Value,
    },
    /// Take a whole table away, with the comments in front of it.
    RemoveTable { path: Vec<Segment> },
}

/// Why a list of edits could not be applied.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum PatchError {
    #[error("at most {MAX_EDITS} edits in one request")]
    TooManyEdits,
    /// The edit at `index` (counting from 0) is not allowed or does not fit the file.
    #[error("edit {index}: {reason}")]
    Invalid { index: usize, reason: String },
    /// The file itself does not parse as TOML.
    #[error("the configuration file does not parse: {0}")]
    Unparsable(String),
}

#[cfg(test)]
mod tests;
