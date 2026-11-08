use hanafubuki_agent_foundation::panic::catch_panic;

use super::AgentSlot;
use crate::AgentRunProjection;

/// A host's activity view reads the Agent's runs from its own threads, so
/// each read resolves the slot and is guarded: with no Agent running there
/// are no runs, and a panic in a read costs the Agent's rows, not the host.
impl AgentSlot {
    pub fn list_run_projections(&self) -> Vec<AgentRunProjection> {
        let Ok(agent) = self.agent() else {
            return Vec::new();
        };
        catch_panic(|| agent.list_run_projections()).unwrap_or_else(|message| {
            log::error!("listing the Agent's task activities panicked: {message}");
            Vec::new()
        })
    }

    pub fn get_run_projection(&self, activity_id: &str) -> Result<AgentRunProjection, String> {
        let agent = self.agent()?;
        catch_panic(|| agent.get_run_projection(activity_id)).unwrap_or_else(|message| {
            log::error!("reading an Agent task activity panicked: {message}");
            Err("Agent internal error while reading the activity".to_string())
        })
    }
}
