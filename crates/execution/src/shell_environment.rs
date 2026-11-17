use hanafubuki_agent_foundation::environment_policy::EnvironmentPolicy;
use serde::{Deserialize, Serialize};

/// Shell startup and environment selected by the host, never model arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ShellEnvironment {
    pub policy: EnvironmentPolicy,
    /// PowerShell profile default when the command leaves `login` unset.
    pub use_profile: bool,
    pub snapshot: bool,
    pub context: Option<ExecutionEnvironmentContext>,
}

impl Default for ShellEnvironment {
    fn default() -> Self {
        Self {
            policy: EnvironmentPolicy::default(),
            use_profile: false,
            snapshot: true,
            context: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionEnvironmentContext {
    pub session_id: u64,
    pub turn_id: u64,
    pub tool_call_id: String,
    pub permission_mode: Option<String>,
}
