//! One TOML configuration file: read tolerantly, written in place.
//!
//! A write edits the document on disk instead of replacing it: a value the
//! write does not change keeps its formatting and comments, and a key the
//! Agent does not know stays where the user put it. The new document is
//! published by an atomic rename.

use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{Map, Value};
use toml_edit::{Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table};

use crate::recovery::AgentConfigRecovery;
use crate::store::object;

const MAX_CONFIG_FILE_BYTES: u64 = 8 * 1024 * 1024;

static NEXT_WRITE: AtomicU64 = AtomicU64::new(0);

/// Who may read a file the Agent writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileAccess {
    Shared,
    /// Owner-only (`0600` on Unix): the file holds credentials.
    Private,
}

pub(crate) struct ConfigFile {
    path: PathBuf,
    access: FileAccess,
    /// Why the file refuses writes: a repair could not keep its original.
    read_only: Mutex<Option<String>>,
    recoveries: Mutex<VecDeque<AgentConfigRecovery>>,
}

/// The recoveries one file keeps, oldest dropped first.
const MAX_RECOVERIES: usize = 20;

/// What a read found.
pub(crate) enum FileRead {
    Missing,
    /// The file is not a TOML document the Agent can read.
    Unusable(String),
    /// The document's tables as JSON objects, which the decoders read.
    Document(Map<String, Value>),
}

impl ConfigFile {
    pub(crate) fn new(path: PathBuf, access: FileAccess) -> Self {
        Self {
            path,
            access,
            read_only: Mutex::new(None),
            recoveries: Mutex::new(VecDeque::new()),
        }
    }

    pub(crate) fn is_read_only(&self) -> bool {
        self.read_only.lock().is_some()
    }

    /// Refuses every later write, until the Agent restarts, with `reason`.
    pub(crate) fn set_read_only(&self, reason: String) {
        *self.read_only.lock() = Some(reason);
    }

    pub(crate) fn record(&self, recovery: AgentConfigRecovery) {
        let mut recoveries = self.recoveries.lock();
        recoveries.push_back(recovery);
        while recoveries.len() > MAX_RECOVERIES {
            recoveries.pop_front();
        }
    }

    pub(crate) fn recoveries(&self) -> Vec<AgentConfigRecovery> {
        self.recoveries.lock().iter().cloned().collect()
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn read(&self) -> Result<FileRead, String> {
        let text = match self.read_text()? {
            Text::Missing => return Ok(FileRead::Missing),
            Text::TooLarge => {
                return Ok(FileRead::Unusable(format!(
                    "the file is larger than {MAX_CONFIG_FILE_BYTES} bytes"
                )))
            }
            Text::Bytes(bytes) => bytes,
        };
        let Ok(text) = String::from_utf8(text) else {
            return Ok(FileRead::Unusable("the file is not UTF-8 text".into()));
        };
        match toml::from_str::<Value>(&text) {
            Ok(Value::Object(document)) => Ok(FileRead::Document(document)),
            Ok(_) => Ok(FileRead::Unusable("the file is not a TOML table".into())),
            Err(error) => Ok(FileRead::Unusable(format!(
                "the file is not valid TOML: {}",
                error.message()
            ))),
        }
    }

    /// Keeps a copy of the file in the sibling `.recovery/` directory before a
    /// repair replaces what the Agent could not use. The file itself stays.
    pub(crate) fn preserve(&self) -> Result<(), String> {
        let parent = parent(&self.path)?;
        let recovery = parent.join(".recovery");
        fs::create_dir_all(&recovery)
            .map_err(|error| format!("create {}: {error}", recovery.display()))?;
        let name = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("config");
        let target = recovery.join(format!(
            "{name}.{}.{}.invalid",
            std::process::id(),
            NEXT_WRITE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::copy(&self.path, &target)
            .map(|_| ())
            .map_err(|error| format!("preserve {}: {error}", self.path.display()))
    }

    /// Writes `value`, an object, over the document on disk: changed values
    /// are replaced, removed ones (`null`) deleted, everything else kept as
    /// written. A file with no document yet lists the keys in the order
    /// `value` declares them.
    pub(crate) fn write<T: Serialize>(&self, value: &T) -> Result<(), String> {
        if let Some(reason) = self.read_only.lock().as_ref() {
            return Err(format!(
                "{} is read-only until the Agent restarts: {reason}",
                self.path.display()
            ));
        }
        let existing = match self.read_text()? {
            Text::Bytes(bytes) => String::from_utf8(bytes).ok(),
            Text::Missing | Text::TooLarge => None,
        };
        let document = existing
            .as_deref()
            .and_then(|text| text.parse::<DocumentMut>().ok());
        let text = match document {
            Some(mut document) => {
                let before = existing
                    .as_deref()
                    .and_then(|text| toml::from_str::<Value>(text).ok());
                merge_table(
                    document.as_table_mut(),
                    before.as_ref().and_then(Value::as_object),
                    &object(value)?,
                );
                document.to_string()
            }
            // The JSON map the merge works on sorts its keys.
            None => match toml::to_string(value) {
                Ok(text) => text,
                Err(_) => {
                    let mut document = DocumentMut::new();
                    merge_table(document.as_table_mut(), None, &object(value)?);
                    document.to_string()
                }
            },
        };
        write_atomic(&self.path, text.as_bytes(), self.access)
    }

    fn read_text(&self) -> Result<Text, String> {
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Text::Missing),
            Err(error) => return Err(format!("read {}: {error}", self.path.display())),
        };
        let mut bytes = Vec::new();
        file.take(MAX_CONFIG_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("read {}: {error}", self.path.display()))?;
        if bytes.len() as u64 > MAX_CONFIG_FILE_BYTES {
            return Ok(Text::TooLarge);
        }
        Ok(Text::Bytes(bytes))
    }
}

enum Text {
    Missing,
    TooLarge,
    Bytes(Vec<u8>),
}

/// Writes `bytes` to `path` through a temporary sibling renamed over it, so a
/// reader sees the old file or the new one, never a part.
pub fn write_atomic(path: &Path, bytes: &[u8], access: FileAccess) -> Result<(), String> {
    fs::create_dir_all(parent(path)?)
        .map_err(|error| format!("create {}: {error}", parent(path).unwrap_or(path).display()))?;
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        NEXT_WRITE.fetch_add(1, Ordering::Relaxed)
    ));
    let temporary = PathBuf::from(temporary);
    let written = create(&temporary, access).and_then(|mut file| {
        file.write_all(bytes)?;
        file.sync_all()
    });
    let published = written.and_then(|()| fs::rename(&temporary, path));
    published.map_err(|error| {
        let _ = fs::remove_file(&temporary);
        format!("write {}: {error}", path.display())
    })
}

