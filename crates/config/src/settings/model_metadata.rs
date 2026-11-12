//! `[modelMetadata]`: how the Agent learns what the models it reaches can
//! do.

use hanafubuki_agent_foundation::settings::{SettingChoice, SettingKind, SettingSpec};
use serde::{Deserialize, Serialize};

use crate::sections::{described, SettingsGroup, SettingsSection};

/// Whether the Agent lists a Provider's models by itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentModelDiscovery {
    /// In the background, at most once a day per Provider: at start for
    /// the default Provider, and when a session's Provider or model is
    /// selected.
    #[default]
    Auto,
    /// Never by itself. The listings a person asks for (the model picker,
    /// `/model`, a Provider's test, `doctor --online`) are still recorded.
    Off,
}

/// Strict as the settings patch that carries it (`agent_settings_write`);
/// the file is read setting by setting, by the specs of [`section`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentModelMetadataSettings {
    pub discovery: AgentModelDiscovery,
    /// Whether the Agent downloads models.dev's public catalogue (once a
    /// day, from this machine) to fill what nothing else says of a model.
    /// Off by default: it is a request to a third party.
    pub models_dev: bool,
}

pub(crate) fn section() -> SettingsSection {
    SettingsSection::new(
        "modelMetadata",
        "Model metadata",
        SettingsGroup::Model,
        10,
        vec![
            described(
                SettingSpec::new(
                    "modelMetadata.discovery",
                    "Model discovery",
                    SettingKind::Choice {
                        values: vec![
                            SettingChoice::new("auto", "Automatic"),
                            SettingChoice::new("off", "Off"),
                        ],
                    },
                ),
                "Whether the Agent lists its Providers' models by itself, in the background",
            ),
            SettingSpec {
                keywords: vec!["catalogue".into(), "catalog".into()],
                ..described(
                    SettingSpec::new(
                        "modelMetadata.modelsDev",
                        "models.dev catalogue",
                        SettingKind::Boolean,
                    ),
                    "Whether the Agent downloads models.dev's public catalogue, about once a day",
                )
            },
        ],
    )
}
