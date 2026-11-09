//! The MCP server settings file (`mcp.toml`), credentials included.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use hanafubuki_agent_mcp::{
    repair_settings, McpCredentialKey, McpError, McpSettings, McpSettingsRepair,
};
use parking_lot::Mutex;
use serde_json::Value;

use crate::file::{ConfigFile, FileAccess};
use crate::store::load;

/// The repairs one store keeps for its settings surface.
const MAX_RETAINED_REPAIRS: usize = 256;

/// The MCP settings, revision-gated like the Provider registry.
///
/// A read repairs what it can (`hanafubuki_agent_mcp::repair_settings`): the
/// original is kept in `.recovery/`, the repaired values written, and the
/// repairs kept for as long as this store lives so the settings surface can
/// say what changed, since the next read finds a clean file.
#[derive(Clone)]
pub struct McpSettingsStore {
    file: Arc<ConfigFile>,
    access: Arc<Mutex<()>>,
    repairs: Arc<Mutex<Vec<McpSettingsRepair>>>,
}

impl McpSettingsStore {
    pub fn new(path: PathBuf) -> Self {
        Self {
            file: Arc::new(ConfigFile::new(path, FileAccess::Private)),
            access: Arc::new(Mutex::new(())),
            repairs: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn path(&self) -> &Path {
        self.file.path()
    }

    /// The repairs this store's reads made, oldest first.
    pub fn repairs(&self) -> Vec<McpSettingsRepair> {
        self.repairs.lock().clone()
    }

    pub fn load(&self) -> Result<McpSettings, McpError> {
        let _guard = self.access.lock();
        self.load_locked()
    }

    pub fn update<F>(&self, expected_revision: u64, update: F) -> Result<McpSettings, McpError>
    where
        F: FnOnce(&mut McpSettings) -> Result<(), McpError>,
    {
        let _guard = self.access.lock();
        let mut settings = self.load_locked()?;
        if settings.revision != expected_revision {
            return Err(McpError::StaleGeneration);
        }
        let before = settings.credentials.keys().cloned().collect::<Vec<_>>();
        update(&mut settings)?;
        settings.revision = settings
            .revision
            .checked_add(1)
            .ok_or(McpError::LimitExceeded)?;
        settings.validate()?;
        let removed = before
            .iter()
            .filter(|key| !settings.credentials.contains_key(*key));
        self.write_locked(&settings, removed)?;
        Ok(settings)
    }

    /// Writes `settings` whole, for a store that has none yet.
    pub(crate) fn write(&self, settings: &McpSettings) -> Result<(), McpError> {
        let _guard = self.access.lock();
        self.write_locked(settings, std::iter::empty())
    }

    /// The write keeps every key the file has that `settings` does not name,
    /// so a removed credential is written as `null` to leave the file.
    fn write_locked<'a>(
        &self,
        settings: &McpSettings,
        removed_credentials: impl Iterator<Item = &'a McpCredentialKey>,
    ) -> Result<(), McpError> {
        let mut document = serde_json::to_value(settings)
            .map_err(|error| McpError::Unavailable(error.to_string()))?;
        if let Some(credentials) = document
            .get_mut("credentials")
            .and_then(Value::as_object_mut)
        {
            for key in removed_credentials {
                credentials.insert(key.as_str().to_string(), Value::Null);
            }
        }
        self.file.write(&document).map_err(McpError::Unavailable)
    }

    pub(crate) fn file(&self) -> &ConfigFile {
        &self.file
    }

    fn load_locked(&self) -> Result<McpSettings, McpError> {
        let mut repairs = Vec::new();
        let loaded = load(
            &self.file,
            McpSettings::default,
            |document| {
                let (settings, found) = repair_settings(Value::Object(document.clone()));
                let messages = found.iter().map(describe).collect();
                repairs = found;
                (settings, messages)
            },
            true,
        )
        .map_err(McpError::Unavailable)?;
        if !loaded.recorded {
            return Ok(loaded.value);
        }
        if let Some(reason) = loaded.unusable {
            repairs.push(McpSettingsRepair::reset(&reason));
        }
        if !repairs.is_empty() {
            let mut retained = self.repairs.lock();
            retained.extend(repairs);
            let excess = retained.len().saturating_sub(MAX_RETAINED_REPAIRS);
            retained.drain(..excess);
        }
        Ok(loaded.value)
    }
}

fn describe(repair: &McpSettingsRepair) -> String {
    format!(
        "{:?} {} {}: {}",
        repair.kind,
        repair.subject.as_deref().unwrap_or("-"),
        repair.field.as_deref().unwrap_or("-"),
        repair.detail
    )
}
