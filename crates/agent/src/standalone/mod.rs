//! The Agent without a host: what the `hanafubuki-agent` binary runs.
//!
//! [`AgentRuntime::standalone`] composes the same slot, launch and service a
//! host composes ([`DesktopAgent`]), with a default for every host port: the
//! working directory, its Git root and the added directories as the
//! workspace, no host plugins, context or instructions, the configuration's
//! `[network]` proxy, the operating system's keyring for secrets, and sinks
//! that drop what they are told until a consumer (the TUI, `exec`) brings
//! its own. The configuration is layered ([`ConfigLayers`]); the data
//! directory's lock is taken by the first start, as for a host. The
//! configured proxy reaches the Agent's processes too.

mod keyring;
mod workspace;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hanafubuki_agent_config::{AgentDirs, ConfigLayers, ConfigOverrides};
use hanafubuki_agent_foundation::background_worker::WorkerShutdownDiagnostic;
use hanafubuki_agent_foundation::environment::ChildProcessEnvironment;
use hanafubuki_agent_foundation::secrets::SecretStore;
use hanafubuki_agent_mcp::{McpEventSink, NoopMcpEventSink};
use parking_lot::Mutex;

use self::keyring::KeyringSecretStore;
pub use self::keyring::{probe_keyring, KEYRING_SERVICE};
use self::workspace::StandaloneWorkspace;
use crate::{
    AgentApplicationService, AgentBackgroundEventSink, AgentConfig, AgentConstructor, AgentHandle,
    AgentHostInstructions, AgentHostPorts, AgentLaunch, AgentPluginEventSink, AgentProxy,
    AgentServiceInputs, AgentSlot, AgentStartMarker, AgentStartOptions, AgentStatusEventSink,
    AgentStoragePaths, AgentUnavailable, AgentWorkspaceRoots, DesktopAgent, NetworkSettingsPort,
    NoHostContext, NoopAgentBackgroundEventSink, NoopAgentPluginEventSink,
    NoopAgentStatusEventSink, SkillRoots, WorkspaceRootsPort,
};

/// Where and how the standalone Agent runs.
#[derive(Clone, Debug)]
pub struct AgentRuntimeOptions {
    pub dirs: AgentDirs,
    /// A directory of System Skills to use as they are, in place of the
    /// ones the Agent embeds and publishes into its data directory.
    pub bundled_skills: Option<PathBuf>,
    /// The working directory: the first workspace root, and with its Git
    /// root the project whose configuration may be read.
    pub cwd: PathBuf,
    /// More workspace roots (`--add-dir`).
    pub add_dirs: Vec<PathBuf>,
    /// The profile of the user's `config.toml` (`-p`).
    pub profile: Option<String>,
    /// `-c key=value`.
    pub overrides: ConfigOverrides,
    /// How the first start runs (safe mode, a rebuilt history index).
    pub start: AgentStartOptions,
}

/// What a standalone consumer replaces: the secret store (the keyring when
/// `None`) and the sinks the Agent tells about its state, its plugins, its
/// MCP servers and the sessions to wake for their background work. Turn
/// and session events go to the sink each call is given.
pub struct AgentRuntimeHost {
    pub secrets: Option<Arc<dyn SecretStore>>,
    pub status: Arc<dyn AgentStatusEventSink>,
    pub plugin_events: Arc<dyn AgentPluginEventSink>,
    pub mcp_events: Arc<dyn McpEventSink>,
    pub background: Arc<dyn AgentBackgroundEventSink>,
}

impl Default for AgentRuntimeHost {
    fn default() -> Self {
        Self {
            secrets: None,
            status: Arc::new(NoopAgentStatusEventSink),
            plugin_events: Arc::new(NoopAgentPluginEventSink),
            mcp_events: Arc::new(NoopMcpEventSink),
            background: Arc::new(NoopAgentBackgroundEventSink),
        }
    }
}

