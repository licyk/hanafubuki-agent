//! The settings of `config.toml`, each declared beside the struct that holds
//! it (`settings/memory.rs` holds `AgentMemorySettings` and its section).
//!
//! A setting's key is the serde name path of its field
//! (`memory.maxSourceAgeDays` is `AgentSettings.memory.max_source_age_days`).
//! The struct's `Default` is the only source of defaults: [`config_sections`]
//! fills each spec's from the serialized defaults, and no spec states one.
//! A key is the string literal `SettingSpec::new` is given, so `cargo xtask
//! check-config-reference` can find it in the reference's tables.
//! The decoders read every key and bound from the specs ([`SettingsRead`]),
//! and a write checks a value against its spec before anything else
//! ([`crate::AgentConfig::write_values`]).
//!
//! The tables keyed by what the user added (`plugins`, `marketplaces`,
//! `pluginOptions`, `tui.keymap`, `tui.presets`) are collections, not
//! settings, and have no spec.

mod read;

use std::sync::LazyLock;

use hanafubuki_agent_foundation::settings::{SettingSpec, SettingValue};
use serde::Serialize;
use serde_json::{Map, Value};

pub(crate) use read::{check_specs, no_rule, LayerRead, SettingsRead};

use crate::keys::{known_settings, lookup};
use crate::settings::{compaction, memory, model_metadata, sandbox, web};
use crate::{settings, standalone, tui, AgentConfig, SettingSource};

/// One heading of settings, in the order a settings page lists them.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSection {
    pub id: String,
    pub title: String,
    /// The tab it is shown under.
    pub group: SettingsGroup,
    /// Its place among the sections of its group, lowest first.
    pub order: u16,
    pub settings: Vec<SettingSpec>,
    /// Each setting's value as the configuration resolves it, in the order
    /// of `settings`; empty in [`config_sections`], which reads none.
    pub values: Vec<SettingState>,
}

/// The tabs of a settings page, in their order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SettingsGroup {
    General,
    Model,
    Tools,
    Memory,
    Context,
    Network,
    Interface,
    Plugins,
    Advanced,
}

/// One setting's value as the configuration resolves it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingState {
    pub key: String,
    /// `None` when nothing sets it and it has no default.
    pub value: Option<SettingValue>,
    /// The layer the value comes from.
    pub source: SettingSource,
    /// Why a layer's value of it could not be used, when one could not.
    pub issue: Option<String>,
}

impl SettingsSection {
    pub(crate) fn new(
        id: &str,
        title: &str,
        group: SettingsGroup,
        order: u16,
        settings: Vec<SettingSpec>,
    ) -> Self {
        Self {
            id: id.to_string(),
            title: title.to_string(),
            group,
            order,
            settings,
            values: Vec::new(),
        }
    }
}

/// `spec` with its one-line summary.
pub(crate) fn described(spec: SettingSpec, summary: &str) -> SettingSpec {
    SettingSpec {
        summary: summary.into(),
        ..spec
    }
}

/// Every section of `config.toml`, each setting's default filled from the
/// defaults of its struct (`None` for a setting unset by default).
pub fn config_sections() -> Vec<SettingsSection> {
    let defaults = known_settings().unwrap_or_default();
    let mut sections = sections();
    for spec in sections
        .iter_mut()
        .flat_map(|section| section.settings.iter_mut())
    {
        let path = spec.key.split('.').map(str::to_string).collect::<Vec<_>>();
        spec.default = lookup(&defaults, &path)
            .filter(|value| !value.is_null())
            .and_then(|value| spec.read(value).ok());
    }
    sections
}

/// The spec of the setting `key` of `config.toml`, a dotted key.
pub(crate) fn setting_spec(key: &str) -> Option<&'static SettingSpec> {
    specs().iter().find(|spec| spec.key == key)
}

/// Every spec, without defaults, for the decoders and the writer.
pub(crate) fn specs() -> &'static [SettingSpec] {
    static SPECS: LazyLock<Vec<SettingSpec>> = LazyLock::new(|| {
        sections()
            .into_iter()
            .flat_map(|section| section.settings)
            .collect()
    });
    &SPECS
}

fn sections() -> Vec<SettingsSection> {
    let mut sections = agent_sections();
    sections.extend(standalone_sections());
    sections
}

/// The sections every configuration reads.
fn agent_sections() -> Vec<SettingsSection> {
    vec![
        settings::general_section(),
        sandbox::section(),
        memory::section(),
        web::section(),
        compaction::section(),
        model_metadata::section(),
    ]
}

/// The sections only the standalone Agent reads: a host supplies its proxy,
/// its logger and its interface.
fn standalone_sections() -> Vec<SettingsSection> {
    vec![standalone::network_section(), tui::section()]
}

impl AgentConfig {
    /// The sections this configuration reads, each setting with its value
    /// as the layers resolve it, the layer it comes from and why a layer's
    /// value could not be used; a host's configuration leaves out the
    /// standalone sections ([`config_sections`] has them all).
    pub fn settings_sections(&self) -> Result<Vec<SettingsSection>, String> {
        let document = self.resolved_document()?;
        let resolved = self.resolved()?;
        let ids = self
            .is_layered()
            .then(standalone_sections)
            .into_iter()
            .flatten()
            .chain(agent_sections())
            .map(|section| section.id)
            .collect::<Vec<_>>();
        let mut sections = config_sections();
        sections.retain(|section| ids.contains(&section.id));
        for section in &mut sections {
            section.values = section
                .settings
                .iter()
                .map(|spec| SettingState {
                    key: spec.key.clone(),
                    value: value_of(&document, spec),
                    source: resolved
                        .sources
                        .get(&spec.key)
                        .cloned()
                        .unwrap_or(SettingSource::Default),
                    issue: resolved.issues.get(&spec.key).cloned(),
                })
                .collect();
        }
        Ok(sections)
    }
}

/// The value of `spec` in the resolved `document`, as its spec reads it.
fn value_of(document: &Map<String, Value>, spec: &SettingSpec) -> Option<SettingValue> {
    let path = spec.key.split('.').map(str::to_string).collect::<Vec<_>>();
    match lookup(document, &path)? {
        Value::Null => None,
        value => spec.read(value).ok(),
    }
}
