//! The settings catalog: every setting the Agent has, as one description a
//! settings page renders without code of its own per setting
//! (site/content/docs/architecture.en.mdx, "The settings catalog").
//!
//! It is built each time it is read, from what each owner already exposes:
//! the configuration's sections ([`AgentConfig::settings_sections`]), the
//! options of every plugin row that has some (its descriptor's schema, a
//! section in the Plugins group) and the collections below, each a module
//! of its own that reads its records from its owner and runs its actions
//! through the owner's calls. An owner that cannot be read leaves its
//! collection empty with the reason, and the rest of the catalog stands.
//!
//! [`AgentConfig::settings_sections`]: hanafubuki_agent_config::AgentConfig::settings_sections

mod hooks;
mod marketplaces;
mod mcp;
mod mcp_secrets;
mod model_policy;
mod package_settings;
mod packages;
mod plugin_mcp;
mod plugins;
mod profiles;
mod projects;
mod provider_models;
mod providers;
mod record;
mod roles;

use std::collections::BTreeMap;

use hanafubuki_agent_config::{
    SettingSource, SettingSpec, SettingState, SettingValue, SettingsGroup, SettingsSection,
    SettingsWrite, SettingsWriteOutcome,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::AgentApplicationService;
use crate::host::AgentPluginStatus;
pub use model_policy::reasoning_effort_choices;
pub use package_settings::package_setting_key;

/// Every setting, as a settings page shows it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsCatalog {
    pub sections: Vec<SettingsSection>,
    pub collections: Vec<SettingsCollection>,
}

/// A list of records edited one at a time (the Providers, the MCP
/// servers), each record's fields described by `record`, their defaults
/// what an `add` writes when a field is left out.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsCollection {
    pub id: String,
    pub title: String,
    pub group: SettingsGroup,
    /// The fields of one record, keys relative to it. A collection without
    /// a `save` action shows them and writes none.
    pub record: Vec<SettingSpec>,
    pub records: Vec<SettingsRecord>,
    pub actions: Vec<SettingsAction>,
    /// The revision of the file the records are in, for a collection whose
    /// writes name it ([`SettingsActionRequest::expected_revision`]).
    pub revision: Option<u64>,
    /// Why its records could not all be read.
    pub issues: Vec<String>,
}

/// One record of a collection.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsRecord {
    pub id: String,
    pub label: String,
    /// One line about it.
    pub summary: String,
    /// A short state shown beside it (`default`, `off`, `ready`).
    pub state: Option<String>,
    /// What is wrong with it now, when something is.
    pub issue: Option<String>,
    /// Why its fields are shown and not written here (a role defined in a
    /// file), when they are not; its actions still run.
    pub read_only: Option<String>,
    /// Its fields' values by key; a secret is never among them.
    pub values: BTreeMap<String, SettingValue>,
    /// The keys of its secret fields that hold a value.
    pub secrets_set: Vec<String>,
}

/// What can be done to a collection or to one of its records. `add`,
/// `save` and `delete` take the values and remove the record; the others
/// are the owner's (`set_default`, `list_models`, `connect`, `trust`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsAction {
    pub id: String,
    pub label: String,
    pub scope: SettingsActionScope,
    /// Asks before it runs.
    pub confirm: bool,
    /// For an `add` whose new record the user names (an MCP server, a
    /// profile, a project), what the name is asked by; `None` when the
    /// owner names it.
    pub names_record: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SettingsActionScope {
    Collection,
    Record,
}

/// An action to run on a collection, or on one of its records. A host's
/// frontend sends it, so it is strict.
#[derive(Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsActionRequest {
    pub collection: String,
    /// The record it runs on; for `add` in a collection whose records the
    /// user names (an MCP server, a profile), the new record's id.
    #[serde(default)]
    pub record: Option<String>,
    pub action: String,
    /// The collection's revision as it was read, for a collection that has
    /// one: a write over a revision that moved is refused.
    #[serde(default)]
    pub expected_revision: Option<u64>,
    /// The fields `add` and `save` write, keys relative to the record, each
    /// checked against its spec; a field left out keeps its value (a secret
    /// included), and `null` unsets an optional one.
    #[serde(default)]
    pub values: Map<String, Value>,
}

