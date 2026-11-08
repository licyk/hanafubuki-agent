//! The Agent's configuration directory and the settings read from it.

use std::path::Path;
use std::sync::Arc;

use hanafubuki_agent_mcp::McpSettings;
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::fields::unknown_keys;
use crate::file::{ConfigFile, FileAccess, FileRead};
use crate::providers::{decode_registry, validate_agent_provider_registry};
use crate::settings::{decode_settings, validate_disabled_plugins};
use crate::{AgentProviderRegistrySettings, AgentSettings, AgentSettingsPatch, McpSettingsStore};

/// The settings, without credentials.
pub const AGENT_SETTINGS_FILE: &str = "config.toml";
/// The Provider registry, API tokens included (owner-only).
pub const AGENT_PROVIDERS_FILE: &str = "providers.toml";
/// The MCP servers, credential values included (owner-only).
pub const AGENT_MCP_FILE: &str = "mcp.toml";
/// The files that hold credentials, which a diagnostic bundle leaves out.
pub const AGENT_CREDENTIAL_FILES: [&str; 2] = [AGENT_PROVIDERS_FILE, AGENT_MCP_FILE];

/// The Agent's configuration in one directory: the settings, the Provider
/// registry and the MCP servers, each its own file with its own lock, so a
/// Provider edit and a permission-mode edit never wait on or conflict with
/// each other. Each file is read once, on first use; a write reads it again
/// first, so an edit made by hand meanwhile is kept.
#[derive(Clone)]
pub struct AgentConfig {
    inner: Arc<AgentConfigInner>,
}

struct AgentConfigInner {
    settings_file: ConfigFile,
    providers_file: ConfigFile,
    settings: Mutex<Option<AgentSettings>>,
    providers: Mutex<Option<AgentProviderRegistrySettings>>,
    mcp: McpSettingsStore,
}

impl AgentConfig {
    /// The configuration in `config_dir`; nothing is read until it is used.
    pub fn new(config_dir: &Path) -> Self {
        Self {
            inner: Arc::new(AgentConfigInner {
                settings_file: ConfigFile::new(
                    config_dir.join(AGENT_SETTINGS_FILE),
                    FileAccess::Shared,
                ),
                providers_file: ConfigFile::new(
                    config_dir.join(AGENT_PROVIDERS_FILE),
                    FileAccess::Private,
                ),
                settings: Mutex::new(None),
                providers: Mutex::new(None),
                mcp: McpSettingsStore::new(config_dir.join(AGENT_MCP_FILE)),
            }),
        }
    }

    /// Whether `config_dir` holds the Agent's settings. They are written
    /// last when a configuration is created, so a creation that stopped
    /// part-way does not count.
    pub fn exists(config_dir: &Path) -> bool {
        config_dir.join(AGENT_SETTINGS_FILE).is_file()
    }

    pub fn settings(&self) -> Result<AgentSettings, String> {
        let mut cached = self.inner.settings.lock();
        if let Some(settings) = cached.as_ref() {
            return Ok(settings.clone());
        }
        let settings = self.load_settings()?;
        *cached = Some(settings.clone());
        Ok(settings)
    }

    /// The settings read so far, without reading the file: `None` before
    /// the first [`Self::settings`].
    pub fn settings_snapshot(&self) -> Option<AgentSettings> {
        self.inner.settings.lock().clone()
    }

    /// Applies `patch` to the settings on disk, as a whole or not at all.
    pub fn write_settings(&self, patch: AgentSettingsPatch) -> Result<AgentSettings, String> {
        self.update_settings(|settings| patch.apply(settings))
    }

    /// Turns the plugin `name` off (`disabled`) or on again.
    pub fn set_plugin_disabled(&self, name: &str, disabled: bool) -> Result<AgentSettings, String> {
        self.update_settings(|settings| {
            if disabled {
                settings.disabled_plugins.insert(name.to_string());
            } else {
                settings.disabled_plugins.remove(name);
            }
            validate_disabled_plugins(&settings.disabled_plugins)
        })
    }

    pub fn providers(&self) -> Result<AgentProviderRegistrySettings, String> {
        let mut cached = self.inner.providers.lock();
        if let Some(registry) = cached.as_ref() {
            return Ok(registry.clone());
        }
        let registry = self.load_providers()?;
        *cached = Some(registry.clone());
        Ok(registry)
    }

    /// Changes the Provider registry when its revision is still
    /// `expected_revision`, and raises the revision.
    pub fn mutate_providers<F>(
        &self,
        expected_revision: u64,
        mutate: F,
    ) -> Result<AgentProviderRegistrySettings, String>
    where
        F: FnOnce(&mut AgentProviderRegistrySettings) -> Result<(), String>,
    {
        let mut cached = self.inner.providers.lock();
        let mut registry = self.load_providers()?;
        if registry.revision != expected_revision {
            *cached = Some(registry);
            return Err("stale Agent Provider registry revision".to_string());
        }
        mutate(&mut registry)?;
        registry.revision = registry
            .revision
            .checked_add(1)
            .ok_or_else(|| "Agent Provider registry revision exhausted".to_string())?;
        validate_agent_provider_registry(&registry)?;
        self.inner.providers_file.write(&object(&registry)?)?;
        *cached = Some(registry.clone());
        Ok(registry)
    }

