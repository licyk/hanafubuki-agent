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

use hanafubuki_agent_foundation::settings::SettingSpec;
use serde::Serialize;

pub(crate) use read::{check_specs, no_rule, LayerRead, SettingsRead};

use crate::keys::{known_settings, lookup};
use crate::settings::{compaction, memory, model_metadata, sandbox, web};
use crate::{settings, standalone, tui};

/// One heading of settings, in the order a settings page lists them.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSection {
    pub id: String,
    pub title: String,
    pub settings: Vec<SettingSpec>,
}

impl SettingsSection {
    pub(crate) fn new(id: &str, title: &str, settings: Vec<SettingSpec>) -> Self {
        Self {
            id: id.to_string(),
            title: title.to_string(),
            settings,
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
    vec![
        settings::general_section(),
        sandbox::section(),
        memory::section(),
        web::section(),
        compaction::section(),
        model_metadata::section(),
        standalone::network_section(),
        tui::section(),
    ]
}