/// The values may hold a secret: only their keys are shown.
impl std::fmt::Debug for SettingsActionRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SettingsActionRequest")
            .field("collection", &self.collection)
            .field("record", &self.record)
            .field("action", &self.action)
            .field("expected_revision", &self.expected_revision)
            .field("values", &self.values.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// What an action did.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsActionOutcome {
    /// The record it made, for `add`.
    pub record: Option<String>,
    /// What it found, for an action that reads (`list_models`).
    pub message: Option<String>,
}

impl AgentApplicationService {
    /// Every setting: the configuration's sections, the plugins' options
    /// and the collections, read now. A host's configuration has neither
    /// the standalone sections nor the profiles and projects.
    pub async fn settings_catalog(&self) -> Result<SettingsCatalog, String> {
        let config = self.config.clone();
        let mut sections = hanafubuki_agent_foundation::blocking::run_blocking_operation(
            self.blocking.as_ref(),
            "agent settings catalog worker",
            move || config.settings_sections(),
        )
        .await??;
        let statuses = self.plugin_status();
        sections.extend(self.plugin_option_sections(&statuses).await?);
        if let Ok(packages) = self.packages().await {
            sections.extend(package_settings::sections(&packages));
        }
        let mut collections = vec![
            providers::collection(self).await,
            provider_models::collection(self).await,
            model_policy::collection(self).await,
            mcp::collection(self).await,
            mcp_secrets::collection(self).await,
            plugin_mcp::collection(self).await,
            plugins::collection(&statuses),
            packages::collection(self).await,
            marketplaces::collection(self).await,
            hooks::collection(self).await,
            roles::collection(self).await,
        ];
        if self.config.is_layered() {
            collections.push(profiles::collection(self).await);
            collections.push(projects::collection(self).await);
        }
        Ok(SettingsCatalog {
            sections,
            collections,
        })
    }

    /// Writes settings by key, as a whole or not at all: a setting of
    /// `config.toml` or a plugin's option (`pluginOptions.<plugin>.<key>`),
    /// each checked against its spec
    /// ([`hanafubuki_agent_config::AgentConfig::write_values`]). The
    /// plugins then take the settings, and a memory scan is signalled. A
    /// code plugin's settings (`plugins."<id>".settings.<key>`) are written
    /// on their own, one plugin's at a time, through
    /// [`Self::set_package_settings`].
    pub async fn write_setting_values(
        &self,
        write: SettingsWrite,
    ) -> Result<SettingsWriteOutcome, String> {
        if let Some(outcome) = package_settings::write(self, &write).await {
            return outcome;
        }
        let config = self.config.clone();
        let options = self.host.option_schemas();
        let outcome = hanafubuki_agent_foundation::blocking::run_blocking_operation(
            self.blocking.as_ref(),
            "agent settings values write worker",
            move || config.write_values_with_options(&write, &options),
        )
        .await??;
        self.apply_settings_to_plugins();
        self.memory.signal_scan(None);
        Ok(outcome)
    }

    /// Runs `request.action` on a collection of the catalog, through the
    /// calls of the collection's owner.
    pub async fn run_settings_action(
        &self,
        request: SettingsActionRequest,
    ) -> Result<SettingsActionOutcome, String> {
        match request.collection.as_str() {
            providers::ID => providers::run(self, request).await,
            provider_models::ID => provider_models::run(self, request).await,
            model_policy::ID => model_policy::run(self, request).await,
            mcp::ID => mcp::run(self, request).await,
            mcp_secrets::ID => mcp_secrets::run(self, request).await,
            plugin_mcp::ID => plugin_mcp::run(self, request).await,
            plugins::ID => plugins::run(self, request).await,
            packages::ID => packages::run(self, request).await,
            marketplaces::ID => marketplaces::run(self, request).await,
            hooks::ID => hooks::run(self, request).await,
            roles::ID => roles::run(self, request).await,
            profiles::ID if self.config.is_layered() => profiles::run(self, request).await,
            projects::ID if self.config.is_layered() => projects::run(self, request).await,
            other => Err(format!("there is no settings collection `{other}`")),
        }
    }

