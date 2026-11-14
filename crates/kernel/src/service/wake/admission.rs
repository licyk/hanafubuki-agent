//! The wake turn: a turn whose only opener is the session's queued wake
//! events.

use super::*;

impl AgentService {
    /// Admits a turn that delivers the session's queued wake events, and
    /// schedules it as `submit_turn_with_sink` does. `input` gives its
    /// permission mode, turn extensions, world state and origin; its text,
    /// images and Skills are not read. `Ok(None)` when nothing is queued or
    /// a turn runs (that turn delivers them); refused while the session's
    /// policy is off.
    pub fn submit_wake_turn_with_sink(
        &self,
        session_id: AgentSessionId,
        input: crate::AgentTurnInput,
        event_sink: Arc<dyn AgentEventSink>,
    ) -> Result<Option<AgentTurnAccepted>, AgentError> {
        if !self.inner.accepting.load(Ordering::Acquire) {
            return Err(AgentError::ShuttingDown);
        }
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
            AgentError::InvalidInput("Agent turn submission requires an async runtime".to_string())
        })?;
        let admitted = match self.begin_turn_with_admission(
            session_id,
            input,
            TurnAdmission::Wake,
            event_sink.as_ref(),
        ) {
            Ok(admitted) => admitted,
            Err(AgentError::NoWakeEvents | AgentError::TurnAlreadyRunning) => return Ok(None),
            Err(error) => return Err(error),
        };
        self.spawn_managed_turn(admitted, event_sink, runtime)
            .map(Some)
    }
}
