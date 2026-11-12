//! Writing settings by key: [`AgentConfig::write_values`] writes the
//! settings of `config.toml` that have a spec, and `config set`
//! ([`AgentConfig::set_user_value`]) writes through it.

use std::collections::BTreeMap;

use hanafubuki_agent_foundation::settings::{SettingApply, SettingSpec};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::AgentConfig;
use crate::keys::{self, KeyChange, KeyTarget};
use crate::layers::SettingSource;
use crate::overrides::parse_value;
use crate::sections::{setting_spec, specs};
use crate::AgentSettings;

/// A change of settings by key, as a settings page or `config set` makes
/// it: every value is checked, and the change is written as a whole or not
/// at all. A host's frontend sends it, so it is strict.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsWrite {
    pub changes: Vec<SettingChange>,
}

/// One setting's change.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingChange {
    /// The setting's key, dotted (`memory.maxSourceAgeDays`).
    pub key: String,
    pub op: SettingWriteOp,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SettingWriteOp {
    /// Sets the key in the user's file to this value, which its spec must
    /// take as a write: in its bounds, a current name of a choice that is
    /// not disabled. `null` resets a nullable text.
    Set(Value),
    /// Removes the key from the user's file, so the layer below it applies
    /// again, or the default.
    Reset,
}

/// What a write did.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsWriteOutcome {
    /// The keys written, in the order of the changes.
    pub written: Vec<String>,
    /// The written keys a layer above the user's file sets, with that
    /// layer: the value written does not apply while it does.
    pub shadowed: Vec<(String, SettingSource)>,
    /// When each written key's change takes effect.
    pub apply: Vec<(String, SettingApply)>,
}

impl AgentConfig {
    /// Writes `write` into the user's `config.toml`, keeping comments and
    /// the keys it does not name as written. Each value is checked against
    /// its setting's spec first, then the changed file is read again, and
    /// the write is refused when that finds a value it cannot use (a rule
    /// across settings, a URL that does not parse): the changes of one
    /// write, a commit group's among them, are checked together. A refused
    /// write changes nothing.
    pub fn write_values(&self, write: &SettingsWrite) -> Result<SettingsWriteOutcome, String> {
        let changes = write
            .changes
            .iter()
            .map(checked_change)
            .collect::<Result<Vec<_>, _>>()?;
        let mut cached = self.inner.settings.lock();
        let user = self.load_user_settings()?;
        keys::check_changes(&user.document, &changes, &AgentSettings::default())?;
        let mut document = Map::new();
        for change in &changes {
            let value = change.value.clone().unwrap_or(Value::Null);
            keys::insert(&mut document, &change.path, value);
        }
        self.inner.settings_file.write(&document)?;
        *cached = None;
        let resolved = self.load_resolved()?;
        *cached = Some(resolved.clone());
        Ok(outcome(&changes, &resolved.sources))
    }

    /// The layer each setting's value comes from, by key; a setting no
    /// layer sets is not listed, and is [`SettingSource::Default`].
    pub fn setting_sources(&self) -> Result<BTreeMap<String, SettingSource>, String> {
        self.resolved().map(|resolved| resolved.sources)
    }

    /// Sets `key` (a dotted TOML key, as [`Self::value`] reads) in the
    /// user's `config.toml` to `raw`, TOML or, when it is not, a string as
    /// for `-c`. The key must be a setting (written by
    /// [`Self::write_values`] when it has a spec), a profile's setting or a
    /// project's `trustLevel`, and the value one the Agent can use; the rest
    /// of the file stays as written.
    pub fn set_user_value(&self, key: &str, raw: &str) -> Result<(), String> {
        let path = keys::parse_key(key)?;
        keys::refuse_plugin_option(key, &path)?;
        let value = serde_json::to_value(parse_value(raw))
            .map_err(|error| format!("`{key}` cannot take this value: {error}"))?;
        let target = keys::target(key, &path)?;
        if let KeyTarget::Setting(setting) = target {
            if let Some(spec) = spec_at(setting) {
                let change = SettingChange {
                    key: spec.key.clone(),
                    op: SettingWriteOp::Set(value),
                };
                return self
                    .write_values(&SettingsWrite {
                        changes: vec![change],
                    })
                    .map(drop);
            }
        }
        let mut cached = self.inner.settings.lock();
        let user = self.load_user_settings()?;
        match target {
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
                let checked = match spec_at(setting) {
                    Some(spec) => spec
                        .check(&value)
                        .map(drop)
                        .map_err(|error| refused(spec, error)),
                    None => Ok(()),
                };
                checked
                    .and_then(|()| keys::check_setting(key, setting, &profile, &value, &user.value))
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
}

/// The change as a key path and the value it writes (`None` removes it),
/// once the spec takes it.
fn checked_change(change: &SettingChange) -> Result<KeyChange, String> {
    let spec = setting_spec(&change.key)
        .ok_or_else(|| format!("`{}` is not a setting of config.toml", change.key))?;
    let value = match &change.op {
        SettingWriteOp::Set(raw) => Some(
            spec.check(raw)
                .map_err(|error| refused(spec, error))?
                .to_json(),
        ),
        SettingWriteOp::Reset => None,
    };
    Ok(KeyChange {
        key: spec.key.clone(),
        path: spec.key.split('.').map(str::to_string).collect(),
        // A TOML file has no null: a nullable text set to null is removed.
        value: value.filter(|value| !value.is_null()),
    })
}

/// The spec whose key is the key path `path`.
fn spec_at(path: &[String]) -> Option<&'static SettingSpec> {
    specs()
        .iter()
        .find(|spec| spec.key.split('.').eq(path.iter().map(String::as_str)))
}

/// A spec's refusal ("`key` is between 1 and 16") as a write's.
fn refused(spec: &SettingSpec, error: String) -> String {
    let key = &spec.key;
    match error.strip_prefix(&format!("`{key}` ")) {
        Some(reason) => format!("`{key}` cannot take this value: it {reason}"),
        None => error,
    }
}

fn outcome(
    changes: &[KeyChange],
    sources: &BTreeMap<String, SettingSource>,
) -> SettingsWriteOutcome {
    let written = changes
        .iter()
        .map(|change| change.key.clone())
        .collect::<Vec<_>>();
    let shadowed = written
        .iter()
        .filter_map(|key| match sources.get(key)? {
            SettingSource::Default | SettingSource::User => None,
            source => Some((key.clone(), source.clone())),
        })
        .collect();
    let apply = written
        .iter()
        .filter_map(|key| Some((key.clone(), setting_spec(key)?.apply)))
        .collect();
    SettingsWriteOutcome {
        written,
        shadowed,
        apply,
    }
}
