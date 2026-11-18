//! Checked edits of explicitly selected configuration layers.
use super::AgentConfig;
use crate::file::resolve_destination;
use crate::file::{ConfigFile, FileAccess, FileRead};
use crate::keys::{self, KeyChange};
use crate::{AgentSettings, ConfigLayerSource, SettingSpec};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsLayer {
    User,
    Profile(String),
    Project,
}

impl AgentConfig {
    /// The user's explicit trust decision for a workspace used by a side request.
    pub fn trusts_project(&self, directory: &std::path::Path) -> Result<bool, String> {
        Ok(crate::layers::is_trusted(&self.user_document()?, directory))
    }

    /// The current project's target, including a configuration file not yet created.
    pub fn project_configuration_source(&self) -> Result<Option<ConfigLayerSource>, String> {
        let Some(project) = self
            .inner
            .layers
            .as_ref()
            .and_then(|layers| layers.project.as_ref())
        else {
            return Ok(None);
        };
        let user = self.load_user_settings()?.document;
        Ok(Some(ConfigLayerSource::Project {
            path: crate::ConfigLayers::project_file(project),
            trusted: crate::layers::is_trusted(&user, project),
        }))
    }

    /// The selected layer's own values and save path, before higher layers apply.
    pub fn layer_document(
        &self,
        layer: &SettingsLayer,
    ) -> Result<(PathBuf, Map<String, Value>), String> {
        match layer {
            SettingsLayer::User => Ok((
                self.inner.settings_file.path().to_path_buf(),
                self.user_document()?,
            )),
            SettingsLayer::Profile(name) => {
                let document = self.user_document()?;
                Ok((
                    self.inner.settings_file.path().to_path_buf(),
                    document
                        .get("profiles")
                        .and_then(|profiles| profiles.get(name))
                        .and_then(Value::as_object)
                        .cloned()
                        .unwrap_or_default(),
                ))
            }
            SettingsLayer::Project => {
                let path = self
                    .project_configuration_source()?
                    .into_iter()
                    .find_map(|source| match source {
                        ConfigLayerSource::Project {
                            path,
                            trusted: true,
                        } => Some(path),
                        _ => None,
                    })
                    .ok_or(
                        "The current project must be trusted before editing its configuration",
                    )?;
                let resolved = resolve_destination(&path)?;
                let file = ConfigFile::new(resolved.system().to_path_buf(), FileAccess::Shared);
                let document = match file.read()? {
                    FileRead::Missing => Map::new(),
                    FileRead::Document(document) => document,
                    FileRead::Unusable(reason) => return Err(reason),
                };
                Ok((resolved.native().to_path_buf(), document))
            }
        }
    }

    /// Writes fields described by their owners' registered specs. Unknown keys
    /// in the stored document survive; a null patch removes an override.
    pub fn write_layer_fields(
        &self,
        layer: &SettingsLayer,
        values: &Map<String, Value>,
        specs: &[SettingSpec],
    ) -> Result<PathBuf, String> {
        let changes = values
            .iter()
            .map(|(key, value)| {
                let spec = specs
                    .iter()
                    .find(|spec| spec.key == *key)
                    .ok_or_else(|| format!("Unknown setting {key}"))?;
                let value = if value.is_null() {
                    None
                } else {
                    Some(spec.check(value)?.to_json()).filter(|value| !value.is_null())
                };
                Ok(KeyChange {
                    key: key.clone(),
                    path: keys::parse_key(key)?,
                    value,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let (path, before) = self.layer_document(layer)?;
        let below = self.load_user_settings()?.value;
        let defaults = AgentSettings::default();
        keys::check_changes(
            &before,
            &changes,
            if *layer == SettingsLayer::User {
                &defaults
            } else {
                &below
            },
        )?;
        let mut patch = Map::new();
        let prefix = match layer {
            SettingsLayer::Profile(name) => vec!["profiles".into(), name.clone()],
            _ => Vec::new(),
        };
        for change in changes {
            let mut path = prefix.clone();
            path.extend(change.path);
            keys::insert(&mut patch, &path, change.value.unwrap_or(Value::Null));
        }
        let mut cached = self.inner.settings.lock();
        match layer {
            SettingsLayer::User | SettingsLayer::Profile(_) => {
                self.inner.settings_file.write(&patch)?
            }
            SettingsLayer::Project => {
                let path = resolve_destination(&path)?;
                ConfigFile::new(path.system().to_path_buf(), FileAccess::Shared).write(&patch)?;
            }
        }
        *cached = None;
        Ok(path)
    }
}

/// Reads a dotted TOML key, including quoted collection identities.
pub fn setting_value_at<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    keys::parse_key(key)
        .ok()?
        .iter()
        .try_fold(value, |value, key| value.get(key))
}

/// Applies checked dotted fields to a boundary document for owner validation.
pub fn patch_setting_values(
    document: &mut Map<String, Value>,
    values: &BTreeMap<String, Value>,
) -> Result<(), String> {
    for (key, value) in values {
        let path = keys::parse_key(key)?;
        if value.is_null() {
            keys::remove(document, &path);
        } else {
            keys::insert(document, &path, value.clone());
        }
    }
    Ok(())
}
