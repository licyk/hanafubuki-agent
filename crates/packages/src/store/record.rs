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
//! store then quarantines it. Reading writes nothing. The packages
//! themselves are the cache's directories; a package without an entry is
//! still installed.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use hanafubuki_agent_foundation::persistence::quarantine_file;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::{ManifestFormat, PackagePermissions, PackageSource, PluginId};

const INSTALLED_FILE: &str = "install.json";
const LINKED_FILE: &str = "linked.json";

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
}

impl Records {
    /// The records under `root` (the data directory's `plugins/`). An
    /// `install.json` entry of source `linked`, which an earlier build of
    /// links wrote, is read as a linked one unless `linked.json` has the
    /// id; the store's next write moves it there.
    pub(crate) fn read(root: &Path) -> Self {
        let mut records = Self::default();
        let installed = records.entries(&root.join(INSTALLED_FILE));
        let linked = records.entries(&root.join(LINKED_FILE));
        records.linked = linked
            .into_iter()
            .filter_map(|(id, entry)| match linked_of(&entry) {
                Some(record) => Some((id, record)),
                None => {
                    log::warn!("the linked record's entry {id} names no directory and is ignored");
                    None
                }
            })
            .collect();
        for (id, entry) in installed {
            let record = entry_of(&entry);
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
                return BTreeMap::new();
            }
        };
        let packages = match serde_json::from_slice::<Value>(&bytes) {
            Ok(Value::Object(mut file)) => match file.remove("packages") {
                Some(Value::Object(packages)) => packages,
                _ => Map::new(),
            },
            _ => {
                self.unreadable.push(path.to_path_buf());
                return BTreeMap::new();
            }
        };
        packages
            .into_iter()
            .filter_map(|(key, entry)| match PluginId::parse(&key) {
                Ok(id) => Some((id, entry)),
                Err(error) => {
                    log::warn!("{}'s entry {key} is ignored: {error}", path.display());
                    None
                }
            })
            .collect()
    }
}

fn entry_of(value: &Value) -> InstallRecord {
    InstallRecord {
        version: string(value, "version"),
        format: field(value, "format"),
        source: field(value, "source"),
        content_hash: string(value, "contentHash"),
        granted_permissions: field(value, "grantedPermissions"),
    }
}

fn linked_of(value: &Value) -> Option<LinkedRecord> {
    Some(LinkedRecord {
        path: PathBuf::from(string(value, "path").filter(|path| !path.is_empty())?),
        version: string(value, "version"),
        format: field(value, "format"),
        granted_permissions: field(value, "grantedPermissions"),
    })
}

fn string(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::to_string)
}

fn field<T: serde::de::DeserializeOwned>(value: &Value, key: &str) -> Option<T> {
    serde_json::from_value(value.get(key)?.clone()).ok()
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
