//! What the store knows of each package beyond its files, in two files of
//! the Agent's own state:
//!
//! - `plugins/install.json`, the packages in the cache: the version
//!   installed, where it came from and the content hash taken at install,
//!   and the permissions the user granted its code;
//! - `plugins/linked.json`, the packages linked for development: the
//!   directory that is the package, and the same grant. Builds that predate
//!   links never read or write it, so their rewrite of `install.json`
//!   cannot drop a link.
//!
//! Each is written as a whole (a temporary file renamed over it) and read
//! tolerantly: unknown fields are ignored, each field falls back on its own
//! (a source of a kind this build does not know is none), an entry whose
//! key is not a plugin id, or a linked entry without a directory, is
//! skipped, and a file that does not read at all is taken as empty; the
//! store then quarantines it. Reading writes nothing. A write never loses
//! what it could not read: a file that cannot be opened (other than a
//! missing one) is not written over at all, a file that does not read is
//! quarantined first (and the write refused if that fails), and a file
//! that read with an entry or a field left out is copied to `.recovery`
//! first. The packages themselves are the cache's directories; a package
//! without an entry is still installed.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use hanafubuki_agent_foundation::persistence::quarantine_file;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::{ManifestFormat, PackagePermissions, PackageSource, PluginId};

const INSTALLED_FILE: &str = "install.json";
const LINKED_FILE: &str = "linked.json";
/// Where a file's copy is kept, beside the quarantined ones.
const RECOVERY_DIR: &str = ".recovery";

static NEXT_COPY: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallRecord {
    pub version: Option<String>,
    pub format: Option<ManifestFormat>,
    /// Where it came from; a linked package's directory (never written to
    /// `install.json`).
    pub source: Option<PackageSource>,
    /// [`crate::content_hash`] of the package as installed.
    pub content_hash: Option<String>,
    /// What the user granted its code; kept across updates, so an update
    /// that asks for more waits for a new grant.
    pub granted_permissions: Option<PackagePermissions>,
}

impl InstallRecord {
    /// The directory the package is linked to, when it is linked.
    pub fn linked(&self) -> Option<&Path> {
        match &self.source {
            Some(PackageSource::Linked { path }) => Some(path),
            _ => None,
        }
    }
}

/// A `linked.json` entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LinkedRecord {
    /// The linked directory, as a person reads it.
    pub(crate) path: PathBuf,
    /// Its manifest's version and format when it was linked.
    pub(crate) version: Option<String>,
    pub(crate) format: Option<ManifestFormat>,
    pub(crate) granted_permissions: Option<PackagePermissions>,
}

impl LinkedRecord {
    /// The record an [`crate::InstalledPackage`] carries: source `linked`,
    /// no content hash.
    pub(crate) fn install_record(&self) -> InstallRecord {
        InstallRecord {
            version: self.version.clone(),
            format: self.format,
            source: Some(PackageSource::Linked {
                path: self.path.clone(),
            }),
            content_hash: None,
            granted_permissions: self.granted_permissions.clone(),
        }
    }
}

/// Both files, read. An id may be in both (an older build's install, or
/// its `"source": null` rewrite, under a linked id): the cache decides
/// which one is the package.
#[derive(Debug, Default)]
pub(crate) struct Records {
    pub(crate) installed: BTreeMap<PluginId, InstallRecord>,
    pub(crate) linked: BTreeMap<PluginId, LinkedRecord>,
    /// The files that did not read at all, taken as empty.
    unreadable: Vec<PathBuf>,
    /// The files that read with an entry or a field left out.
    partial: Vec<PathBuf>,
    /// Why a file that exists could not be opened.
    failed: Vec<String>,
}

impl Records {
    /// The records under `root` (the data directory's `plugins/`). An
    /// `install.json` entry of source `linked`, which an earlier build of
    /// links wrote, is read as a linked one unless `linked.json` has the
    /// id; the store's next write moves it there.
    pub(crate) fn read(root: &Path) -> Self {
        let mut records = Self::default();
        let installed_file = root.join(INSTALLED_FILE);
        let linked_file = root.join(LINKED_FILE);
        let installed = records.entries(&installed_file);
        let linked = records.entries(&linked_file);
        let mut linked_lossy = false;
        records.linked = linked
            .into_iter()
            .filter_map(|(id, entry)| {
                let (record, lossy) = linked_of(&entry);
                linked_lossy |= lossy;
                if record.is_none() {
                    log::warn!("the linked record's entry {id} names no directory and is ignored");
                }
                Some((id, record?))
            })
            .collect();
        records.note_partial(linked_lossy, &linked_file);
        let mut installed_lossy = false;
        for (id, entry) in installed {
            let (record, lossy) = entry_of(&entry);
            installed_lossy |= lossy;
            match record.linked() {
                Some(path) => {
                    let linked = LinkedRecord {
                        path: path.to_path_buf(),
                        version: record.version,
                        format: record.format,
                        granted_permissions: record.granted_permissions,
                    };
                    records.linked.entry(id).or_insert(linked);
                }
                None => {
                    records.installed.insert(id, record);
                }
            }
        }
        records.note_partial(installed_lossy, &installed_file);
        records
    }

    /// Moves aside each file that did not read, so the next write does not
    /// replace what a person may still recover.
    pub(crate) fn quarantine_unreadable(&self) {
        for path in &self.unreadable {
            log::warn!("{} does not read and is quarantined", path.display());
            if let Err(error) = quarantine_file(path) {
                log::warn!("{} was not quarantined: {error}", path.display());
            }
        }
    }

