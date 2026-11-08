use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentSessionMode {
    Default,
    /// Read-only exploration that ends with a proposed plan. Only tools whose
    /// effect is read-only are available.
    Plan,
}

/// Who asked for a turn. The kernel only carries it to the observers
/// (`AgentNotification::TurnAdmitted`); it changes nothing in the turn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AgentTurnOrigin {
    /// A message the user wrote and submitted from the interface.
    User,
    /// Anything else: a turn started through the service API, or the fixed
    /// instruction a plan implementation sends.
    #[default]
    Programmatic,
}
