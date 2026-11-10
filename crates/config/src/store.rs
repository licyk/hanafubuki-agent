//! The Agent's configuration directory and the settings read from it.

use std::path::Path;
use std::sync::Arc;

use hanafubuki_agent_mcp::McpSettings;
use hanafubuki_agent_packages::{
    validate_marketplace_name, MarketplaceRecord, PluginId, PluginSettings,
};
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::fields::unknown_keys;
use crate::file::{ConfigFile, FileAccess, FileRead};
use crate::keys::{self, KeyTarget};
use crate::layers::Resolved;
use crate::overrides::parse_value;
use crate::providers::{decode_registry, validate_agent_provider_registry};
use crate::recovery::{AgentConfigProblem, AgentConfigRecovery};
use crate::settings::{decode_settings, validate_disabled_plugins};
use crate::{
    AgentProviderRegistrySettings, AgentSettings, AgentSettingsPatch, AgentStandaloneSettings,
    AgentStandaloneSettingsPatch, AgentTuiPreferences, AgentTuiPreferencesPatch, ConfigLayerSource,
    ConfigLayers, McpSettingsStore,
};

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
///
/// A host's configuration is the user's files alone ([`Self::new`]). The
/// standalone Agent's settings are layered ([`Self::layered`]): what
/// [`Self::settings`] returns has the profile, the project's file and the
/// `-c` overrides applied, while a write still changes only the user's
/// `config.toml`. The Provider registry and the MCP servers are never
/// layered.
#[derive(Clone)]
pub struct AgentConfig {
    inner: Arc<AgentConfigInner>,
}

struct AgentConfigInner {
    settings_file: ConfigFile,
    providers_file: ConfigFile,
    /// `None` for a host's configuration.
    layers: Option<ConfigLayers>,
    settings: Mutex<Option<Resolved>>,
    providers: Mutex<Option<AgentProviderRegistrySettings>>,
    mcp: McpSettingsStore,
}

impl AgentConfig {
    /// The configuration in `config_dir`; nothing is read until it is used.
    pub fn new(config_dir: &Path) -> Self {
        Self::open(config_dir, None)
    }

    /// The standalone configuration in `config_dir`, with `layers` over the
    /// user's settings.
    pub fn layered(config_dir: &Path, layers: ConfigLayers) -> Self {
        Self::open(config_dir, Some(layers))
    }

    fn open(config_dir: &Path, layers: Option<ConfigLayers>) -> Self {
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
                layers,
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
        self.resolved().map(|resolved| resolved.settings)
    }

    /// The settings only the standalone Agent reads (`[network]`,
    /// `logLevel`); a host's configuration has the defaults, since the host
    /// supplies its own.
    pub fn standalone_settings(&self) -> Result<AgentStandaloneSettings, String> {
        self.resolved().map(|resolved| resolved.standalone)
    }

    /// The terminal interface's preferences (`[tui]`), with the layers
    /// applied; a host's configuration has the defaults.
    pub fn tui_preferences(&self) -> Result<AgentTuiPreferences, String> {
        self.standalone_settings().map(|standalone| standalone.tui)
    }

    /// What the layers of `config.toml` hold that the settings cannot use
    /// (unknown keys, values the layer below replaces), as the last read
    /// found it; empty for a host's configuration, which is not layered.
    pub fn layer_warnings(&self) -> Result<Vec<String>, String> {
        self.resolved().map(|resolved| resolved.warnings)
    }

    /// Writes the keys `patch` names into the `[tui]` table of the user's
    /// `config.toml`, keeping the rest of the file as written, and returns
    /// the preferences as the layers now resolve them.
    pub fn write_tui_preferences(
        &self,
        patch: AgentTuiPreferencesPatch,
    ) -> Result<AgentTuiPreferences, String> {
        {
            let mut cached = self.inner.settings.lock();
            // A file that needs repair is kept in `.recovery/` before the
            // write changes it.
            self.load_user_settings()?;
            self.inner.settings_file.write(&patch.document())?;
            *cached = None;
        }
        self.tui_preferences()
    }

    /// Writes the keys `patch` names into the user's `config.toml` and
    /// returns the standalone settings as the layers now resolve them.
    pub fn write_standalone_settings(
        &self,
        patch: AgentStandaloneSettingsPatch,
    ) -> Result<AgentStandaloneSettings, String> {
        {
            let mut cached = self.inner.settings.lock();
            self.load_user_settings()?;
            self.inner.settings_file.write(&patch.document())?;
            *cached = None;
        }
        self.standalone_settings()
    }

