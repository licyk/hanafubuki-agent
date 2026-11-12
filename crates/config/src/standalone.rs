//! The settings only the standalone Agent reads from `config.toml`: what a
//! host supplies itself when it embeds the Agent (its proxy, its logger,
//! its interface). The settings API never writes them; `config set`
//! ([`crate::AgentConfig::set_user_value`]), the terminal interface
//! ([`crate::AgentConfig::write_tui_preferences`]) or a person does.

use hanafubuki_agent_foundation::settings::{
    SettingApply, SettingChoice, SettingKind, SettingSpec,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::sections::{
    described, no_rule, LayerRead, SettingsGroup, SettingsRead, SettingsSection,
};
use crate::tui::read_tui;
use crate::{AgentProxy, AgentTuiPreferences};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStandaloneSettings {
    /// `[network]`.
    pub network: AgentNetworkSettings,
    /// The level of the standalone log file (`logLevel`).
    pub log_level: AgentLogLevel,
    /// `[tui]`.
    pub tui: AgentTuiPreferences,
}

/// A change to the standalone settings from the configuration center: each
/// field present is replaced, and the write changes only those keys.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentStandaloneSettingsPatch {
    pub network: Option<AgentNetworkSettings>,
    pub log_level: Option<AgentLogLevel>,
}

impl AgentStandaloneSettingsPatch {
    pub(crate) fn document(&self) -> Map<String, Value> {
        let mut table = Map::new();
        if let Some(network) = &self.network {
            table.insert(
                "network".into(),
                serde_json::json!({ "proxy": network.proxy }),
            );
        }
        if let Some(level) = self.log_level {
            table.insert("logLevel".into(), Value::String(level.as_str().to_string()));
        }
        table
    }
}

impl Default for AgentStandaloneSettings {
    fn default() -> Self {
        Self {
            network: AgentNetworkSettings {
                proxy: AgentProxy::System,
            },
            log_level: AgentLogLevel::Info,
            tui: AgentTuiPreferences::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentNetworkSettings {
    pub proxy: AgentProxy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentLogLevel {
    Off,
    Error,
    Warn,
    Info,
    Debug,
    /// The only level that records model and tool content.
    Trace,
}
impl AgentLogLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Trace => "trace",
        }
    }
}

impl From<AgentLogLevel> for log::LevelFilter {
    fn from(level: AgentLogLevel) -> Self {
        match level {
            AgentLogLevel::Off => Self::Off,
            AgentLogLevel::Error => Self::Error,
            AgentLogLevel::Warn => Self::Warn,
            AgentLogLevel::Info => Self::Info,
            AgentLogLevel::Debug => Self::Debug,
            AgentLogLevel::Trace => Self::Trace,
        }
    }
}

pub(crate) fn network_section() -> SettingsSection {
    SettingsSection::new(
        "network",
        "Network and logging",
        SettingsGroup::Network,
        10,
        vec![
            SettingSpec {
                help: Some(
                    "\"system\", \"disabled\", or a proxy URL (http, https, socks4, socks4a, socks5, socks5h)"
                        .into(),
                ),
                keywords: vec!["proxy".into()],
                ..described(
                    SettingSpec::new(
                        "network.proxy",
                        "Proxy",
                        SettingKind::Text {
                            maximum_chars: None,
                            secret: false,
                            nullable: false,
                        },
                    ),
                    "The proxy of the Providers, the web tools, MCP and the processes the Agent starts",
                )
            },
            SettingSpec {
                apply: SettingApply::NextLaunch,
                keywords: vec!["log".into()],
                ..described(
                    SettingSpec::new(
                        "logLevel",
                        "Log level",
                        SettingKind::Choice {
                            values: [
                                ("off", "Off"),
                                ("error", "Error"),
                                ("warn", "Warn"),
                                ("info", "Info"),
                                ("debug", "Debug"),
                                ("trace", "Trace"),
                            ]
                            .into_iter()
                            .map(|(value, label)| SettingChoice::new(value, label))
                            .collect(),
                        },
                    ),
                    "The log file's level; only trace records model and tool content",
                )
            },
        ],
    )
}

/// The standalone settings `object` holds, each read by its spec, a missing
/// or invalid one taking its value in `defaults`.
pub(crate) fn decode_standalone(
    object: &Map<String, Value>,
    defaults: &AgentStandaloneSettings,
    report: &mut LayerRead,
) -> AgentStandaloneSettings {
    let mut read = SettingsRead::new(object, report);
    let network = read.read("network", &defaults.network, no_rule);
    let log_level = read.read("logLevel", &defaults.log_level, no_rule);
    let tui = read_tui(&mut read, &defaults.tui);
    AgentStandaloneSettings {
        network,
        log_level,
        tui,
    }
}
