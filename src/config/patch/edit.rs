//! Applying edits to the text of the file with `toml_edit`, which keeps comments, ordering and spacing for
//! everything an edit does not touch.

use serde_json::Value as Json;
use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, Value};

use super::{Edit, MAX_EDITS, MAX_PATH_DEPTH, MAX_VALUE_DEPTH, PatchError, Segment};

/// The sections a patch may reach. `admin` (credentials), `metrics`, `ledger`, `discovery`, the top-level
/// settings and the `select` rules are not on the list (ADR 0034).
const EDITABLE: [&str; 9] = [
    "providers",
    "targets",
    "routes",
    "profiles",
    "aliases",
    "budget",
    "health",
    "overrides",
    "keys",
];

/// The longest name a path segment may have.
const MAX_NAME_LEN: usize = 128;

/// Applies `edits` in order to `text` and returns the edited text. Nothing is applied unless every edit is.
///
/// # Errors
/// [`PatchError::TooManyEdits`] past [`MAX_EDITS`], [`PatchError::Invalid`] naming the first edit that is not
/// allowed or does not fit the file, and [`PatchError::Unparsable`] when `text` is not TOML.
pub fn apply(text: &str, edits: &[Edit]) -> Result<String, PatchError> {
    if edits.len() > MAX_EDITS {
        return Err(PatchError::TooManyEdits);
    }
    for (index, edit) in edits.iter().enumerate() {
        allowed(edit).map_err(|reason| PatchError::Invalid { index, reason })?;
    }
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|e: toml_edit::TomlError| PatchError::Unparsable(e.to_string()))?;
    for (index, edit) in edits.iter().enumerate() {
        perform(&mut doc, edit).map_err(|reason| PatchError::Invalid { index, reason })?;
    }
    Ok(doc.to_string())
}

fn path_of(edit: &Edit) -> &[Segment] {
    match edit {
        Edit::Set { path, .. }
        | Edit::Remove { path }
        | Edit::AddTable { path, .. }
        | Edit::RemoveTable { path } => path,
    }
}

/// Whether the edit may be made at all, judged from its path and kind alone.
fn allowed(edit: &Edit) -> Result<(), String> {
    let path = path_of(edit);
    if path.is_empty() {
        return Err("the path is empty".into());
    }
    if path.len() > MAX_PATH_DEPTH {
        return Err(format!("the path is deeper than {MAX_PATH_DEPTH}"));
    }
    for segment in path {
        if let Segment::Key(name) = segment {
            valid_name(name)?;
        }
    }
    let Segment::Key(section) = &path[0] else {
        return Err("the path must start with a section name".into());
    };
    if !EDITABLE.contains(&section.as_str()) {
        return Err(format!("`{section}` cannot be edited through a patch"));
    }
    if section == "keys" {
        key_edit(edit, path)?;
    }
    Ok(())
}

/// A key is a credential: only its settings can change, and never its hash or its existence.
fn key_edit(edit: &Edit, path: &[Segment]) -> Result<(), String> {
    if matches!(edit, Edit::AddTable { .. } | Edit::RemoveTable { .. }) {
        return Err("a key cannot be added or removed through a patch".into());
    }
    let Some(Segment::Key(field)) = path.get(2) else {
        return Err("only a setting of a key can be edited, as `keys.<id>.<setting>`".into());
    };
    if field == "sha256" {
        return Err("a key's hash cannot be edited through a patch".into());
    }
    Ok(())
}

fn valid_name(name: &str) -> Result<(), String> {
    let fine =
        |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':' | '+' | '/');
    if name.is_empty() || name.len() > MAX_NAME_LEN || !name.chars().all(fine) {
        return Err(format!(
            "`{}` is not a valid name (1 to {MAX_NAME_LEN} letters, digits and `_-.:+/`)",
            name.escape_debug()
        ));
    }
    Ok(())
}

fn perform(doc: &mut DocumentMut, edit: &Edit) -> Result<(), String> {
    match edit {
        Edit::Set { path, value } => {
            let value = to_value(value, 1)?;
            let (last, parent) = split(path);
            set(descend(doc.as_table_mut(), parent)?, last, value)
        }
        Edit::Remove { path } => {
            let (last, parent) = split(path);
            remove(descend(doc.as_table_mut(), parent)?, last, false)
        }
        Edit::AddTable { path, value } => {
            let table = to_table(value)?;
            let (last, parent) = split(path);
            if let (Some(Segment::Key(section)), true) = (parent.first(), parent.len() == 1) {
                // The first table of a section (the file has no `[profiles.x]` yet) creates the section.
                if doc.get(section).is_none() {
                    let mut implicit = Table::new();
                    implicit.set_implicit(true);
                    doc.insert(section, Item::Table(implicit));
                }
            }
            add_table(descend(doc.as_table_mut(), parent)?, last, table)
        }
        Edit::RemoveTable { path } => {
            let (last, parent) = split(path);
            remove(descend(doc.as_table_mut(), parent)?, last, true)
        }
    }
}