    /// Makes both files safe to write over, or says why they are not: a
    /// file that could not be opened is refused, one that did not read is
    /// quarantined and one that read in part is copied, each before
    /// anything is written.
    pub(crate) fn prepare_write(&self) -> Result<(), String> {
        if !self.failed.is_empty() {
            return Err(format!(
                "{}; nothing is written over it",
                self.failed.join("; ")
            ));
        }
        for path in &self.unreadable {
            quarantine_file(path).map_err(|error| {
                format!(
                    "{} does not read and was not quarantined: {error}",
                    path.display()
                )
            })?;
        }
        for path in &self.partial {
            keep_copy(path)?;
        }
        Ok(())
    }

    fn note_partial(&mut self, lossy: bool, path: &Path) {
        if lossy && !self.partial.iter().any(|known| known == path) {
            self.partial.push(path.to_path_buf());
        }
    }

    /// Writes both files under `root`: `linked.json` first, so a link
    /// moved out of `install.json` is never in neither; `linked.json` only
    /// when there is a link or the file exists.
    pub(crate) fn write(&self, root: &Path) -> Result<(), String> {
        let linked = root.join(LINKED_FILE);
        if !self.linked.is_empty() || linked.exists() {
            write_file(&linked, &self.linked)?;
        }
        write_file(&root.join(INSTALLED_FILE), &self.installed)
    }

    /// The entries of the file at `path` by id; a file that does not read
    /// is noted.
    fn entries(&mut self, path: &Path) -> BTreeMap<PluginId, Value> {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return BTreeMap::new(),
            Err(error) => {
                log::warn!("{} cannot be read: {error}", path.display());
                self.failed
                    .push(format!("{} cannot be read: {error}", path.display()));
                return BTreeMap::new();
            }
        };
        let packages = match serde_json::from_slice::<Value>(&bytes) {
            Ok(Value::Object(mut file)) => match file.remove("packages") {
                Some(Value::Object(packages)) => packages,
                None => Map::new(),
                Some(_) => {
                    self.unreadable.push(path.to_path_buf());
                    return BTreeMap::new();
                }
            },
            _ => {
                self.unreadable.push(path.to_path_buf());
                return BTreeMap::new();
            }
        };
        let mut lossy = false;
        let entries = packages
            .into_iter()
            .filter_map(|(key, entry)| match PluginId::parse(&key) {
                Ok(id) => Some((id, entry)),
                Err(error) => {
                    log::warn!("{}'s entry {key} is ignored: {error}", path.display());
                    lossy = true;
                    None
                }
            })
            .collect();
        self.note_partial(lossy, path);
        entries
    }
}

/// The record of an `install.json` entry, and whether a field of it was
/// left out.
fn entry_of(value: &Value) -> (InstallRecord, bool) {
    let mut lossy = false;
    let record = InstallRecord {
        version: string(value, "version", &mut lossy),
        format: field(value, "format", &mut lossy),
        source: field(value, "source", &mut lossy),
        content_hash: string(value, "contentHash", &mut lossy),
        granted_permissions: field(value, "grantedPermissions", &mut lossy),
    };
    (record, lossy || !value.is_object())
}

/// The record of a `linked.json` entry, `None` without a directory, and
/// whether something of it was left out.
fn linked_of(value: &Value) -> (Option<LinkedRecord>, bool) {
    let mut lossy = false;
    let path = string(value, "path", &mut lossy).filter(|path| !path.is_empty());
    let version = string(value, "version", &mut lossy);
    let format = field(value, "format", &mut lossy);
    let granted_permissions = field(value, "grantedPermissions", &mut lossy);
    match path {
        Some(path) => {
            let record = LinkedRecord {
                path: PathBuf::from(path),
                version,
                format,
                granted_permissions,
            };
            (Some(record), lossy)
        }
        None => (None, true),
    }
}

/// The text of `key`; a value there that is not text is left out, noted
/// in `lossy`.
fn string(value: &Value, key: &str, lossy: &mut bool) -> Option<String> {
    match value.get(key)? {
        Value::String(text) => Some(text.clone()),
        Value::Null => None,
        _ => {
            *lossy = true;
            None
        }
    }
}

/// `key` decoded; a value there that does not decode is left out, noted
/// in `lossy`.
fn field<T: serde::de::DeserializeOwned>(value: &Value, key: &str, lossy: &mut bool) -> Option<T> {
    match value.get(key)? {
        Value::Null => None,
        present => serde_json::from_value(present.clone())
            .map_err(|_| *lossy = true)
            .ok(),
    }
}

/// Copies the file at `path` to `.recovery` beside it, before a write
/// replaces what it could not read.
fn keep_copy(path: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    let recovery = parent.join(RECOVERY_DIR);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("records");
    let target = recovery.join(format!(
        "{name}.{}.{}.partial",
        std::process::id(),
        NEXT_COPY.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&recovery)
        .and_then(|()| fs::copy(path, &target).map(drop))
        .map_err(|error| {
            format!(
                "{} read in part and could not be copied to {}: {error}",
                path.display(),
                target.display()
            )
        })
}

#[derive(Serialize)]
struct RecordFile<'a, T> {
    packages: BTreeMap<String, &'a T>,
}

fn write_file<T: Serialize>(path: &Path, records: &BTreeMap<PluginId, T>) -> Result<(), String> {
    let file = RecordFile {
        packages: records
            .iter()
            .map(|(id, record)| (id.key(), record))
            .collect(),
    };
    let bytes = serde_json::to_vec_pretty(&file).map_err(|error| error.to_string())?;
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    (|| -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = fs::File::create(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })()
    .map_err(|error| format!("write {}: {error}", path.display()))
}
