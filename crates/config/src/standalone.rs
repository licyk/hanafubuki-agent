//! The settings only the standalone Agent reads from `config.toml`: what a
//! host supplies itself when it embeds the Agent (its proxy, its logger,
//! its interface). The settings API never writes them; `config set`
//! ([`crate::AgentConfig::set_user_value`]), the terminal interface
//! ([`crate::AgentConfig::write_tui_preferences`]) or a person does.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::fields::{read_object, Fields};
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
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

/// The standalone settings `object` holds, each missing or invalid one
/// taking its value in `defaults`.
pub(crate) fn decode_standalone(
    object: &Map<String, Value>,
    defaults: &AgentStandaloneSettings,
    repairs: &mut Vec<String>,
) -> AgentStandaloneSettings {
    let mut fields = Fields::new(object, repairs);
    let network = read_object(
        &mut fields,
        "network",
        &defaults.network,
        |fields, network| {
            network.proxy = fields.read("proxy", network.proxy.clone());
        },
    );
    let log_level = fields.read("logLevel", defaults.log_level);
    let tui = read_tui(&mut fields, &defaults.tui);
    AgentStandaloneSettings {
        network,
        log_level,
        tui,
    }
}
