//! Steering the running turn for the chat's input queue (Codex's pending
//! steers): a message the chat sends with an id gets one answer, the
//! sequence the turn took it under or that it did not reach the turn; the
//! turn's `SteeringConsumed` events then say which sequences it read. The
//! chat keeps each message until then, since the Agent drops the steering
//! a cancelled or failed turn had not read.

use hanafubuki_agent::AgentSteeringRequest;

use super::{AgentBridge, AgentReply, TurnUpdate};

/// What became of a steering message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SteerOutcome {
    /// The turn took it under this sequence, to read at its next step.
    Accepted(u64),
    /// It did not reach the turn: no turn ran any more, or the Agent
    /// refused it.
    NotDelivered,
}

impl AgentBridge {
    /// Steers the running turn with `text`. With an `id` the chat hears
    /// back either way; without one (a message that was to start a turn,
    /// sent before the chat heard of a subagent's turn) only a failure is
    /// said, with the text, which goes back to the composer.
    pub(super) fn steer(&self, id: Option<u64>, text: String) -> Option<TurnUpdate> {
        let not_delivered = |text| match id {
            Some(id) => TurnUpdate::Steered {
                id,
                outcome: SteerOutcome::NotDelivered,
            },
            None => TurnUpdate::Undelivered(text),
        };
        let Some(turn) = self.turn.filter(|_| self.turn_running) else {
            return Some(not_delivered(text));
        };
        let unsent = id.is_none().then(|| text.clone());
        let sink = self.sink();
        self.spawn(move |agent| async move {
            let request = AgentSteeringRequest {
                text,
                images: Vec::new(),
                host: Default::default(),
            };
            let accepted = agent
                .steer_turn_with_sink(
                    turn.session_id,
                    turn.turn_id,
                    turn.generation,
                    request,
                    sink,
                )
                .await;
            let update = match (id, accepted) {
                (Some(id), accepted) => TurnUpdate::Steered {
                    id,
                    outcome: accepted.map_or(SteerOutcome::NotDelivered, |accepted| {
                        SteerOutcome::Accepted(accepted.sequence.value())
                    }),
                },
                (None, Ok(_)) => return Ok(None),
                (None, Err(_)) => TurnUpdate::Undelivered(unsent.unwrap_or_default()),
            };
            Ok(Some(AgentReply::Update(Box::new(update))))
        });
        None
    }
}
