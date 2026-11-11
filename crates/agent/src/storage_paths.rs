use std::path::PathBuf;

/// The Agent's durable storage, under the data directory the host names. Its
/// configuration is not here: it lives in the host's configuration directory
/// (`hanafubuki_agent_config::AgentConfig`).
///
/// The host chooses the root. The Agent application then passes
/// only the relevant leaf path to each concrete repository, so no repository
/// or service independently guesses where another Agent store lives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentStoragePaths {
    pub root: PathBuf,
    pub history_root: PathBuf,
    pub memory_root: PathBuf,
    pub skills_root: PathBuf,
    pub skill_state_file: PathBuf,
    /// The crash-loop start-up marker (`AgentStartMarker`).
    pub start_marker_file: PathBuf,
    /// The fatal panic the last run ended in, which the process's panic
    /// hook writes (the host's, embedded; its foundation's copy of
    /// `configure_crash_record` writes the same record) and the start
    /// marker reads.
    pub crash_record_file: PathBuf,
    /// The process groups of the Agent's own processes
    /// (`hanafubuki_agent_process::process_groups::ProcessGroupRecord`).
    pub process_groups_file: PathBuf,
    /// The standalone Agent's log files (`logging`); a host logs to its own.
    pub logs_dir: PathBuf,
    /// The Git worktrees the Agent makes (`hanafubuki-agent-worktree`).
    pub worktrees_root: PathBuf,
    /// The hooks' trust and enablement (`hanafubuki-agent-hooks`).
    pub hooks_state_file: PathBuf,
    /// The installed plugins: their cache, their data, the git-added
    /// marketplaces and the install record (`hanafubuki-agent-packages`).
    pub plugins_root: PathBuf,
    /// What the Agent knows about each model it reaches
    /// (`hanafubuki_agent_kernel::ModelMetadataSnapshot`).
    pub model_metadata_file: PathBuf,
}

impl AgentStoragePaths {
    pub fn under(root: PathBuf) -> Self {
        Self {
            history_root: root.clone(),
            memory_root: root.join("memory"),
            skills_root: root.join("skills"),
            skill_state_file: root.join("skills").join("state.json"),
            start_marker_file: root.join("start-marker.json"),
            crash_record_file: root.join("crash-record.json"),
            process_groups_file: root.join("process-groups.json"),
            logs_dir: root.join("logs"),
            worktrees_root: root.join("worktrees"),
            hooks_state_file: root.join("hooks-state.json"),
            plugins_root: root.join("plugins"),
            model_metadata_file: root.join("model-metadata.json"),
            root,
        }
    }
}

#[cfg(test)]
mod tests;