/// The last segment of a path, and the segments before it.
fn split(path: &[Segment]) -> (&Segment, &[Segment]) {
    // `allowed` has refused an empty path before anything is performed.
    let (last, parent) = path.split_last().unwrap_or((&path[0], &[]));
    (last, parent)
}

/// A place in the document an edit can stand in or step through.
enum Node<'a> {
    Table(&'a mut Table),
    Item(&'a mut Item),
    Value(&'a mut Value),
}

fn descend<'a>(root: &'a mut Table, path: &[Segment]) -> Result<Node<'a>, String> {
    let mut node = Node::Table(root);
    for (depth, segment) in path.iter().enumerate() {
        node = step(node, segment)
            .ok_or_else(|| format!("there is nothing at {}", render_path(&path[..=depth])))?;
    }
    Ok(node)
}

fn step<'a>(node: Node<'a>, segment: &Segment) -> Option<Node<'a>> {
    match (node, segment) {
        (Node::Table(table), Segment::Key(key)) => table.get_mut(key).map(Node::Item),
        (Node::Item(item), Segment::Key(key)) => {
            item.as_table_like_mut()?.get_mut(key).map(Node::Item)
        }
        (Node::Item(item), Segment::Index(at)) => match item {
            Item::ArrayOfTables(tables) => tables.get_mut(*at).map(Node::Table),
            other => other.as_array_mut()?.get_mut(*at).map(Node::Value),
        },
        (Node::Value(value), Segment::Key(key)) => {
            value.as_inline_table_mut()?.get_mut(key).map(Node::Value)
        }
        (Node::Value(value), Segment::Index(at)) => {
            value.as_array_mut()?.get_mut(*at).map(Node::Value)
        }
        (Node::Table(_), Segment::Index(_)) => None,
    }
}

fn render_path(path: &[Segment]) -> String {
    path.iter()
        .map(|segment| match segment {
            Segment::Key(name) => name.clone(),
            Segment::Index(at) => format!("[{at}]"),
        })
        .collect::<Vec<_>>()
        .join(".")
}

fn set(parent: Node<'_>, last: &Segment, mut value: Value) -> Result<(), String> {
    match (parent, last) {
        (Node::Table(table), Segment::Key(key)) => set_in_table_like(table, key, value),
        (Node::Item(item), Segment::Key(key)) => match item.as_table_like_mut() {
            Some(table) => set_in_table_like(table, key, value),
            None => match item.as_inline_table_mut() {
                Some(inline) => {
                    set_in_inline(inline, key, value);
                    Ok(())
                }
                None => Err("that is not a table".into()),
            },
        },
        (Node::Value(parent), Segment::Key(key)) => match parent.as_inline_table_mut() {
            Some(inline) => {
                set_in_inline(inline, key, value);
                Ok(())
            }
            None => Err("that is not a table".into()),
        },
        (parent, Segment::Index(at)) => {
            let array = array_of(parent)?;
            let old = array
                .get(*at)
                .ok_or_else(|| format!("the array has no position {at}"))?;
            *value.decor_mut() = old.decor().clone();
            array.replace(*at, value);
            Ok(())
        }
    }
}

fn set_in_table_like(
    table: &mut dyn toml_edit::TableLike,
    key: &str,
    mut value: Value,
) -> Result<(), String> {
    match table.get_mut(key) {
        Some(existing) if existing.is_table() || existing.is_array_of_tables() => Err(format!(
            "`{key}` is a table; use add_table and remove_table to change a whole table"
        )),
        Some(existing) => {
            if let Some(old) = existing.as_value() {
                *value.decor_mut() = old.decor().clone();
            }
            *existing = Item::Value(value);
            Ok(())
        }
        None => {
            table.insert(key, Item::Value(value));
            Ok(())
        }
    }
}

fn set_in_inline(inline: &mut InlineTable, key: &str, mut value: Value) {
    if let Some(old) = inline.get(key) {
        *value.decor_mut() = old.decor().clone();
    }
    inline.insert(key, value);
}

fn array_of(node: Node<'_>) -> Result<&mut Array, String> {
    let array = match node {
        Node::Item(item) => item.as_array_mut(),
        Node::Value(value) => value.as_array_mut(),
        Node::Table(_) => None,
    };
    array.ok_or_else(|| "that is not an array".into())
}

