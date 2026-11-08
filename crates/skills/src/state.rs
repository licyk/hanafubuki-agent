//! Which Skills the user turned off.
//!
//! A discovered Skill is on. That is the only default that matches what people
//! expect after installing one, and it means the state file records exceptions
//! rather than the catalog — an id that no longer resolves is simply forgotten
//! the next time the file is written.

use std::collections::BTreeSet;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use hanafubuki_agent_foundation::persistence::quarantine_file;

use crate::fs_util::atomic_write;
use crate::model::{SkillCatalogEntry, SkillError, SkillId};

const STATE_VERSION: u32 = 3;
const MAX_STATE_BYTES: u64 = 256 * 1024;
const MAX_DISABLED: usize = 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SkillStateDocument {
    pub(crate) version: u32,
    pub(crate) revision: u64,
    /// Skills the user turned off. Everything else discovered is on.
    pub(crate) disabled: Vec<SkillId>,
}

impl Default for SkillStateDocument {
    fn default() -> Self {
        Self {
            version: STATE_VERSION,
            revision: 1,
            disabled: Vec::new(),
        }
    }
}

impl SkillStateDocument {
    pub(crate) fn is_disabled(&self, id: &SkillId) -> bool {
        self.disabled.contains(id)
    }

    /// Returns whether anything changed.
    pub(crate) fn set_disabled(&mut self, id: &SkillId, disabled: bool) -> bool {
        let before = self.disabled.len();
        if disabled {
            if !self.disabled.contains(id) {
                self.disabled.push(id.clone());
                self.disabled.sort();
            }
        } else {
            self.disabled.retain(|entry| entry != id);
        }
        before != self.disabled.len()
    }
}

/// Load the state file, repairing or resetting anything that is not the current
/// shape. A corrupt selection is never a reason to refuse to start.
pub(crate) fn load_state(path: &Path) -> Result<SkillStateDocument, SkillError> {
    if !path.exists() {
        return Ok(SkillStateDocument::default());
    }
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| SkillError::Io(format!("inspect Skill state: {error}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > MAX_STATE_BYTES
    {
        return reset(path, "state must be a bounded regular file");
    }
    let bytes = std::fs::read(path)
        .map_err(|error| SkillError::Io(format!("read Skill state: {error}")))?;
    let Ok(raw) = serde_json::from_slice::<Value>(&bytes) else {
        return reset(path, "state is not JSON");
    };
    let Some(object) = raw.as_object() else {
        return reset(path, "state root is not an object");
    };
    let revision = object
        .get("revision")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .max(1);
    let mut disabled = object
        .get("disabled")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                // An id this build no longer understands is dropped, not fatal.
                .filter_map(|value| SkillId::from_untrusted(value).ok())
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default()
        .into_iter()
        .collect::<Vec<_>>();
    if disabled.len() > MAX_DISABLED {
        return reset(path, "state names more Skills than are supported");
    }
    disabled.sort();
    let state = SkillStateDocument {
        version: STATE_VERSION,
        revision,
        disabled,
    };
    let canonical = serde_json::to_value(&state)
        .map_err(|error| SkillError::Io(format!("encode Skill state: {error}")))?;
    if canonical != raw {
        quarantine_file(path).map_err(SkillError::Io)?;
        save_state(path, &state)?;
        log::warn!("repaired incompatible Skill state path={}", path.display());
    }
    Ok(state)
}

fn reset(path: &Path, reason: &str) -> Result<SkillStateDocument, SkillError> {
    quarantine_file(path).map_err(SkillError::Io)?;
    let state = SkillStateDocument::default();
    save_state(path, &state)?;
    log::warn!(
        "reset incompatible Skill state path={} reason={reason}",
        path.display()
    );
    Ok(state)
}

pub(crate) fn save_state(path: &Path, state: &SkillStateDocument) -> Result<(), SkillError> {
    let bytes = serde_json::to_vec_pretty(state)
        .map_err(|error| SkillError::Io(format!("encode Skill state: {error}")))?;
    if bytes.len() as u64 > MAX_STATE_BYTES {
        return Err(SkillError::LimitExceeded(
            "Skill state exceeds its byte limit".to_string(),
        ));
    }
    atomic_write(path, &bytes, "Skill state")
}

pub(crate) fn bump_revision(state: &mut SkillStateDocument) {
    state.revision = state.revision.saturating_add(1);
}

/// A revision that changes whenever the catalog or the selection does, so
/// callers can detect that what they were looking at has moved on.
pub(crate) fn catalog_revision(entries: &[SkillCatalogEntry], state_revision: u64) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(state_revision.to_be_bytes());
    for entry in entries {
        hasher.update(entry.id.as_str().as_bytes());
        hasher.update([0]);
        hasher.update(entry.name.as_bytes());
        hasher.update([0]);
        hasher.update([u8::from(entry.enabled)]);
        hasher.update(entry.resources.len().to_be_bytes());
    }
    let digest = hasher.finalize();
    let mut prefix = [0u8; 8];
    prefix.copy_from_slice(&digest[..8]);
    u64::from_be_bytes(prefix).max(1)
}

#[cfg(test)]
mod tests;
