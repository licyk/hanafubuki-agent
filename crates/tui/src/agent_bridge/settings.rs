// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/warnings`, `/memories` and `/config` onto the facade's calls: the
//! warnings the Agent keeps (the configuration's recoveries and layers, the
//! plugins, MCP, the hooks, the panics), memory's settings, status, the
//! session's contribution and the reset, and the compaction strategy. What a
//! call found comes back as a [`SettingsReply`]; a failure names the
//! command.

use std::time::{SystemTime, UNIX_EPOCH};

use hanafubuki_agent::panic::recorded_panics;
use hanafubuki_agent::{
    AgentCompactionStrategy, AgentConfigProblem, AgentConfigRecovery, AgentHandle,
    AgentMemoryContributionMode, AgentMemorySettings, AgentMemoryStatus, AgentPluginState,
    AgentSessionId, AgentSettingsPatch,
};

use super::{AgentBridge, AgentReply, TurnUpdate};

/// Where a warning comes from, as `/warnings` counts them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum WarningSource {
    Configuration,
    Plugins,
    Mcp,
    Hooks,
    Panics,
    Compaction,
    Interface,
}

impl WarningSource {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Configuration => "configuration",
            Self::Plugins => "plugins",
            Self::Mcp => "MCP",
            Self::Hooks => "hooks",
            Self::Panics => "panics",
            Self::Compaction => "compaction",
            Self::Interface => "interface",
        }
    }
}

/// One warning: where from, what it says, and when (the time it was
/// found, for a condition that holds now).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Warning {
    pub(crate) source: WarningSource,
    pub(crate) text: String,
    pub(crate) at_unix_ms: u64,
}

impl Warning {
    pub(crate) fn new(source: WarningSource, text: String) -> Self {
        Self {
            source,
            text,
            at_unix_ms: now_unix_ms(),
        }
    }

    /// What dismissing it is remembered by.
    pub(crate) fn key(&self) -> String {
        format!("{}\n{}", self.source.name(), self.text)
    }
}

/// Memory as `/memories` shows it.
#[derive(Clone, Debug)]
pub(crate) struct MemoriesState {
    pub(crate) settings: AgentMemorySettings,
    /// The store's status, or why it could not be read.
    pub(crate) status: Result<AgentMemoryStatus, String>,
    /// The session's contribution and its revision; `None` with no session.
    pub(crate) contribution: Option<(AgentMemoryContributionMode, u64)>,
    /// What the last change did, for the panel's note.
    pub(crate) note: Option<String>,
}

/// What `/warnings`, `/memories` and `/config` ask of the Agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SettingsCall {
    Warnings,
    Memories,
    /// The settings `/config` shows beside the interface's own.
    Config,
    /// Saves `compaction.strategy`.
    SetCompactionStrategy(AgentCompactionStrategy),
    /// Saves memory's settings (`memory.*`).
    SetMemory(AgentMemorySettings),
    /// Turns the session's contribution on or off, at the revision read.
    SetContribution {
        enabled: bool,
        revision: u64,
    },
    /// Empties the memory store at the generation read.
    ResetMemory {
        generation: u64,
    },
}

impl SettingsCall {
    fn command(&self) -> &'static str {
        match self {
            Self::Warnings => "warnings",
            Self::Config | Self::SetCompactionStrategy(_) => "config",
            _ => "memories",
        }
    }
}

#[derive(Debug)]
pub(crate) enum SettingsReply {
    /// The Agent's warnings, for the interface to add its own to.
    Warnings(Vec<Warning>),
    Memories(Box<MemoriesState>),
    /// The compaction strategy as saved; `saved` when this call saved it.
    Config {
        compaction_strategy: AgentCompactionStrategy,
        saved: bool,
    },
    Failed {
        command: &'static str,
        error: String,
    },
}

impl AgentBridge {
    pub(super) fn settings_call(&mut self, call: SettingsCall) {
        let command = call.command();
        let session = self.session_id;
        self.spawn(move |agent| async move {
            let reply = run(&agent, session, call)
                .await
                .unwrap_or_else(|error| SettingsReply::Failed { command, error });
            Ok(Some(AgentReply::Update(Box::new(TurnUpdate::Settings(
                reply,
            )))))
        });
    }

    /// What the configuration's reads found, at once: for the count the
    /// interface shows at start.
    pub(crate) fn config_warnings(&self) -> Vec<Warning> {
        config_warnings(&self.agent)
    }
}

