//! The standalone configuration's layers over the user's `config.toml`,
//! lowest first: the selected profile (`[profiles.<name>]` of the user's
//! file), the project's `.hanafubuki-agent/config.toml` when the user's file
//! trusts the project, and the `-c` overrides.
//!
//! Each layer is decoded over the one below it, field by field: a key a
//! layer leaves out, or sets to a value that cannot be used, keeps the value
//! below. Only the user's file is ever written, and only by the Agent's
//! settings writes; the layers above it are read.

use std::path::{Path, PathBuf};

use hanafubuki_agent_foundation::paths::ResolvedPath;
use serde_json::{Map, Value};

use crate::fields::unknown_keys;
use crate::file::{ConfigFile, FileAccess, FileRead};
use crate::keys::{HOOKS, PROFILES, PROJECTS, TRUST_LEVEL};
use crate::settings::decode_settings;
use crate::standalone::decode_standalone;
use crate::store::object;
use crate::{AgentSettings, AgentStandaloneSettings, ConfigOverrides, AGENT_SETTINGS_FILE};

/// The directory of a project's configuration, in the project's root. The
/// user's file names trusted projects in its `projects` table, keyed by
/// path: `[projects."/path/to/project"] trustLevel = "trusted"`.
pub const PROJECT_CONFIG_DIR: &str = ".hanafubuki-agent";

/// What the standalone Agent layers over the user's `config.toml`.
#[derive(Clone, Debug, Default)]
pub struct ConfigLayers {
    /// The profile `-p` selects.
    pub profile: Option<String>,
    /// The project the Agent works in (its working directory's Git root, or
    /// the working directory): its `.hanafubuki-agent/config.toml` is read
    /// when the user's file trusts it.
    pub project: Option<PathBuf>,
    pub overrides: ConfigOverrides,
}

/// One layer of a configuration, as it was found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigLayerSource {
    /// The user's `config.toml`; `present` is false when there is none.
    User { path: PathBuf, present: bool },
    /// The profile `-p` selects.
    Profile { name: String },
    /// The project's file, which is read only when the project is trusted.
    Project { path: PathBuf, trusted: bool },
    /// The `-c` overrides.
    Overrides,
}

/// The settings with every layer applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Resolved {
    pub(crate) settings: AgentSettings,
    pub(crate) standalone: AgentStandaloneSettings,
    /// What the layers hold that the settings cannot use: unknown keys and
    /// values replaced by the layer below, each naming its layer.
    pub(crate) warnings: Vec<String>,
}

impl Resolved {
    pub(crate) fn unlayered(settings: AgentSettings) -> Self {
        Self {
            settings,
            standalone: AgentStandaloneSettings::default(),
            warnings: Vec::new(),
        }
    }
}

impl ConfigLayers {
    /// The project configuration file of `project`.
    pub fn project_file(project: &Path) -> PathBuf {
        project.join(PROJECT_CONFIG_DIR).join(AGENT_SETTINGS_FILE)
    }

    /// The layers over `user`, the user's settings as their file holds them
    /// (`document`, read from `user_path`). Fails only for a profile the file
    /// does not have.
    pub(crate) fn resolve(
        &self,
        user_path: &Path,
        user: AgentSettings,
        document: &Map<String, Value>,
    ) -> Result<Resolved, String> {
        let mut repairs = Vec::new();
        let standalone =
            decode_standalone(document, &AgentStandaloneSettings::default(), &mut repairs);
        let mut resolved = Resolved {
            settings: user,
            standalone,
            warnings: Vec::new(),
        };
        let user_source = user_path.display().to_string();
        report(
            &user_source,
            document,
            &mut resolved,
            &[PROFILES, PROJECTS, HOOKS],
            repairs,
        )?;
        if let Some(name) = &self.profile {
            let profile = profile(document, name).ok_or_else(|| {
                format!("{user_source} has no profile `{name}` ([profiles.{name}])")
            })?;
            apply(&format!("profile `{name}`"), profile, &mut resolved)?;
        }
        if let Some((path, project)) = self.project_document(document) {
            apply(&path.display().to_string(), &project, &mut resolved)?;
        }
        if !self.overrides.is_empty() {
            let overrides = match serde_json::to_value(self.overrides.document()) {
                Ok(Value::Object(overrides)) => overrides,
                _ => return Err("the -c overrides cannot be read as settings".to_string()),
            };
            apply("-c", &overrides, &mut resolved)?;
        }
        Ok(resolved)
    }