    pub fn mcp(&self) -> &McpSettingsStore {
        &self.inner.mcp
    }

    /// Writes a whole configuration into a directory that has none, the
    /// settings last (see [`Self::exists`]).
    pub fn create(
        &self,
        settings: &AgentSettings,
        providers: &AgentProviderRegistrySettings,
        mcp: &McpSettings,
    ) -> Result<(), String> {
        self.inner.providers_file.write(&object(providers)?)?;
        self.inner
            .mcp
            .write(mcp)
            .map_err(|error| error.to_string())?;
        self.inner.settings_file.write(&object(settings)?)?;
        *self.inner.settings.lock() = Some(settings.clone());
        *self.inner.providers.lock() = Some(providers.clone());
        Ok(())
    }

    fn update_settings(
        &self,
        update: impl FnOnce(&mut AgentSettings) -> Result<(), String>,
    ) -> Result<AgentSettings, String> {
        let mut cached = self.inner.settings.lock();
        let mut settings = self.load_settings()?;
        update(&mut settings)?;
        self.inner.settings_file.write(&object(&settings)?)?;
        *cached = Some(settings.clone());
        Ok(settings)
    }

    fn load_settings(&self) -> Result<AgentSettings, String> {
        load(
            &self.inner.settings_file,
            AgentSettings::default,
            |document| {
                let mut repairs = Vec::new();
                (decode_settings(document, &mut repairs), repairs)
            },
        )
        .map(|loaded| loaded.value)
    }

    fn load_providers(&self) -> Result<AgentProviderRegistrySettings, String> {
        load(
            &self.inner.providers_file,
            AgentProviderRegistrySettings::default,
            |document| {
                let mut repairs = Vec::new();
                (decode_registry(document, &mut repairs), repairs)
            },
        )
        .map(|loaded| loaded.value)
    }
}

/// The settings a document holds, read tolerantly, and the repairs made:
/// what a configuration written by another version holds (the host's import
/// of its own settings files reads them with this).
pub fn read_settings_document(document: &Map<String, Value>) -> (AgentSettings, Vec<String>) {
    let mut repairs = Vec::new();
    (decode_settings(document, &mut repairs), repairs)
}

/// The Provider registry a document holds, as [`read_settings_document`].
pub fn read_providers_document(
    document: &Map<String, Value>,
) -> (AgentProviderRegistrySettings, Vec<String>) {
    let mut repairs = Vec::new();
    (decode_registry(document, &mut repairs), repairs)
}

/// The MCP settings a document holds, as [`read_settings_document`].
pub fn read_mcp_document(document: &Map<String, Value>) -> (McpSettings, Vec<String>) {
    let (settings, repairs) =
        hanafubuki_agent_mcp::repair_settings(Value::Object(document.clone()));
    let repairs = repairs.iter().map(|repair| repair.detail.clone()).collect();
    (settings, repairs)
}

pub(crate) struct Loaded<T> {
    pub(crate) value: T,
    /// Why the file could not be used at all, when it could not.
    pub(crate) unusable: Option<String>,
}

/// Reads `file` with `decode`. A file that cannot be read as a document, or
/// whose values needed repair, is kept in `.recovery/` and rewritten: with
/// the defaults, or with the repaired values over the user's own document.
/// Unknown keys are logged and stay in the file.
pub(crate) fn load<T: Serialize>(
    file: &ConfigFile,
    defaults: impl FnOnce() -> T,
    decode: impl FnOnce(&Map<String, Value>) -> (T, Vec<String>),
) -> Result<Loaded<T>, String> {
    let path = file.path().display();
    match file.read()? {
        FileRead::Missing => Ok(Loaded {
            value: defaults(),
            unusable: None,
        }),
        FileRead::Unusable(reason) => {
            log::warn!("Agent configuration {path} is unusable ({reason}); a copy is kept in .recovery and the defaults are used");
            file.preserve()?;
            let value = defaults();
            file.write(&object(&value)?)?;
            Ok(Loaded {
                value,
                unusable: Some(reason),
            })
        }
        FileRead::Document(document) => {
            let (value, repairs) = decode(&document);
            let canonical = object(&value)?;
            for key in unknown_keys(&document, &canonical) {
                log::warn!(
                    "Agent configuration {path} has the unknown key {key}; it is kept and ignored"
                );
            }
            if !repairs.is_empty() {
                for repair in &repairs {
                    log::warn!("Agent configuration {path} repaired {repair}");
                }
                file.preserve()?;
                file.write(&canonical)?;
            }
            Ok(Loaded {
                value,
                unusable: None,
            })
        }
    }
}

pub(crate) fn object<T: Serialize>(value: &T) -> Result<Map<String, Value>, String> {
    match serde_json::to_value(value) {
        Ok(Value::Object(object)) => Ok(object),
        _ => Err("the Agent configuration cannot be encoded".to_string()),
    }
}

#[cfg(test)]
mod tests;