async fn run(
    agent: &AgentHandle,
    session: Option<AgentSessionId>,
    call: SettingsCall,
) -> Result<SettingsReply, String> {
    let note = match call {
        SettingsCall::Warnings => return Ok(SettingsReply::Warnings(agent_warnings(agent).await)),
        SettingsCall::Config => {
            return Ok(SettingsReply::Config {
                compaction_strategy: agent.settings_view().await?.settings.compaction.strategy,
                saved: false,
            })
        }
        SettingsCall::SetCompactionStrategy(strategy) => {
            let mut compaction = agent.settings_view().await?.settings.compaction;
            compaction.strategy = strategy;
            let saved = agent
                .write_settings(AgentSettingsPatch {
                    compaction: Some(compaction),
                    ..AgentSettingsPatch::default()
                })
                .await?;
            return Ok(SettingsReply::Config {
                compaction_strategy: saved.settings.compaction.strategy,
                saved: true,
            });
        }
        SettingsCall::Memories => None,
        SettingsCall::SetMemory(memory) => {
            agent
                .write_settings(AgentSettingsPatch {
                    memory: Some(memory),
                    ..AgentSettingsPatch::default()
                })
                .await?;
            None
        }
        SettingsCall::SetContribution { enabled, revision } => {
            let session = session.ok_or("there is no session yet")?;
            agent.set_memory_session_contribution(session, revision, enabled)?;
            None
        }
        SettingsCall::ResetMemory { generation } => {
            agent.reset_memory(generation)?;
            Some("Memory was reset: the memories and their sources are gone.".to_string())
        }
    };
    let settings = agent.settings_view().await?.settings.memory;
    let contribution = match session {
        Some(session) => Some(agent.memory_session_contribution(session)?),
        None => None,
    };
    Ok(SettingsReply::Memories(Box::new(MemoriesState {
        settings,
        status: agent.memory_status(),
        contribution,
        note,
    })))
}

/// Every warning the Agent holds now.
async fn agent_warnings(agent: &AgentHandle) -> Vec<Warning> {
    let mut warnings = config_warnings(agent);
    for plugin in agent.plugin_status() {
        let failed = (plugin.state == AgentPluginState::Failed).then(|| {
            plugin
                .error
                .clone()
                .unwrap_or_else(|| "it failed".to_string())
        });
        for problem in failed.into_iter().chain(plugin.warning.clone()) {
            let text = format!("Plugin `{}`: {problem}", plugin.name);
            warnings.push(Warning::new(WarningSource::Plugins, text));
        }
    }
    match agent.mcp_snapshot().await {
        Ok(snapshot) => {
            for repair in snapshot.settings_repairs {
                let text = format!("mcp.toml: {}", repair.detail);
                warnings.push(Warning::new(WarningSource::Mcp, text));
            }
            for summary in snapshot.diagnostics {
                if let Some(entry) = summary.headline {
                    let text = format!(
                        "MCP server `{}`: {} {}",
                        summary.server_id.as_str(),
                        entry.message,
                        entry.next_step
                    );
                    warnings.push(Warning {
                        source: WarningSource::Mcp,
                        text: text.trim_end().to_string(),
                        at_unix_ms: entry.timestamp_unix_ms,
                    });
                }
            }
        }
        Err(error) => warnings.push(Warning::new(
            WarningSource::Mcp,
            format!("The MCP servers' state could not be read: {error}"),
        )),
    }
    match agent.hooks_list().await {
        Ok(hooks) => warnings.extend(
            hooks
                .diagnostics
                .into_iter()
                .map(|diagnostic| Warning::new(WarningSource::Hooks, diagnostic)),
        ),
        Err(error) => warnings.push(Warning::new(
            WarningSource::Hooks,
            format!("The hooks could not be read: {error}"),
        )),
    }
    warnings.extend(recorded_panics().recent.into_iter().map(|fault| Warning {
        source: WarningSource::Panics,
        text: format!(
            "A panic in `{}` at {}: {}",
            fault.thread, fault.location, fault.message
        ),
        at_unix_ms: fault.at_unix_millis,
    }));
    warnings
}

/// The configuration's recoveries and what its layers could not use.
fn config_warnings(agent: &AgentHandle) -> Vec<Warning> {
    let mut warnings: Vec<Warning> = agent
        .config_recoveries()
        .iter()
        .flat_map(recovery_warnings)
        .collect();
    match agent.config_layer_warnings() {
        Ok(layers) => warnings.extend(
            layers
                .into_iter()
                .map(|text| Warning::new(WarningSource::Configuration, text)),
        ),
        Err(error) => warnings.push(Warning::new(
            WarningSource::Configuration,
            format!("The configuration could not be read: {error}"),
        )),
    }
    warnings
}

fn recovery_warnings(recovery: &AgentConfigRecovery) -> Vec<Warning> {
    let file = recovery.file.display();
    let read_only = recovery
        .read_only
        .as_ref()
        .map(|reason| format!(" It is read-only until the Agent restarts: {reason}."))
        .unwrap_or_default();
    let texts = match &recovery.problem {
        AgentConfigProblem::Unusable { reason } => vec![format!(
            "{file} could not be read ({reason}); the defaults are used, and the file is kept \
             in .recovery.{read_only}"
        )],
        AgentConfigProblem::Repaired { repairs } => repairs
            .iter()
            .map(|repair| format!("{file} was repaired: {repair}.{read_only}"))
            .collect(),
    };
    texts
        .into_iter()
        .map(|text| Warning {
            source: WarningSource::Configuration,
            text,
            at_unix_ms: recovery.recorded_at_unix_ms,
        })
        .collect()
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}
