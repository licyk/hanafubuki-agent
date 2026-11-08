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
            root,
        }
    }
}

#[cfg(test)]
mod tests;
