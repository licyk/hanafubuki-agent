//! `plugins/install.json`: what the store knows of each installed package
//! beyond its files, the version installed, where it came from and the
//! content hash taken at install, and the permissions the user granted its
//! code. It is the Agent's own state, written as a
//! whole (a temporary file renamed over it) and read tolerantly: unknown
//! fields are ignored, each field falls back on its own, an entry whose
//! key is not a plugin id is skipped, and a file that does not read at all
//! is quarantined and taken as empty. The packages themselves are the
//! cache's directories; a package without an entry is still installed.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use hanafubuki_agent_foundation::persistence::quarantine_file;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::{ManifestFormat, PackagePermissions, PackageSource, PluginId};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallRecord {
    pub version: Option<String>,
    pub format: Option<ManifestFormat>,
    pub source: Option<PackageSource>,
    /// [`crate::content_hash`] of the package as installed.
    pub content_hash: Option<String>,
    /// What the user granted its code; kept across updates, so an update
    /// that asks for more waits for a new grant.
    pub granted_permissions: Option<PackagePermissions>,
}

pub(crate) type InstallRecords = BTreeMap<PluginId, InstallRecord>;

#[derive(Serialize)]
struct RecordFile {
    packages: BTreeMap<String, InstallRecord>,
}

pub(crate) fn read(path: &Path) -> InstallRecords {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return InstallRecords::new(),
        Err(error) => {
            log::warn!(
                "the install record {} cannot be read: {error}",
                path.display()
            );
            return InstallRecords::new();
        }
    };
    let packages = match serde_json::from_slice::<Value>(&bytes) {
        Ok(Value::Object(mut file)) => match file.remove("packages") {
            Some(Value::Object(packages)) => packages,
            _ => Map::new(),
        },
        _ => {
            log::warn!(
                "the install record {} does not read and is quarantined",
                path.display()
            );
            if let Err(error) = quarantine_file(path) {
                log::warn!(
                    "the install record {} was not quarantined: {error}",
                    path.display()
                );
            }
            return InstallRecords::new();
        }
    };
    packages
        .into_iter()
        .filter_map(|(key, entry)| match PluginId::parse(&key) {
            Ok(id) => Some((id, entry_of(&entry))),
            Err(error) => {
                log::warn!("the install record's entry {key} is ignored: {error}");
                None
            }
        })
        .collect()
}

fn entry_of(value: &Value) -> InstallRecord {
    let field = |key: &str| value.get(key).cloned();
    let string = |key: &str| field(key).and_then(|value| value.as_str().map(str::to_string));
    InstallRecord {
        version: string("version"),
        format: field("format").and_then(|format| serde_json::from_value(format).ok()),
        source: field("source").and_then(|source| serde_json::from_value(source).ok()),
        content_hash: string("contentHash"),
        granted_permissions: field("grantedPermissions")
            .and_then(|permissions| serde_json::from_value(permissions).ok()),
    }
}

pub(crate) fn write(path: &Path, records: &InstallRecords) -> Result<(), String> {
    let file = RecordFile {
        packages: records
            .iter()
            .map(|(id, record)| (id.key(), record.clone()))
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
