//! Where the standalone Agent keeps its configuration and its data.

use std::path::PathBuf;

/// Names one directory that holds both the configuration and the data, in
/// place of the operating system's directories.
pub const AGENT_HOME_ENV: &str = "HANAFUBUKI_AGENT_HOME";

const APPLICATION_DIRECTORY: &str = "hanafubuki-agent";

/// The standalone Agent's two directories. A host names its own instead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentDirs {
    /// `config.toml`, `providers.toml`, `mcp.toml`.
    pub config_dir: PathBuf,
    /// History, memory, Skills, logs and the lock (`AgentStoragePaths`).
    pub data_dir: PathBuf,
}

impl AgentDirs {
    /// The directories of this process: [`AGENT_HOME_ENV`] when it is set,
    /// the operating system's otherwise.
    pub fn standalone() -> Result<Self, String> {
        Self::resolve(
            std::env::var_os(AGENT_HOME_ENV)
                .filter(|home| !home.is_empty())
                .map(PathBuf::from),
        )
    }

    /// `home` holds both, as Codex's `CODEX_HOME` does (a relative one is
    /// taken from the working directory). Without it, `hanafubuki-agent` in
    /// the user's configuration and data directories: on Linux
    /// `~/.config/hanafubuki-agent` and `~/.local/share/hanafubuki-agent`, on
    /// macOS and Windows one directory, since their two are the same.
    pub fn resolve(home: Option<PathBuf>) -> Result<Self, String> {
        if let Some(home) = home {
            let home = std::path::absolute(&home).map_err(|error| {
                format!("{AGENT_HOME_ENV} {} is unusable: {error}", home.display())
            })?;
            return Ok(Self {
                config_dir: home.clone(),
                data_dir: home,
            });
        }
        let missing = |kind: &str| {
            format!("the operating system names no {kind} directory; set {AGENT_HOME_ENV}")
        };
        Ok(Self {
            config_dir: dirs::config_dir()
                .ok_or_else(|| missing("configuration"))?
                .join(APPLICATION_DIRECTORY),
            data_dir: dirs::data_dir()
                .ok_or_else(|| missing("data"))?
                .join(APPLICATION_DIRECTORY),
        })
    }
}

#[cfg(test)]
mod tests;
