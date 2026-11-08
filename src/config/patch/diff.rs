//! The reviewable diff of a patch (ADR 0034): a unified diff of the file before and after, with every value that
//! could be a secret shown as `<redacted>`.
//!
//! The masking walks the parsed document, not the lines, so it finds a hash or a header value however the file
//! spells it (inline table, standard table, array of tables). A hash is not a secret to guess but the gateway
//! never shows one (ADR 0004); a provider header value can be one. Because a masked value looks the same before
//! and after, a value that changed is marked `<redacted: changed>` on the after side, so the diff still shows
//! that something moved without saying what.

use std::collections::{BTreeMap, BTreeSet};

use toml_edit::{DocumentMut, Item, Value};

use super::PatchError;

const MASK: &str = "<redacted>";
const MASK_CHANGED: &str = "<redacted: changed>";

/// A unified diff of `before` to `after` with three lines of context, empty when they are the same text.
///
/// # Errors
/// [`PatchError::Unparsable`] when either text is not TOML.
pub fn masked_diff(before: &str, after: &str) -> Result<String, PatchError> {
    let mut old = parse(before)?;
    let mut new = parse(after)?;
    let changed = changed_secrets(&old, &new);
    mask(&mut old, &BTreeSet::new());
    mask(&mut new, &changed);
    let (old, new) = (old.to_string(), new.to_string());
    Ok(similar::TextDiff::from_lines(&old, &new)
        .unified_diff()
        .context_radius(3)
        .header("current", "patched")
        .to_string())
}

fn parse(text: &str) -> Result<DocumentMut, PatchError> {
    text.parse()
        .map_err(|e: toml_edit::TomlError| PatchError::Unparsable(e.to_string()))
}

/// Whether a key holds a value the diff must not show: any hash, and everything under `headers`.
fn is_secret_key(key: &str) -> bool {
    key.ends_with("sha256") || key == "headers"
}

/// Calls `visit` with the path and the value of every scalar that is, or sits under, a secret key.
fn walk(item: &mut Item, path: &str, secret: bool, visit: &mut dyn FnMut(&str, &mut Value)) {
    match item {
        Item::Table(table) => {
            for (key, child) in table.iter_mut() {
                walk_child(child, path, key.get(), secret, visit);
            }
        }
        Item::ArrayOfTables(tables) => {
            for (at, table) in tables.iter_mut().enumerate() {
                let path = format!("{path}[{at}]");
                for (key, child) in table.iter_mut() {
                    walk_child(child, &path, key.get(), secret, visit);
                }
            }
        }
        Item::Value(value) => walk_value(value, path, secret, visit),
        Item::None => {}
    }
}

fn walk_child(
    child: &mut Item,
    path: &str,
    key: &str,
    secret: bool,
    visit: &mut dyn FnMut(&str, &mut Value),
) {
    walk(
        child,
        &format!("{path}.{key}"),
        secret || is_secret_key(key),
        visit,
    );
}

fn walk_value(
    value: &mut Value,
    path: &str,
    secret: bool,
    visit: &mut dyn FnMut(&str, &mut Value),
) {
    match value {
        Value::InlineTable(table) => {
            for (key, child) in table.iter_mut() {
                let key = key.get();
                let path = format!("{path}.{key}");
                walk_value(child, &path, secret || is_secret_key(key), visit);
            }
        }
        Value::Array(array) => {
            for (at, child) in array.iter_mut().enumerate() {
                walk_value(child, &format!("{path}[{at}]"), secret, visit);
            }
        }
        scalar if secret => visit(path, scalar),
        _ => {}
    }
}

/// The paths whose secret value is not the same before and after.
fn changed_secrets(old: &DocumentMut, new: &DocumentMut) -> BTreeSet<String> {
    let (before, after) = (secrets(old), secrets(new));
    after
        .into_iter()
        .filter(|(path, value)| before.get(path).is_some_and(|was| was != value))
        .map(|(path, _)| path)
        .collect()
}

fn secrets(doc: &DocumentMut) -> BTreeMap<String, String> {
    let mut found = BTreeMap::new();
    let mut copy = doc.clone();
    walk(copy.as_item_mut(), "", false, &mut |path, value| {
        found.insert(path.to_string(), value.to_string().trim().to_string());
    });
    found
}

fn mask(doc: &mut DocumentMut, changed: &BTreeSet<String>) {
    walk(doc.as_item_mut(), "", false, &mut |path, value| {
        let decor = value.decor().clone();
        let text = if changed.contains(path) {
            MASK_CHANGED
        } else {
            MASK
        };
        *value = Value::from(text);
        *value.decor_mut() = decor;
    });
}
