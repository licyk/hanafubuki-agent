//! The `execution` plugin's options (`[pluginOptions.execution]`): how long
//! a command may run. The schema is the Agent's (its descriptor), built
//! from these bounds and this default, so both say the same.

use serde::Deserialize;

/// The range of `commandTimeoutMinutes`: a minute to a day.
pub const COMMAND_TIMEOUT_MINUTES_LIMITS: (u64, u64) = (1, 24 * 60);
/// Ten minutes, the limit before it was an option.
pub const DEFAULT_COMMAND_TIMEOUT_MINUTES: u64 = 10;

/// The configuration the row derives from the settings, which checked each
/// value against the schema.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct ExecutionOptions {
    command_timeout_minutes: u64,
}

impl Default for ExecutionOptions {
    fn default() -> Self {
        Self {
            command_timeout_minutes: DEFAULT_COMMAND_TIMEOUT_MINUTES,
        }
    }
}

impl ExecutionOptions {
    /// The command timeout as a command's request carries it.
    pub(crate) fn command_timeout_ms(&self) -> u64 {
        self.command_timeout_minutes.saturating_mul(60_000)
    }
}