/// The standalone Agent: its slot (start, restart, status, the running
/// Agent's handle) and its configuration.
pub struct AgentRuntime {
    agent: DesktopAgent,
    workspace: Arc<StandaloneWorkspace>,
    config: AgentConfig,
    storage: AgentStoragePaths,
    launch: Mutex<Option<AgentLaunch>>,
}

impl AgentRuntime {
    /// Composes the Agent and reads its configuration, so a bad profile or
    /// working directory fails here; nothing starts until [`Self::start`].
    pub fn standalone(
        options: AgentRuntimeOptions,
        host: AgentRuntimeHost,
    ) -> Result<Self, String> {
        let workspace = Arc::new(StandaloneWorkspace::new(&options.cwd, &options.add_dirs)?);
        let layers = ConfigLayers {
            profile: options.profile,
            project: Some(workspace.project()),
            overrides: options.overrides,
        };
        let config_dir = options.dirs.config_dir;
        let config = AgentConfig::layered(&config_dir, layers.clone());
        config.settings()?;
        let storage = AgentStoragePaths::under(options.dirs.data_dir);
        let skill_roots = SkillRoots {
            system: options
                .bundled_skills
                .clone()
                .unwrap_or_else(|| storage.skills_root.join("system")),
            user: storage.skills_root.join("packages"),
            install_bundled_system_skills: options.bundled_skills.is_none(),
            state_file: storage.skill_state_file.clone(),
        };
        let roots: Arc<dyn WorkspaceRootsPort> = workspace.clone();
        let marker = AgentStartMarker::new(storage.start_marker_file.clone())
            .with_crash_record(storage.crash_record_file.clone());
        // A command line that starts the Agent asks for it, so an earlier
        // run's unfinished starts never keep it off.
        let slot = AgentSlot::new(None, host.status).starting_with(options.start);
        let agent = DesktopAgent::new(slot, marker, HashMap::new())?;
        let processes = agent.child_environment();
        ConfiguredNetwork {
            config: config.clone(),
            processes: processes.clone(),
        }
        .proxy()?;
        let storage_paths = storage.clone();
        let (secrets, plugin_events, mcp_events, background) = (
            host.secrets,
            host.plugin_events,
            host.mcp_events,
            host.background,
        );
        let construct: AgentConstructor = Arc::new(move |runtime, instance| {
            // Read again for every instance, as a host's is.
            let config = AgentConfig::layered(&config_dir, layers.clone());
            let blocking = runtime.blocking_port();
            let secrets = secrets
                .clone()
                .unwrap_or_else(|| KeyringSecretStore::shared(blocking.clone()));
            AgentApplicationService::open(AgentServiceInputs {
                config: config.clone(),
                storage_paths: storage_paths.clone(),
                skill_roots: skill_roots.clone(),
                blocking,
                mcp_event_sink: mcp_events.clone(),
                plugin_event_sink: plugin_events.clone(),
                background_event_sink: background.clone(),
                host: AgentHostPorts {
                    plugins: Vec::new(),
                    workspace: roots.clone(),
                    steering_context: Arc::new(NoHostContext),
                    instructions: AgentHostInstructions::default(),
                    network: Arc::new(ConfiguredNetwork {
                        config,
                        processes: processes.clone(),
                    }),
                    secrets,
                },
                instance,
            })
        });
        let launch = AgentLaunch {
            construct,
            process_groups_file: storage.process_groups_file.clone(),
        };
        Ok(Self {
            agent,
            workspace,
            config,
            storage,
            launch: Mutex::new(Some(launch)),
        })
    }

    /// Starts the Agent, once; returns at once ([`Self::wait_until_started`]).
    pub fn start(&self) {
        if let Some(launch) = self.launch.lock().take() {
            self.agent.start(launch);
        }
    }

    /// The running Agent once its start settled, or why it does not run (the
    /// data directory in use by another process, among others).
    pub fn wait_until_started(&self, timeout: Duration) -> Result<AgentHandle, AgentUnavailable> {
        self.agent
            .slot()
            .wait_until_started(Instant::now() + timeout)
    }

