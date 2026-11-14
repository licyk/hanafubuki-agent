//! The `subagents` plugin's options (`[pluginOptions.subagents]`): how many
//! agents across one tree run at once and how long `wait_agent` waits. The
//! bounds in code stay hard limits; the options choose within them.

use std::time::Duration;

use hanafubuki_agent_config::{PluginOptionSchema, SettingKind, SettingSpec, SettingValue};
use serde::Deserialize;

/// The most agents across one tree the option lets run at once.
const MAXIMUM_RUNNING_AGENTS_LIMIT: i64 = 16;
/// The wait bounds a `wait_agent` timeout is clamped to, in seconds: what
/// the options choose within.
const WAIT_SECONDS_LIMITS: (i64, i64) = (10, 300);
/// Live agents include parents waiting for their own background work.
const DEFAULT_RUNNING_AGENTS: i64 = 8;
const DEFAULT_WAIT_SECONDS: i64 = 30;

/// The configuration the row derives from the settings: every option, as
/// the boundary checked it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct SubagentsOptions {
    maximum_running_agents: u64,
    default_wait_seconds: u64,
    minimum_wait_seconds: u64,
    maximum_wait_seconds: u64,
}

impl Default for SubagentsOptions {
    fn default() -> Self {
        Self {
            maximum_running_agents: DEFAULT_RUNNING_AGENTS as u64,
            default_wait_seconds: DEFAULT_WAIT_SECONDS as u64,
            minimum_wait_seconds: WAIT_SECONDS_LIMITS.0 as u64,
            maximum_wait_seconds: WAIT_SECONDS_LIMITS.1 as u64,
        }
    }
}

/// What the plugin runs with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SubagentLimits {
    pub(super) maximum_running_agents: usize,
    pub(super) waits: WaitBounds,
}

/// What `wait_agent`'s spec says, so a change of them restarts the plugin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct WaitBounds {
    pub(super) default: Duration,
    pub(super) minimum: Duration,
    pub(super) maximum: Duration,
}

impl SubagentLimits {
    /// Each option is in its bounds already; between them, a minimum above
    /// the maximum is taken down to it and the default into them.
    pub(super) fn of(options: &SubagentsOptions) -> Self {
        let maximum = options.maximum_wait_seconds;
        let minimum = options.minimum_wait_seconds.min(maximum);
        let default = options.default_wait_seconds.clamp(minimum, maximum);
        Self {
            maximum_running_agents: usize::try_from(options.maximum_running_agents)
                .unwrap_or(usize::MAX),
            waits: WaitBounds {
                default: Duration::from_secs(default),
                minimum: Duration::from_secs(minimum),
                maximum: Duration::from_secs(maximum),
            },
        }
    }
}

/// The options' schema, which the row's descriptor carries.
pub(crate) fn subagents_option_schema() -> PluginOptionSchema {
    let integer =
        |key: &str, label: &str, summary: &str, (minimum, maximum), default| SettingSpec {
            summary: summary.to_string(),
            default: Some(SettingValue::Integer(default)),
            ..SettingSpec::new(
                key,
                label,
                SettingKind::Integer {
                    minimum: Some(minimum),
                    maximum: Some(maximum),
                },
            )
        };
    PluginOptionSchema {
        options: vec![
            integer(
                "maximumRunningAgents",
                "Running sub-agents",
                "How many sub-agents work at once under one conversation, theirs included",
                (1, MAXIMUM_RUNNING_AGENTS_LIMIT),
                DEFAULT_RUNNING_AGENTS,
            ),
            integer(
                "defaultWaitSeconds",
                "Default wait",
                "How long wait_agent waits when the model gives no timeout (seconds)",
                WAIT_SECONDS_LIMITS,
                DEFAULT_WAIT_SECONDS,
            ),
            integer(
                "minimumWaitSeconds",
                "Shortest wait",
                "The shortest wait_agent timeout (seconds)",
                WAIT_SECONDS_LIMITS,
                WAIT_SECONDS_LIMITS.0,
            ),
            integer(
                "maximumWaitSeconds",
                "Longest wait",
                "The longest wait_agent timeout (seconds)",
                WAIT_SECONDS_LIMITS,
                WAIT_SECONDS_LIMITS.1,
            ),
        ],
    }
}
