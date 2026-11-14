//! `update-check.json` in the data directory: when the update check last
//! asked, the latest stable release it found, and the version the update
//! prompt was told to skip.
//!
//! It is read as `tui-state.json` is: an unknown key is kept as it was, a
//! field that does not read is absent and the file is moved aside into
//! `.recovery/` before it is next written; a file that is not a JSON object
//! is moved aside at once. A file that exists but cannot be read is never
//! written, and the check is then not made. Each write reads the file again
//! first, so what another instance wrote meanwhile is kept.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;

use hanafubuki_agent::{quarantine_file, write_atomic, FileAccess};
use hanafubuki_agent_update::Version;
use serde_json::{Map, Value};

pub(crate) const UPDATE_CHECK_FILE: &str = "update-check.json";
/// How long an answer is used before the check asks again.
pub(crate) const CHECK_INTERVAL: Duration = Duration::from_secs(20 * 60 * 60);
const MAX_CACHE_BYTES: u64 = 64 * 1024;

const LATEST_VERSION: &str = "latestVersion";
const CHECKED_AT_MS: &str = "checkedAtMs";
const DISMISSED_VERSION: &str = "dismissedVersion";

#[derive(Debug, Default)]
pub(crate) struct UpdateCache {
    /// The file as read, its other keys kept as they were.
    document: Map<String, Value>,
    /// The latest stable release a check found.
    pub(crate) latest_version: Option<Version>,
    /// When a check last got an answer (milliseconds since the epoch).
    pub(crate) checked_at_ms: Option<u64>,
    /// The version "Skip until next version" chose.
    pub(crate) dismissed_version: Option<Version>,
    /// A field did not read: the file is moved aside before a write.
    lossy: bool,
    /// The file exists and was not read: it is never written.
    unreadable: bool,
}

impl UpdateCache {
    /// The cache in `data_dir`; empty when there is none, or none that
    /// reads.
    pub(crate) fn load(data_dir: &Path) -> Self {
        let path = cache_path(data_dir);
        let bytes = match read_bounded(&path) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return Self::default(),
            Err(Unread::Unreadable) => return Self::unreadable(),
            Err(Unread::TooLarge) => return set_aside(&path),
        };
        let Ok(Value::Object(document)) = serde_json::from_slice(&bytes) else {
            return set_aside(&path);
        };
        decode(document)
    }

    /// Whether the check asks now, at `now_ms`: never answered, answered
    /// [`CHECK_INTERVAL`] ago or more, or dated after now (a clock set
    /// back). Never for a file that cannot be read.
    pub(crate) fn is_due(&self, now_ms: u64) -> bool {
        if self.unreadable {
            return false;
        }
        let interval = CHECK_INTERVAL.as_millis() as u64;
        self.checked_at_ms
            .is_none_or(|checked| checked > now_ms || now_ms - checked >= interval)
    }

    /// Reads the cache in `data_dir` again, lets `change` set its fields,
    /// and writes it.
    pub(crate) fn update(data_dir: &Path, change: impl FnOnce(&mut Self)) -> Result<(), String> {
        let path = cache_path(data_dir);
        let mut cache = Self::load(data_dir);
        if cache.unreadable {
            return Err(format!(
                "{UPDATE_CHECK_FILE} was not read, so it is not written"
            ));
        }
        change(&mut cache);
        if cache.lossy {
            quarantine_file(&path)?;
        }
        let bytes = serde_json::to_vec_pretty(&cache.encode())
            .map_err(|error| format!("{UPDATE_CHECK_FILE} was not encoded: {error}"))?;
        write_atomic(&path, &bytes, FileAccess::Private)
    }

    fn unreadable() -> Self {
        Self {
            unreadable: true,
            ..Self::default()
        }
    }

    /// The document with the fields set, the others' keys as they were.
    fn encode(mut self) -> Map<String, Value> {
        let fields = [
            (LATEST_VERSION, self.latest_version.map(version_value)),
            (CHECKED_AT_MS, self.checked_at_ms.map(Value::from)),
            (DISMISSED_VERSION, self.dismissed_version.map(version_value)),
        ];
        for (key, value) in fields {
            match value {
                Some(value) => self.document.insert(key.to_string(), value),
                None => self.document.remove(key),
            };
        }
        self.document
    }
}

fn cache_path(data_dir: &Path) -> PathBuf {
    data_dir.join(UPDATE_CHECK_FILE)
}

/// A file that is not a cache, moved aside: the cache is empty, or, when
/// it could not be moved, unreadable.
fn set_aside(path: &Path) -> UpdateCache {
    match quarantine_file(path) {
        Ok(_) => UpdateCache::default(),
        Err(_) => UpdateCache::unreadable(),
    }
}

enum Unread {
    TooLarge,
    Unreadable,
}

/// The file's bytes; `None` when there is no file.
fn read_bounded(path: &Path) -> Result<Option<Vec<u8>>, Unread> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(Unread::Unreadable),
    };
    if !metadata.is_file() {
        return Err(Unread::Unreadable);
    }
    if metadata.len() > MAX_CACHE_BYTES {
        return Err(Unread::TooLarge);
    }
    fs::read(path).map(Some).map_err(|_| Unread::Unreadable)
}

fn decode(document: Map<String, Value>) -> UpdateCache {
    let mut lossy = false;
    let latest_version = field(&document, LATEST_VERSION, &mut lossy, |value| {
        stable_version(value)
    });
    let checked_at_ms = field(&document, CHECKED_AT_MS, &mut lossy, Value::as_u64);
    let dismissed_version = field(&document, DISMISSED_VERSION, &mut lossy, |value| {
        value.as_str()?.parse::<Version>().ok()
    });
    UpdateCache {
        document,
        latest_version,
        checked_at_ms,
        dismissed_version,
        lossy,
        unreadable: false,
    }
}

/// The field `key` read by `read`; one present that does not read is
/// absent, and marks the file `lossy`.
fn field<T>(
    document: &Map<String, Value>,
    key: &str,
    lossy: &mut bool,
    read: impl FnOnce(&Value) -> Option<T>,
) -> Option<T> {
    let value = document.get(key)?;
    let read = read(value);
    *lossy |= read.is_none();
    read
}

fn stable_version(value: &Value) -> Option<Version> {
    let version = value.as_str()?.parse::<Version>().ok()?;
    version.pre.is_empty().then_some(version)
}

fn version_value(version: Version) -> Value {
    Value::String(version.to_string())
}

#[cfg(test)]
mod tests;
