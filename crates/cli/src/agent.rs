//! The standalone Agent a command runs on: its runtime composed from the
//! global flags, the process's panic hook and log file, and, for the
//! commands that need it, the running Agent.

use std::future::Future;
use std::time::Duration;

use hanafubuki_agent::logging;
use hanafubuki_agent::{
    panic, AgentDirs, AgentHandle, AgentRuntime, AgentRuntimeHost, AgentRuntimeOptions,
    AgentStartOptions, ConfigOverrides, AGENT_SHUTDOWN_BUDGET,
};

use crate::cli::GlobalArgs;
use crate::exit::Failure;

/// How long a command waits for the Agent to start: recovery reads every
/// session's index row, and the plugins start in turn.
const START_TIMEOUT: Duration = Duration::from_secs(120);

pub(crate) struct Standalone {
    pub(crate) runtime: AgentRuntime,
    pub(crate) dirs: AgentDirs,
}

impl Standalone {
    /// Composes the Agent from the global flags with `host`'s sinks, then
    /// installs the panic hook and the log file; nothing starts yet.
    pub(crate) fn open(global: &GlobalArgs, host: AgentRuntimeHost) -> Result<Self, Failure> {
        let dirs = dirs(global)?;
        let cwd = match &global.cd {
            Some(directory) => directory.clone(),
            None => std::env::current_dir()
                .map_err(|error| Failure::new(format!("the working directory: {error}")))?,
        };
        let runtime = AgentRuntime::standalone(
            AgentRuntimeOptions {
                dirs: dirs.clone(),
                bundled_skills: None,
                cwd,
                add_dirs: global.add_dirs.clone(),
                profile: global.profile.clone(),
                overrides: overrides(global)?,
                start: AgentStartOptions::default(),
            },
            host,
        )
        .map_err(Failure::new)?;
        panic::configure_crash_record(runtime.storage_paths().crash_record_file.clone());
        panic::install_panic_hook(logging::logging_blocked);
        let level = runtime
            .config()
            .standalone_settings()
            .map_err(Failure::new)?
            .log_level;
        if let Err(error) =
            logging::install_file_logger(&runtime.storage_paths().logs_dir, level.into())
        {
            eprintln!("warning: the log file is not written: {error}");
        }
        Ok(Self { runtime, dirs })
    }

    /// Starts the Agent and waits until it runs.
    pub(crate) fn start(&self) -> Result<AgentHandle, Failure> {
        self.runtime.start();
        self.runtime
            .wait_until_started(START_TIMEOUT)
            .map_err(|unavailable| Failure::new(unavailable.to_string()))
    }

    pub(crate) fn shutdown(self) {
        for diagnostic in self.runtime.shutdown(AGENT_SHUTDOWN_BUDGET) {
            eprintln!(
                "warning: {} did not stop: {}",
                diagnostic.worker, diagnostic.message
            );
        }
    }
}

/// Runs `command` on the started Agent, then shuts it down whatever the
/// command's result.
pub(crate) fn with_agent<T>(
    global: &GlobalArgs,
    command: impl FnOnce(&Standalone, &AgentHandle) -> Result<T, Failure>,
) -> Result<T, Failure> {
    let standalone = Standalone::open(global, AgentRuntimeHost::default())?;
    let result = standalone
        .start()
        .and_then(|agent| command(&standalone, &agent));
    standalone.shutdown();
    result
}

/// Runs what `operation` makes of the Agent as a task of its runtime, where
/// every call into it runs, and waits for it.
pub(crate) fn block_on<T, E, F>(
    agent: &AgentHandle,
    operation: impl FnOnce(AgentHandle) -> F,
) -> Result<T, Failure>
where
    T: Send + 'static,
    E: ToString + Send + 'static,
    F: Future<Output = Result<T, E>> + Send + 'static,
{
    agent
        .runtime()
        .block_on(operation(agent.clone()))
        .map_err(|failure| Failure::new(failure.to_string()))?
        .map_err(|error| Failure::new(error.to_string()))
}

fn dirs(global: &GlobalArgs) -> Result<AgentDirs, Failure> {
    let mut dirs = AgentDirs::standalone().map_err(Failure::new)?;
    if let Some(data_dir) = &global.data_dir {
        dirs.data_dir = std::path::absolute(data_dir)
            .map_err(|error| Failure::new(format!("--data-dir {}: {error}", data_dir.display())))?;
    }
    Ok(dirs)
}

/// The `-c` overrides, then the flags that are keys of config.toml, so a
/// flag wins over `-c`.
fn overrides(global: &GlobalArgs) -> Result<ConfigOverrides, Failure> {
    let mut raw = global.overrides.clone();
    if let Some(mode) = global.permission_mode {
        raw.push(format!("permissionMode={}", mode.key_value()));
    }
    if let Some(level) = global.log_level {
        let level = format!("{level:?}").to_lowercase();
        raw.push(format!("logLevel={level}"));
    }
    ConfigOverrides::parse(&raw).map_err(Failure::usage)
}
