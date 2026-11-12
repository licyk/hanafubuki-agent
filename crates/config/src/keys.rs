//! Keys of `config.toml` by path, as `config get` and `config set` name
//! them: a dotted TOML key (`memory.enabled`, `profiles.ci.permissionMode`,
//! `projects."/home/me/project".trustLevel`).

use serde_json::{Map, Value};

use crate::sections::LayerRead;
use crate::settings::decode_settings;
use crate::standalone::decode_standalone;
use crate::store::object;
use crate::{AgentSettings, AgentStandaloneSettings};

pub(crate) const PROFILES: &str = "profiles";
pub(crate) const PROJECTS: &str = "projects";
/// Read by the hooks plugin from each layer's file, not by the settings.
pub(crate) const HOOKS: &str = "hooks";
/// Read as role definitions from each layer's file, not by the settings.
pub(crate) const AGENTS: &str = "agents";
pub(crate) const TRUST_LEVEL: &str = "trustLevel";
/// Checked against the plugin's schema, which the caller has
/// ([`crate::AgentConfig::set_plugin_options`]).
pub(crate) const PLUGIN_OPTIONS: &str = "pluginOptions";
/// The values of a project's `trustLevel`; only the first trusts it.
const TRUST_LEVELS: [&str; 2] = ["trusted", "untrusted"];

/// What a key names.
pub(crate) enum KeyTarget<'a> {
    /// A setting, which the layers resolve.
    Setting(&'a [String]),
    /// A setting of the profile `name`.
    Profile { name: &'a str, key: &'a [String] },
    /// A project's trust.
    ProjectTrust,
}

pub(crate) fn parse_key(key: &str) -> Result<Vec<String>, String> {
    let keys = toml_edit::Key::parse(key)
        .map_err(|_| format!("`{key}` is not a key (write a dotted TOML key)"))?;
    Ok(keys.iter().map(|key| key.get().to_string()).collect())
}

/// Refuses a write of a plugin option the generic path cannot check: one
/// is written with its plugin's schema ([`crate::AgentConfig::set_plugin_options`]).
pub(crate) fn refuse_plugin_option(key: &str, path: &[String]) -> Result<(), String> {
    let options = |path: &[String]| path.first().is_some_and(|first| first == PLUGIN_OPTIONS);
    match path {
        _ if options(path) => Err(format!(
            "`{key}` is a plugin option, checked against the plugin's options: write \
             {PLUGIN_OPTIONS}.<plugin>.<option>"
        )),
        [first, _, rest @ ..] if first == PROFILES && options(rest) => Err(format!(
            "`{key}` is a profile's plugin option, which is written in the file by hand"
        )),
        _ => Ok(()),
    }
}

pub(crate) fn target<'a>(key: &str, path: &'a [String]) -> Result<KeyTarget<'a>, String> {
    match path {
        [first, name, rest @ ..] if first == PROFILES && !rest.is_empty() => {
            Ok(KeyTarget::Profile { name, key: rest })
        }
        [first, _, level] if first == PROJECTS && level == TRUST_LEVEL => {
            Ok(KeyTarget::ProjectTrust)
        }
        [first, ..] if first == PROFILES || first == PROJECTS => Err(format!(
            "`{key}` names no setting: write profiles.<name>.<key> or \
             projects.\"<path>\".{TRUST_LEVEL}"
        )),
        _ => Ok(KeyTarget::Setting(path)),
    }
}

/// One key a write changes.
pub(crate) struct KeyChange {
    pub(crate) key: String,
    pub(crate) path: Vec<String>,
    /// `None` removes the key.
    pub(crate) value: Option<Value>,
}