fn remove(parent: Node<'_>, last: &Segment, whole_table: bool) -> Result<(), String> {
    let gone = match (parent, last) {
        (Node::Table(table), Segment::Key(key)) => remove_from_table(table, key, whole_table)?,
        (Node::Item(item), Segment::Key(key)) => {
            if let Some(table) = item.as_table_like_mut() {
                remove_from_table(table, key, whole_table)?
            } else {
                let inline = item.as_inline_table_mut().ok_or("that is not a table")?;
                !whole_table && inline.remove(key).is_some()
            }
        }
        (Node::Value(parent), Segment::Key(key)) => {
            let inline = parent.as_inline_table_mut().ok_or("that is not a table")?;
            !whole_table && inline.remove(key).is_some()
        }
        (parent, Segment::Index(at)) => {
            let array = array_of(parent)?;
            let present = *at < array.len();
            if present && !whole_table {
                array.remove(*at);
            }
            present && !whole_table
        }
    };
    if gone {
        Ok(())
    } else if whole_table {
        Err("there is no such table to remove".into())
    } else {
        Err("there is nothing there to remove".into())
    }
}

fn remove_from_table(
    table: &mut dyn toml_edit::TableLike,
    key: &str,
    whole_table: bool,
) -> Result<bool, String> {
    match table.get(key) {
        None => Ok(false),
        Some(item) if whole_table && !(item.is_table_like() || item.is_array_of_tables()) => {
            Err(format!("`{key}` is not a table; use remove"))
        }
        Some(item) if !whole_table && (item.is_table() || item.is_array_of_tables()) => Err(
            format!("`{key}` is a table; use remove_table to take a whole table away"),
        ),
        Some(_) => Ok(table.remove(key).is_some()),
    }
}

fn add_table(parent: Node<'_>, last: &Segment, table: Table) -> Result<(), String> {
    let Segment::Key(key) = last else {
        return Err("a table is added under a name, not at an array position".into());
    };
    let parent: &mut dyn toml_edit::TableLike = match parent {
        Node::Table(table) => table,
        Node::Item(item) => match item.as_table_mut() {
            Some(table) => table,
            None => {
                return Err("a table can only be added inside a standard table".into());
            }
        },
        Node::Value(_) => {
            return Err("a table can only be added inside a standard table".into());
        }
    };
    if parent.contains_key(key) {
        return Err(format!("`{key}` already exists"));
    }
    parent.insert(key, Item::Table(table));
    Ok(())
}

/// JSON to a TOML table: each key of the object is a key of the table; nested objects become inline tables.
fn to_table(json: &Json) -> Result<Table, String> {
    let Json::Object(map) = json else {
        return Err("a table's value must be an object".into());
    };
    let mut table = Table::new();
    table.decor_mut().set_prefix("\n");
    for (key, value) in map {
        valid_key(key)?;
        table.insert(key, Item::Value(to_value(value, 2)?));
    }
    Ok(table)
}

fn valid_key(key: &str) -> Result<(), String> {
    if key.is_empty() || key.len() > MAX_NAME_LEN || key.chars().any(char::is_control) {
        return Err(format!("`{}` is not a valid key", key.escape_debug()));
    }
    Ok(())
}

/// `depth` is how many arrays and objects enclose this value, counting itself when it is one; scalars do not nest.
fn to_value(json: &Json, depth: usize) -> Result<Value, String> {
    if depth > MAX_VALUE_DEPTH && matches!(json, Json::Array(_) | Json::Object(_)) {
        return Err(format!("the value nests deeper than {MAX_VALUE_DEPTH}"));
    }
    Ok(match json {
        Json::Null => return Err("null has no TOML form; remove the key instead".into()),
        Json::Bool(b) => Value::from(*b),
        Json::String(s) => Value::from(s.as_str()),
        Json::Number(n) => number(n)?,
        Json::Array(items) => {
            let mut array = Array::new();
            for item in items {
                array.push(to_value(item, depth + 1)?);
            }
            Value::Array(array)
        }
        Json::Object(map) => {
            let mut inline = InlineTable::new();
            for (key, value) in map {
                valid_key(key)?;
                inline.insert(key, to_value(value, depth + 1)?);
            }
            Value::InlineTable(inline)
        }
    })
}

fn number(n: &serde_json::Number) -> Result<Value, String> {
    if let Some(i) = n.as_i64() {
        return Ok(Value::from(i));
    }
    if n.is_u64() {
        return Err("that integer is too large".into());
    }
    match n.as_f64() {
        Some(f) if f.is_finite() => Ok(Value::from(f)),
        _ => Err("that number has no TOML form".into()),
    }
}
