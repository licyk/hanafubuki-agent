//! The `execution` plugin's options (`[pluginOptions.execution]`): command
//! timeout, environment selection and shell startup. The facade owns the
//! settings schema and uses the timeout bounds and default declared here.

use hanafubuki_agent_execution::ShellEnvironment;
use hanafubuki_agent_foundation::environment_policy::{EnvironmentInherit, EnvironmentPolicy};
use serde::Deserialize;

/// The range of `commandTimeoutMinutes`: a minute to a day.
pub const COMMAND_TIMEOUT_MINUTES_LIMITS: (u64, u64) = (1, 24 * 60);
/// Ten minutes, the limit before it was an option.
pub const DEFAULT_COMMAND_TIMEOUT_MINUTES: u64 = 10;

/// The configuration the row derives from the settings, which checked each
/// value against the schema.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct ExecutionOptions {
    command_timeout_minutes: u64,
    environment_inherit: EnvironmentInherit,
    environment_ignore_default_excludes: bool,
    environment_exclude: Vec<String>,
    environment_include_only: Vec<String>,
    environment_set: Vec<String>,
    shell_use_profile: bool,
    shell_snapshot: bool,
}

impl Default for ExecutionOptions {
    fn default() -> Self {
        Self {
            command_timeout_minutes: DEFAULT_COMMAND_TIMEOUT_MINUTES,
            environment_inherit: EnvironmentInherit::All,
            environment_ignore_default_excludes: true,
            environment_exclude: Vec::new(),
            environment_include_only: Vec::new(),
            environment_set: Vec::new(),
            shell_use_profile: false,
            shell_snapshot: true,
        }
    }
}

impl ExecutionOptions {
    pub(crate) fn shell_environment(&self) -> Result<ShellEnvironment, String> {
        let mut set = std::collections::BTreeMap::new();
        for entry in &self.environment_set {
            // A leading '=' belongs to a Windows hidden variable's name.
            let separator = entry
                .char_indices()
                .find(|(index, ch)| *index > 0 && *ch == '=')
                .map(|(index, _)| index)
                .ok_or("environmentSet entries must be NAME=VALUE")?;
            let (name, value) = (&entry[..separator], &entry[separator + 1..]);
            hanafubuki_agent_foundation::environment_policy::insert(
                &mut set,
                name.into(),
                value.into(),
            );
        }
        let policy = EnvironmentPolicy {
            inherit: self.environment_inherit,
            ignore_default_excludes: self.environment_ignore_default_excludes,
            exclude: self.environment_exclude.clone(),
            include_only: self.environment_include_only.clone(),
            set,
        };
        Ok(ShellEnvironment {
            policy,
            use_profile: self.shell_use_profile,
            snapshot: self.shell_snapshot,
            context: None,
        })
    }

    /// The command timeout as a command's request carries it.
    pub(crate) fn command_timeout_ms(&self) -> u64 {
        self.command_timeout_minutes.saturating_mul(60_000)
    }
}

#[cfg(test)]
mod tests;
