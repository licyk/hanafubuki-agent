//! The hooks' trust and enablement, keyed by hook: whether each is turned
//! off, and the hash of the definition the user trusted (with, for a
//! plugin's hook, its package's content hash). It is the Agent's
//! own state, a file in the data directory written as a whole (a temporary
//! file renamed over it), and read tolerantly: unknown fields are ignored,
//! an entry that does not read is skipped, and a file that does not read
//! at all is quarantined and taken as empty (every hook untrusted).

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use hanafubuki_agent_foundation::persistence::quarantine_file;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// One hook's state.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HookState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) trusted_hash: Option<String>,
    /// For a plugin's hook, the content hash of its package when trusted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) trusted_content_hash: Option<String>,
}

pub(crate) type HookStates = BTreeMap<String, HookState>;

/// The state file; one writer at a time per process.
pub(crate) struct HookStateStore {
    path: PathBuf,
    write: Mutex<()>,
}

impl HookStateStore {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path,
            write: Mutex::new(()),
        }
    }

    pub(crate) fn load(&self) -> HookStates {
        read(&self.path)
    }

    /// Applies `change` to the state on disk.
    pub(crate) fn update(&self, change: impl FnOnce(&mut HookStates)) -> Result<(), String> {
        let _write = self.write.lock();
        let mut states = read(&self.path);
        change(&mut states);
        write(&self.path, &states)
    }
}

#[derive(Serialize)]
struct StateFile<'a> {
    hooks: &'a HookStates,
}

fn read(path: &Path) -> HookStates {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return HookStates::new(),
        Err(error) => {
            log::warn!(
                "the hooks' state {} cannot be read: {error}",
                path.display()
            );
            return HookStates::new();
        }
    };
    let hooks = match serde_json::from_slice::<Value>(&bytes) {
        Ok(Value::Object(mut file)) => match file.remove("hooks") {
            Some(Value::Object(hooks)) => hooks,
            _ => Map::new(),
        },
        _ => {
            log::warn!(
                "the hooks' state {} does not read and is quarantined; every hook is untrusted",
                path.display()
            );
            if let Err(error) = quarantine_file(path) {
                log::warn!(
                    "the hooks' state {} was not quarantined: {error}",
                    path.display()
                );
            }
            return HookStates::new();
        }
    };
    hooks
        .into_iter()
        .filter_map(|(key, state)| match HookState::deserialize(state) {
            Ok(state) => Some((key, state)),
            Err(error) => {
                log::warn!("the hooks' state of {key} is ignored: {error}");
                None
            }
        })
        .collect()
}

fn write(path: &Path, states: &HookStates) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(&StateFile { hooks: states })
        .map_err(|error| error.to_string())?;
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