/// Checks `value` for `key` in `document`, a table of settings decoded over
/// `below`: the key must be a setting's, and the value one the Agent can
/// use (a value the document already could not use is not this write's).
pub(crate) fn check_setting(
    key: &str,
    path: &[String],
    document: &Map<String, Value>,
    value: &Value,
    below: &AgentSettings,
) -> Result<(), String> {
    let mut changed = document.clone();
    insert(&mut changed, path, value.clone());
    // `[plugins]` and `[marketplaces]` are keyed by what the user installed
    // and added: a key there is a setting when the changed document holds
    // it once read.
    if lookup(&known_settings()?, path).is_none()
        && lookup(&object(&decoded(&changed, below))?, path).is_none()
    {
        return Err(format!("`{key}` is not a setting of config.toml"));
    }
    let change = KeyChange {
        key: key.to_string(),
        path: path.to_vec(),
        value: Some(value.clone()),
    };
    check_changes(document, &[change], below)
}

/// Checks that `document` with `changes` made holds no value the Agent
/// cannot use that it did not already hold, reading it as one: the
/// changes of one write are checked together.
pub(crate) fn check_changes(
    document: &Map<String, Value>,
    changes: &[KeyChange],
    below: &AgentSettings,
) -> Result<(), String> {
    let before = repairs(document, below);
    let mut changed = document.clone();
    for change in changes {
        match &change.value {
            Some(value) => insert(&mut changed, &change.path, value.clone()),
            None => remove(&mut changed, &change.path),
        }
    }
    let Some(repair) = repairs(&changed, below)
        .into_iter()
        .find(|repair| !before.contains(repair))
    else {
        return Ok(());
    };
    Err(match changes {
        [change] => format!("`{}` cannot take this value: {repair}", change.key),
        _ => format!("these values cannot be written together: {repair}"),
    })
}

pub(crate) fn check_trust(key: &str, value: &Value) -> Result<(), String> {
    match value.as_str() {
        Some(level) if TRUST_LEVELS.contains(&level) => Ok(()),
        _ => Err(format!(
            "`{key}` is \"{}\" or \"{}\"",
            TRUST_LEVELS[0], TRUST_LEVELS[1]
        )),
    }
}

/// Every setting, with its default, as one document.
pub(crate) fn known_settings() -> Result<Map<String, Value>, String> {
    let mut known = object(&AgentSettings::default())?;
    known.extend(object(&AgentStandaloneSettings::default())?);
    Ok(known)
}

pub(crate) fn lookup<'a>(document: &'a Map<String, Value>, path: &[String]) -> Option<&'a Value> {
    let (last, parents) = path.split_last()?;
    let mut table = document;
    for key in parents {
        table = table.get(key)?.as_object()?;
    }
    table.get(last)
}

/// Sets `path` in `document` to `value`, making the tables on the way.
pub(crate) fn insert(document: &mut Map<String, Value>, path: &[String], value: Value) {
    let Some((last, parents)) = path.split_last() else {
        return;
    };
    let mut table = document;
    for key in parents {
        let entry = table
            .entry(key.clone())
            .or_insert_with(|| Value::Object(Map::new()));
        if !entry.is_object() {
            *entry = Value::Object(Map::new());
        }
        let Value::Object(next) = entry else {
            return;
        };
        table = next;
    }
    table.insert(last.clone(), value);
}

/// Removes `path` from `document`, when it is there.
fn remove(document: &mut Map<String, Value>, path: &[String]) {
    let Some((last, parents)) = path.split_last() else {
        return;
    };
    let mut table = document;
    for key in parents {
        let Some(Value::Object(next)) = table.get_mut(key) else {
            return;
        };
        table = next;
    }
    table.remove(last);
}

fn decoded(document: &Map<String, Value>, below: &AgentSettings) -> AgentSettings {
    decode_settings(document, below, "", &mut LayerRead::default())
}

fn repairs(document: &Map<String, Value>, below: &AgentSettings) -> Vec<String> {
    let mut read = LayerRead::default();
    decode_settings(document, below, "", &mut read);
    decode_standalone(document, &AgentStandaloneSettings::default(), &mut read);
    read.repairs
}

#[cfg(test)]
mod tests;
