use super::AgentApplicationService;
use hanafubuki_agent_foundation::background_worker::WorkerShutdownDiagnostic;
use hanafubuki_agent_kernel::{
    AgentError, AgentGeneration, AgentSessionContext, AgentSessionId, AgentSessionRuntimeSnapshot,
    AgentTurnId, AgentTurnPhase,
};
use std::sync::atomic::Ordering;
use std::time::Instant;

/// One turn as a host's activity view shows it: each session's current or
/// last turn, under an id that names the session, turn and generation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentRunProjection {
    pub activity_id: String,
    /// The turn's number within its session.
    pub turn: u64,
    pub phase: AgentTurnPhase,
    pub created_at: u128,
    pub updated_at: u128,
    pub error: Option<String>,
}

impl AgentApplicationService {
    /// Every session the Agent holds, as it is now, each read once its own
    /// staged commit settled; a session only in the history store is not
    /// among them. Off the async runtime only.
    pub fn session_snapshots(&self) -> Vec<AgentSessionRuntimeSnapshot> {
        self.core.list_snapshots()
    }

    /// What `session_id`'s context holds, as `/context` shows it: the last
    /// request a sampling step of this run sent, group by group, with what
    /// the Provider reported of it; before one, an estimate of the next
    /// (no tool runs and no model is called); the window and the usage,
    /// whose cache figures give the session's hit rate. During a turn it
    /// reads the request the turn sent last.
    pub async fn session_context(
        &self,
        session_id: AgentSessionId,
    ) -> Result<AgentSessionContext, AgentError> {
        // The estimate plans the next request's tools from the plugins.
        self.host.ensure_started().await;
        self.activate_session(session_id)?;
        self.core.session_context(session_id).await
    }

    pub fn list_run_projections(&self) -> Vec<AgentRunProjection> {
        self.core
            .list_snapshots()
            .into_iter()
            .filter_map(|session| {
                session.turn.map(|turn| AgentRunProjection {
                    activity_id: agent_activity_id(session.id, turn.id, turn.generation),
                    turn: turn.id.value(),
                    phase: turn.phase,
                    created_at: turn.created_at,
                    updated_at: turn.updated_at,
                    error: turn.last_error,
                })
            })
            .collect()
    }

    pub fn get_run_projection(&self, activity_id: &str) -> Result<AgentRunProjection, String> {
        self.list_run_projections()
            .into_iter()
            .find(|projection| projection.activity_id == activity_id)
            .ok_or_else(|| format!("Agent activity not found: {activity_id}"))
    }

    pub fn shutdown(&self, deadline: Instant) -> Vec<WorkerShutdownDiagnostic> {
        let mut diagnostics = self.stop_accepting_and_cancel_turns(deadline);
        diagnostics.extend(self.shutdown_workers_after_mcp(deadline));
        diagnostics
    }

    pub fn stop_accepting_and_cancel_turns(
        &self,
        deadline: Instant,
    ) -> Vec<WorkerShutdownDiagnostic> {
        self.accepting.store(false, Ordering::Release);
        let lost = self.capture_lost_background_work();
        let mut diagnostics = self
            .core
            .stop_operations(deadline)
            .into_iter()
            .map(|diagnostic| WorkerShutdownDiagnostic {
                worker: "agent-core".to_string(),
                message: diagnostic.message,
            })
            .collect::<Vec<_>>();
        // Capture while turns are live, then report after cancellation:
        // an active parent must not consume these during shutdown.
        diagnostics.extend(self.report_lost_background_work(lost));
        diagnostics
    }

    /// Connects the MCP servers in the background, so the Agent runs without
    /// waiting for them (`service/mcp/startup.rs`).
    pub(crate) fn spawn_mcp_start(&self) {
        self.mcp_application.spawn_start(&self.kernel_faults);
    }

    /// Stops the MCP side by `deadline`: cancels the connects under way
    /// (each disposes the process it spawned), then closes every connection
    /// and confirms its process gone. What is not confirmed by then is
    /// reported and keeps its cleanup obligation.
    pub async fn shutdown_mcp(&self, deadline: Instant) -> Result<(), String> {
        self.mcp_application.stop_connecting(deadline).await;
        self.mcp
            .shutdown(deadline)
            .await
            .map_err(|error| error.to_string())
    }

    pub fn shutdown_workers_after_mcp(&self, deadline: Instant) -> Vec<WorkerShutdownDiagnostic> {
        let mut diagnostics = self.memory_workers.shutdown(deadline);
        diagnostics.extend(
            self.core
                .shutdown_history(deadline)
                .into_iter()
                .map(|diagnostic| WorkerShutdownDiagnostic {
                    worker: "agent-history".to_string(),
                    message: diagnostic.message,
                }),
        );
        diagnostics.extend(self.workers.shutdown(deadline));
        diagnostics.extend(self.executions.shutdown(deadline));
        diagnostics
    }
}

pub fn agent_activity_id(
    session_id: AgentSessionId,
    turn_id: AgentTurnId,
    generation: AgentGeneration,
) -> String {
    format!(
        "agent:{}:{}:{}",
        session_id.value(),
        turn_id.value(),
        generation.value()
    )
}