    /// The settings read so far, without reading the file: `None` before
    /// the first [`Self::settings`].
    pub fn settings_snapshot(&self) -> Option<AgentSettings> {
        self.inner
            .settings
            .lock()
            .as_ref()
            .map(|resolved| resolved.settings.clone())
    }

    fn resolved(&self) -> Result<Resolved, String> {
        let mut cached = self.inner.settings.lock();
        if let Some(resolved) = cached.as_ref() {
            return Ok(resolved.clone());
        }
        let resolved = self.load_resolved()?;
        *cached = Some(resolved.clone());
        Ok(resolved)
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

    /// Writes the `[plugins]` entry of `id`, or removes it (`None`), in the
    /// user's `config.toml`, keeping the rest of the file as written, and
    /// returns the settings as the layers now resolve them.
    pub fn set_package_settings(
        &self,
        id: &PluginId,
        settings: Option<&PluginSettings>,
    ) -> Result<AgentSettings, String> {
        self.write_entry(&["plugins".to_string(), id.key()], settings)
    }

    /// Replaces the settings of the code of `id`
    /// (`[plugins."<id>"].settings`), or removes them (`None`), as
    /// [`Self::set_package_settings`] writes the rest of its entry.
    pub fn set_package_code_settings(
        &self,
        id: &PluginId,
        settings: Option<&Map<String, Value>>,
    ) -> Result<AgentSettings, String> {
        let path = ["plugins".to_string(), id.key(), "settings".to_string()];
        let written = self
            .settings()?
            .plugins
            .packages
            .get(&id.key())
            .is_some_and(|entry| entry.settings.is_some());
        // A write merges into the table it finds, which would keep the keys
        // the new settings drop: the old table goes first.
        if written {
            self.write_entry(&path, None::<&Value>)?;
        }
        match settings {
            Some(settings) => self.write_entry(&path, Some(settings)),
            None => self.settings(),
        }
    }

    /// Writes the `[marketplaces]` record of `name`, or removes it
    /// (`None`), as [`Self::set_package_settings`] does.
    pub fn set_marketplace(
        &self,
        name: &str,
        record: Option<&MarketplaceRecord>,
    ) -> Result<AgentSettings, String> {
        validate_marketplace_name(name).map_err(|error| error.to_string())?;
        self.write_entry(&["marketplaces".to_string(), name.to_string()], record)
    }

    /// A table's entry, written whole or removed: an unset field is written
    /// as `null`, which the write removes from the file.
    fn write_entry<T: Serialize>(
        &self,
        path: &[String],
        value: Option<&T>,
    ) -> Result<AgentSettings, String> {
        let value = match value {
            Some(value) => serde_json::to_value(value)
                .map_err(|_| "the Agent configuration cannot be encoded".to_string())?,
            None => Value::Null,
        };
        {
            let mut cached = self.inner.settings.lock();
            self.load_user_settings()?;
            let mut change = Map::new();
            keys::insert(&mut change, path, value);
            self.inner.settings_file.write(&change)?;
            *cached = None;
        }
        self.settings()
    }

    /// The settings with every layer applied, as one document (`config
    /// get`).
    pub fn resolved_document(&self) -> Result<Map<String, Value>, String> {
        let resolved = self.resolved()?;
        let mut document = object(&resolved.settings)?;
        document.extend(object(&resolved.standalone)?);
        Ok(document)
    }

    /// The value of `key`, a dotted TOML key: a setting as the layers
    /// resolve it, a profile's or a project's entry as the user's file holds
    /// it; `None` when it is not set.
    pub fn value(&self, key: &str) -> Result<Option<Value>, String> {
        let path = keys::parse_key(key)?;
        let value = match keys::target(key, &path)? {
            KeyTarget::Setting(setting) => keys::lookup(&self.resolved_document()?, setting)
                .cloned()
                .ok_or_else(|| format!("`{key}` is not a setting of config.toml"))?,
            KeyTarget::Profile { .. } | KeyTarget::ProjectTrust => {
                let document = self.load_user_settings()?.document;
                keys::lookup(&document, &path)
                    .cloned()
                    .unwrap_or(Value::Null)
            }
        };
        Ok(Some(value).filter(|value| !value.is_null()))
    }

    /// Sets `key` (a dotted TOML key, as [`Self::value`] reads) in the
    /// user's `config.toml` to `raw`, TOML or, when it is not, a string as
    /// for `-c`. The key must be a setting, a profile's setting or a
    /// project's `trustLevel`, and the value one the Agent can use; the rest
    /// of the file stays as written.
    pub fn set_user_value(&self, key: &str, raw: &str) -> Result<(), String> {
        let path = keys::parse_key(key)?;
        let value = serde_json::to_value(parse_value(raw))
            .map_err(|error| format!("`{key}` cannot take this value: {error}"))?;
        let mut cached = self.inner.settings.lock();
        let user = self.load_user_settings()?;
        match keys::target(key, &path)? {
            KeyTarget::Setting(setting) => keys::check_setting(
                key,
                setting,
                &user.document,
                &value,
                &AgentSettings::default(),
            )?,
            KeyTarget::Profile { name, key: setting } => {
                let profile = keys::lookup(&user.document, &path[..2])
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                keys::check_setting(key, setting, &profile, &value, &user.value)
                    .map_err(|error| format!("{error} (profile `{name}`)"))?
            }
            KeyTarget::ProjectTrust => keys::check_trust(key, &value)?,
        }
        // Only the key is written: the write keeps every other key as the
        // file has it.
        let mut change = Map::new();
        keys::insert(&mut change, &path, value);
        self.inner.settings_file.write(&change)?;
        *cached = None;
        Ok(())
    }

    /// The layers the settings are read from, lowest first (`doctor`).
    pub fn layer_sources(&self) -> Result<Vec<ConfigLayerSource>, String> {
        let path = self.inner.settings_file.path().to_path_buf();
        let present = path.is_file();
        let mut sources = vec![ConfigLayerSource::User { path, present }];
        if let Some(layers) = &self.inner.layers {
            sources.extend(layers.sources(&self.load_user_settings()?.document));
        }
        Ok(sources)
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
        self.inner.providers_file.write(&registry)?;
        *cached = Some(registry.clone());
        Ok(registry)
    }

    pub fn mcp(&self) -> &McpSettingsStore {
        &self.inner.mcp
    }

    /// What the reads of the settings, the Provider registry and the MCP
    /// servers had to fix since this configuration was opened, oldest first.
    pub fn recoveries(&self) -> Vec<AgentConfigRecovery> {
        let mut recoveries = self.inner.settings_file.recoveries();
        recoveries.extend(self.inner.providers_file.recoveries());
        recoveries.extend(self.inner.mcp.file().recoveries());
        recoveries.sort_by_key(|recovery| recovery.recorded_at_unix_ms);
        recoveries
    }

    /// Writes a whole configuration into a directory that has none, the
    /// settings last (see [`Self::exists`]).
    pub fn create(
        &self,
        settings: &AgentSettings,
        providers: &AgentProviderRegistrySettings,
        mcp: &McpSettings,
    ) -> Result<(), String> {
        self.inner.providers_file.write(providers)?;
        self.inner
            .mcp
            .write(mcp)
            .map_err(|error| error.to_string())?;
        self.inner.settings_file.write(settings)?;
        *self.inner.settings.lock() = self
            .inner
            .layers
            .is_none()
            .then(|| Resolved::unlayered(settings.clone()));
        *self.inner.providers.lock() = Some(providers.clone());
        Ok(())
    }

    /// Changes the user's settings: the layers above them are applied
    /// again afterwards, never written.
    fn update_settings(
        &self,
        update: impl FnOnce(&mut AgentSettings) -> Result<(), String>,
    ) -> Result<AgentSettings, String> {
        let mut cached = self.inner.settings.lock();
        let mut settings = self.load_user_settings()?.value;
        update(&mut settings)?;
        self.inner.settings_file.write(&settings)?;
        let resolved = match &self.inner.layers {
            None => Resolved::unlayered(settings),
            Some(_) => self.load_resolved()?,
        };
        *cached = Some(resolved.clone());
        Ok(resolved.settings)
    }

    /// The user's settings file. A layered configuration reports its unknown
    /// keys once the layers are known, since the layers read some of them.
    fn load_user_settings(&self) -> Result<Loaded<AgentSettings>, String> {
        load(
            &self.inner.settings_file,
            AgentSettings::default,
            |document| {
                let mut repairs = Vec::new();
                (
                    decode_settings(document, &AgentSettings::default(), &mut repairs),
                    repairs,
                )
            },
            self.inner.layers.is_none(),
        )
    }

    fn load_resolved(&self) -> Result<Resolved, String> {
        let user = self.load_user_settings()?;
        match &self.inner.layers {
            None => Ok(Resolved::unlayered(user.value)),
            Some(layers) => {
                layers.resolve(self.inner.settings_file.path(), user.value, &user.document)
            }
        }
    }

    fn load_providers(&self) -> Result<AgentProviderRegistrySettings, String> {
        load(
            &self.inner.providers_file,
            AgentProviderRegistrySettings::default,
            |document| {
                let mut repairs = Vec::new();
                (decode_registry(document, &mut repairs), repairs)
            },
            true,
        )
        .map(|loaded| loaded.value)
    }
}

/// The settings a document holds, read tolerantly, and the repairs made:
/// what a configuration written by another version holds (the host's import
/// of its own settings files reads them with this).
pub fn read_settings_document(document: &Map<String, Value>) -> (AgentSettings, Vec<String>) {
    let mut repairs = Vec::new();
    (
        decode_settings(document, &AgentSettings::default(), &mut repairs),
        repairs,
    )
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
    /// What the file held as it was read; empty when it held nothing usable.
    pub(crate) document: Map<String, Value>,
    /// Whether this read recorded a recovery: a file left read-only finds
    /// the same problem at every read and records it once.
    pub(crate) recorded: bool,
}

/// Reads `file` with `decode`. A file that cannot be read as a document, or
/// whose values needed repair, is kept in `.recovery/` and rewritten: with
/// the defaults, or with the repaired values over the user's own document.
/// When that fails, the fixed values are used from memory and the file
/// refuses writes: one would replace the user's original with no copy kept.
/// Unknown keys stay in the file, and are logged when `report_unknown`.
pub(crate) fn load<T: Serialize>(
    file: &ConfigFile,
    defaults: impl FnOnce() -> T,
    decode: impl FnOnce(&Map<String, Value>) -> (T, Vec<String>),
    report_unknown: bool,
) -> Result<Loaded<T>, String> {
    let path = file.path().display();
    let fresh = !file.is_read_only();
    match file.read()? {
        FileRead::Missing => Ok(Loaded {
            value: defaults(),
            unusable: None,
            document: Map::new(),
            recorded: false,
        }),
        FileRead::Unusable(reason) => {
            let value = defaults();
            if fresh {
                log::warn!("Agent configuration {path} is unusable ({reason}); a copy is kept in .recovery and the defaults are used");
                let problem = AgentConfigProblem::Unusable {
                    reason: reason.clone(),
                };
                replace(file, problem, &value);
            }
            Ok(Loaded {
                value,
                unusable: Some(reason),
                document: Map::new(),
                recorded: fresh,
            })
        }
        FileRead::Document(document) => {
            let (value, repairs) = decode(&document);
            let canonical = object(&value)?;
            if report_unknown {
                for key in unknown_keys(&document, &canonical) {
                    log::warn!(
                        "Agent configuration {path} has the unknown key {key}; it is kept and ignored"
                    );
                }
            }
            let recorded = fresh && !repairs.is_empty();
            if recorded {
                for repair in &repairs {
                    log::warn!("Agent configuration {path} repaired {repair}");
                }
                replace(file, AgentConfigProblem::Repaired { repairs }, &canonical);
            }
            Ok(Loaded {
                value,
                unusable: None,
                document,
                recorded,
            })
        }
    }
}

/// Keeps `file` in `.recovery/` and writes `value` over it, or leaves the
/// file read-only when either fails; records the recovery either way.
fn replace<T: Serialize>(file: &ConfigFile, problem: AgentConfigProblem, value: &T) {
    let read_only = file.preserve().and_then(|()| file.write(value)).err();
    if let Some(error) = &read_only {
        log::warn!(
            "Agent configuration {} is used from memory and is read-only until the Agent restarts: {error}",
            file.path().display()
        );
        file.set_read_only(error.clone());
    }
    file.record(AgentConfigRecovery::new(
        file.path().to_path_buf(),
        problem,
        read_only,
    ));
}

pub(crate) fn object<T: Serialize>(value: &T) -> Result<Map<String, Value>, String> {
    match serde_json::to_value(value) {
        Ok(Value::Object(object)) => Ok(object),
        _ => Err("the Agent configuration cannot be encoded".to_string()),
    }
}

#[cfg(test)]
mod tests;
