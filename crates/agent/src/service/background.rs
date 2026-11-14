//! A session's background work, for its host: what it holds open, the
//! events it has not read, whether it is woken for them, a root's tree in
//! full, and the woken turn a host submits when it is told
//! `AgentBackgroundEvent::Woken`.

use std::sync::Arc;

use hanafubuki_agent_contracts::TurnExtensions;
use hanafubuki_agent_foundation::blocking::run_blocking_operation;
use hanafubuki_agent_kernel::{
    AgentError, AgentEventSink, AgentSessionId, AgentWakePolicy, AgentWakeTurnAccepted,
};

use super::AgentApplicationService;
use crate::wake::{AgentBackgroundStatus, AgentBackgroundTasks};

impl AgentApplicationService {
    /// Starts the turn that delivers `session_id`'s queued wake events (what
    /// its sub-agents reported): a `Programmatic` turn whose only opener is
    /// the events, in the session's permission mode, with no user message.
    /// Returns it with the events it delivers. `Ok(None)` when nothing is
    /// queued any more, when a turn runs (it delivers them at its next
    /// step), or when the session is not woken (`wakeOnBackgroundTasks` is
    /// off, or it is a side conversation or a review). It runs while the
    /// session's wakes are paused too: the host asked for it. Each one
    /// counts toward the runaway bound.
    pub async fn submit_wake_turn_with_sink(
        &self,
        session_id: AgentSessionId,
        event_sink: Arc<dyn AgentEventSink>,
    ) -> Result<Option<AgentWakeTurnAccepted>, String> {
        let session = self.activate_provider_session(session_id).await?;
        if session.wake_policy == AgentWakePolicy::Off {
            return Ok(None);
        }
        let permission_mode = self.driver.turn_permission_mode(session_id, None).await?;
        let world_state = self
            .driver
            .collect_world_state(
                session_id,
                session.mode,
                permission_mode,
                None,
                TurnExtensions::default(),
            )
            .await
            .map_err(|error| error.to_string())?;
        let turn = self
            .driver
            .collect_turn_extensions(session_id, session.mode)
            .await
            .map_err(|error| error.to_string())?;
        let core = self.core.clone();
        let accepted = hanafubuki_agent_foundation::blocking::run_blocking_operation(
            self.blocking.as_ref(),
            "agent wake turn admission worker",
            move || {
                core.submit_wake_turn_with_sink(
                    session_id,
                    hanafubuki_agent_kernel::AgentTurnInput {
                        text: String::new(),
                        images: Vec::new(),
                        skills: Vec::new(),
                        permission_mode,
                        turn,
                        world_state,
                        origin: hanafubuki_agent_kernel::AgentTurnOrigin::Programmatic,
                        provisional_title: None,
                    },
                    event_sink,
                )
                .map_err(|error| error.to_string())
            },
        )
        .await??;
        if accepted.is_some() {
            self.wakes.pauses.woken(session_id);
            self.wakes.feed.hint(session_id);
        }
        Ok(accepted)
    }

    /// What `session_id`'s background work holds open, the events it has
    /// not read, and whether it is woken for them. Its agents are those
    /// this instance runs for it, read before its queue: an agent counts
    /// until its end is queued, so one that ended is counted or queued,
    /// never neither. Cheap; it may load the session.
    pub fn background_status(
        &self,
        session_id: AgentSessionId,
    ) -> Result<AgentBackgroundStatus, AgentError> {
        self.wakes.status(&self.driver, session_id)
    }

    /// The background work of `root`'s tree in full: its sub-agents (each
    /// as `subagents` lists it, with its last answer's start), its
    /// background terminals with their recent output, the events queued on it
    /// and whether it is woken for them. The counts agree with
    /// [`Self::background_status`]: an agent listed as running is one it
    /// counts. Read it again when told `AgentBackgroundEvent::Changed`. It
    /// lists the tree's sessions and reads their last answers off the
    /// async runtime.
    pub async fn background_tasks(
        &self,
        root: AgentSessionId,
    ) -> Result<AgentBackgroundTasks, AgentError> {
        let (driver, wakes) = (self.driver.clone(), self.wakes.clone());
        run_blocking_operation(
            self.blocking.as_ref(),
            "agent background tasks worker",
            move || wakes.tasks(&driver, root, true),
        )
        .await
        .map_err(AgentError::Internal)?
    }
}