fn create(path: &Path, access: FileAccess) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    if access == FileAccess::Private {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(not(unix))]
    let _ = access;
    options.open(path)
}

fn parent(path: &Path) -> Result<&Path, String> {
    path.parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))
}

fn merge_table(table: &mut Table, before: Option<&Map<String, Value>>, after: &Map<String, Value>) {
    for (key, value) in after {
        let previous = before.and_then(|before| before.get(key));
        if value.is_null() {
            table.remove(key);
            continue;
        }
        if previous == Some(value) && table.contains_key(key) {
            continue;
        }
        match (value, table.get_mut(key)) {
            (Value::Object(object), Some(Item::Table(nested))) => {
                merge_table(nested, previous.and_then(Value::as_object), object);
            }
            (_, Some(Item::Value(old))) if !is_array_of_tables(value) => {
                let decor = old.decor().clone();
                let mut new = inline_value(value);
                *new.decor_mut() = decor;
                *old = new;
            }
            _ => {
                table.insert(key, item(value));
            }
        }
    }
}

fn is_array_of_tables(value: &Value) -> bool {
    matches!(value, Value::Array(items) if !items.is_empty() && items.iter().all(Value::is_object))
}

/// `value` as a TOML item: a non-empty object a table, a list of objects an
/// array of tables, anything else an inline value.
fn item(value: &Value) -> Item {
    match value {
        Value::Object(object) if !object.is_empty() => Item::Table(table(object)),
        Value::Array(items) if is_array_of_tables(value) => {
            let mut tables = ArrayOfTables::new();
            for object in items.iter().filter_map(Value::as_object) {
                tables.push(table(object));
            }
            Item::ArrayOfTables(tables)
        }
        _ => Item::Value(inline_value(value)),
    }
}

fn table(object: &Map<String, Value>) -> Table {
    let mut table = Table::new();
    // A table that holds only tables gets no header of its own.
    table.set_implicit(true);
    for (key, value) in object {
        if !value.is_null() {
            table.insert(key, item(value));
        }
    }
    table
}

fn inline_value(value: &Value) -> toml_edit::Value {
    match value {
        Value::Bool(value) => (*value).into(),
        Value::Number(number) => match number.as_i64() {
            Some(integer) => integer.into(),
            None => number.as_f64().unwrap_or_default().into(),
        },
        Value::String(text) => text.as_str().into(),
        Value::Array(items) => {
            let mut array = Array::new();
            for item in items.iter().filter(|item| !item.is_null()) {
                array.push(inline_value(item));
            }
            toml_edit::Value::Array(array)
        }
        Value::Object(object) => {
            let mut table = InlineTable::new();
            for (key, value) in object.iter().filter(|(_, value)| !value.is_null()) {
                table.insert(key, inline_value(value));
            }
            toml_edit::Value::InlineTable(table)
        }
        // A TOML document has no null: `merge_table` and `table` remove the
        // key instead of writing one.
        Value::Null => "".into(),
    }
}

#[cfg(test)]
mod tests;