    pub fn slot(&self) -> &AgentSlot {
        self.agent.slot()
    }

    /// The layered configuration, as it was read when the runtime was made.
    pub fn config(&self) -> &AgentConfig {
        &self.config
    }

    #[cfg(test)]
    pub(crate) fn child_environment_for_test(&self) -> HashMap<String, Option<String>> {
        self.agent.child_environment_for_test()
    }

    /// Makes `path` (relative to the current working directory, or
    /// absolute) the working directory: the first workspace root, with its
    /// Git root and the added directories after it. The running Agent's
    /// sessions stay live: the filesystem tools and commands work in the new
    /// roots at once, and each session sees them in its next turn's
    /// context. Refused, with nothing changed, while a turn, a compaction
    /// or a background terminal runs. The configuration's project layer
    /// stays the one read at start. Blocks the caller.
    ///
    /// Standalone only: an embedded host supplies the workspace through its
    /// own `WorkspaceRootsPort`.
    pub fn set_working_directory(&self, path: &Path) -> Result<AgentWorkspaceRoots, String> {
        let moved = self.workspace.moved_to(path)?;
        if let Ok(agent) = self.agent.slot().agent() {
            agent.move_workspace(moved.roots())?;
        }
        let roots = moved.roots().clone();
        self.workspace.apply(moved);
        Ok(roots)
    }

    pub fn storage_paths(&self) -> &AgentStoragePaths {
        &self.storage
    }

    /// Shuts the Agent down within `budget` (`AGENT_SHUTDOWN_BUDGET`) and
    /// releases the data directory; blocks the caller, which is outside any
    /// async context.
    pub fn shutdown(&self, budget: Duration) -> Vec<WorkerShutdownDiagnostic> {
        self.agent.shutdown(budget)
    }
}

/// The proxy variables a process reads, upper and lower case: a process
/// that inherits this process's environment (an MCP stdio server) must not
/// keep a lower-case one the configured proxy overrides. Windows names are
/// not case-sensitive.
const PROXY_VARIABLES: &[&str] = if cfg!(windows) {
    &["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"]
} else {
    &[
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ]
};

const NO_PROXY_VARIABLES: &[&str] = if cfg!(windows) {
    &["NO_PROXY"]
} else {
    &["NO_PROXY", "no_proxy"]
};

/// The configured proxy in the environment of the Agent's processes, as a
/// host's overlay puts its own there; the system's proxy is what this
/// process's environment already holds.
fn proxy_environment(proxy: &AgentProxy) -> HashMap<String, Option<String>> {
    let address = match proxy {
        AgentProxy::System => return HashMap::new(),
        AgentProxy::Disabled => None,
        AgentProxy::Explicit(url) => Some(url.clone()),
    };
    let local = Some("localhost,127.0.0.1,::1".to_string());
    PROXY_VARIABLES
        .iter()
        .map(|name| (name.to_string(), address.clone()))
        .chain(
            NO_PROXY_VARIABLES
                .iter()
                .map(|name| (name.to_string(), local.clone())),
        )
        .collect()
}

/// The proxy of the configuration's `[network]` section. Each read also
/// puts it in the environment of the processes the Agent starts from then
/// on, as a host keeps its own overlay current: the Agent reads it at start
/// and at each settings change, before it restarts what the proxy reaches.
struct ConfiguredNetwork {
    config: AgentConfig,
    processes: ChildProcessEnvironment,
}

impl NetworkSettingsPort for ConfiguredNetwork {
    fn proxy(&self) -> Result<AgentProxy, String> {
        let proxy = self.config.standalone_settings()?.network.proxy;
        self.processes
            .replace(proxy_environment(&proxy))
            .map_err(|error| format!("invalid Agent child process environment: {error}"))?;
        Ok(proxy)
    }
}