    /// The layers above the user's file that exist (a project file that is
    /// there, overrides that were given), lowest first.
    pub(crate) fn sources(&self, user: &Map<String, Value>) -> Vec<ConfigLayerSource> {
        let mut sources = Vec::new();
        if let Some(name) = &self.profile {
            sources.push(ConfigLayerSource::Profile { name: name.clone() });
        }
        if let Some(project) = self.project.as_deref() {
            let path = Self::project_file(project);
            if path.is_file() {
                let trusted = is_trusted(user, project);
                sources.push(ConfigLayerSource::Project { path, trusted });
            }
        }
        if !self.overrides.is_empty() {
            sources.push(ConfigLayerSource::Overrides);
        }
        sources
    }

    /// The project's configuration file and what it holds, when the project
    /// is trusted and has one the Agent can read. A project's file is never
    /// repaired or quarantined: it is the project's, not the user's.
    fn project_document(&self, user: &Map<String, Value>) -> Option<(PathBuf, Map<String, Value>)> {
        let project = self.project.as_deref()?;
        let file = ConfigFile::new(Self::project_file(project), FileAccess::Shared);
        let path = file.path().display();
        if !file.path().is_file() {
            return None;
        }
        if !is_trusted(user, project) {
            log::info!(
                "project configuration {path} is not read: the project is not trusted \
                 ([projects.\"{}\"] trustLevel = \"trusted\" in the user's config.toml)",
                project.display()
            );
            return None;
        }
        match file.read() {
            Ok(FileRead::Document(document)) => Some((file.path().to_path_buf(), document)),
            Ok(FileRead::Missing) => None,
            Ok(FileRead::Unusable(reason)) => {
                log::warn!("project configuration {path} is not read: {reason}");
                None
            }
            Err(error) => {
                log::warn!("project configuration {path} is not read: {error}");
                None
            }
        }
    }
}

fn profile<'a>(document: &'a Map<String, Value>, name: &str) -> Option<&'a Map<String, Value>> {
    document.get(PROFILES)?.as_object()?.get(name)?.as_object()
}

/// Whether the user's `[projects]` table trusts `project`, comparing
/// locations, not spellings.
fn is_trusted(user: &Map<String, Value>, project: &Path) -> bool {
    let Ok(project) = ResolvedPath::resolve(project) else {
        return false;
    };
    let Some(projects) = user.get(PROJECTS).and_then(Value::as_object) else {
        return false;
    };
    projects.iter().any(|(path, entry)| {
        entry.get(TRUST_LEVEL).and_then(Value::as_str) == Some("trusted")
            && ResolvedPath::resolve(path).is_ok_and(|path| path.key() == project.key())
    })
}

/// Decodes `layer` over `resolved`.
fn apply(source: &str, layer: &Map<String, Value>, resolved: &mut Resolved) -> Result<(), String> {
    let mut repairs = Vec::new();
    let settings = decode_settings(layer, &resolved.settings, &mut repairs);
    let standalone = decode_standalone(layer, &resolved.standalone, &mut repairs);
    resolved.settings = settings;
    resolved.standalone = standalone;
    report(source, layer, resolved, &[HOOKS], repairs)
}

/// Logs, and keeps in `resolved` for the interface's `/warnings`, the keys
/// of `layer` the settings do not have (but those in `tables`, which the
/// layering itself reads) and the values that could not be used.
fn report(
    source: &str,
    layer: &Map<String, Value>,
    resolved: &mut Resolved,
    tables: &[&str],
    repairs: Vec<String>,
) -> Result<(), String> {
    let mut known = object(&resolved.settings)?;
    known.extend(object(&resolved.standalone)?);
    let unknown = unknown_keys(layer, &known)
        .into_iter()
        .filter(|key| !tables.contains(&key.split('.').next().unwrap_or_default()))
        .map(|key| {
            format!("Agent configuration {source} has the unknown key {key}; it is ignored")
        });
    let unusable = repairs.into_iter().map(|repair| {
        format!("Agent configuration {source} has a value that cannot be used, so the one below it is: {repair}")
    });
    for warning in unknown.chain(unusable) {
        log::warn!("{warning}");
        resolved.warnings.push(warning);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