    /// A section per plugin row that has options, in mount order, its keys
    /// the options' full keys (`pluginOptions.<row>.<key>`).
    async fn plugin_option_sections(
        &self,
        statuses: &[AgentPluginStatus],
    ) -> Result<Vec<SettingsSection>, String> {
        let config = self.config.clone();
        let sources = hanafubuki_agent_foundation::blocking::run_blocking_operation(
            self.blocking.as_ref(),
            "agent settings sources worker",
            move || config.setting_sources(),
        )
        .await??;
        let sections = statuses
            .iter()
            .filter_map(|status| Some((status, status.options.as_ref()?)))
            .enumerate()
            .map(|(index, (status, options))| {
                let key = |option: &str| format!("pluginOptions.{}.{option}", status.name);
                let settings = options
                    .schema
                    .options
                    .iter()
                    .map(|spec| SettingSpec {
                        key: key(&spec.key),
                        ..spec.clone()
                    })
                    .collect();
                let values = options
                    .values
                    .iter()
                    .map(|option| {
                        let key = key(&option.key);
                        let mention = format!(": {key} ");
                        SettingState {
                            value: option.value.clone(),
                            source: sources.get(&key).cloned().unwrap_or(SettingSource::Default),
                            issue: options
                                .issues
                                .iter()
                                .find(|issue| issue.contains(&mention))
                                .cloned(),
                            key,
                        }
                    })
                    .collect();
                SettingsSection {
                    id: format!("pluginOptions.{}", status.name),
                    title: status.name.clone(),
                    group: SettingsGroup::Plugins,
                    order: u16::try_from(index * 10).unwrap_or(u16::MAX),
                    settings,
                    values,
                }
            })
            .collect();
        Ok(sections)
    }
}

impl SettingsCollection {
    fn new(id: &str, title: &str, group: SettingsGroup, record: Vec<SettingSpec>) -> Self {
        Self {
            id: id.to_string(),
            title: title.to_string(),
            group,
            record,
            records: Vec::new(),
            actions: Vec::new(),
            revision: None,
            issues: Vec::new(),
        }
    }

    fn with_actions(mut self, actions: Vec<SettingsAction>) -> Self {
        self.actions = actions;
        self
    }

    /// The collection whose owner could not be read: no records, and why.
    fn unavailable(mut self, error: impl ToString) -> Self {
        self.issues.push(error.to_string());
        self
    }
}

impl SettingsAction {
    fn record(id: &str, label: &str) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
            scope: SettingsActionScope::Record,
            confirm: false,
            names_record: None,
        }
    }

    fn collection(id: &str, label: &str) -> Self {
        Self {
            scope: SettingsActionScope::Collection,
            ..Self::record(id, label)
        }
    }

    fn confirmed(self) -> Self {
        Self {
            confirm: true,
            ..self
        }
    }

    /// An `add` whose new record is named `label` by the user.
    fn named(self, label: &str) -> Self {
        Self {
            names_record: Some(label.to_string()),
            ..self
        }
    }
}

const ADD: &str = "add";
const SAVE: &str = "save";
const DELETE: &str = "delete";

impl SettingsActionRequest {
    /// The record it names, which the action needs.
    fn record_id(&self) -> Result<&str, String> {
        self.record
            .as_deref()
            .ok_or_else(|| format!("`{}` runs on a record: name it", self.action))
    }

    /// The revision it was read at, which a write of its collection needs.
    fn revision(&self) -> Result<u64, String> {
        self.expected_revision.ok_or_else(|| {
            format!(
                "a write of `{}` names the revision it was read at",
                self.collection
            )
        })
    }

    fn unknown_action<T>(&self) -> Result<T, String> {
        Err(format!(
            "the settings collection `{}` has no action `{}`",
            self.collection, self.action
        ))
    }
}

/// A value's serialized name (`active`, `chat_completions`).
fn name_of<T: Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(Value::String(name)) => name,
        _ => String::new(),
    }
}
