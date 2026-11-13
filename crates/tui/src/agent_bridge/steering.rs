//! Steering the running turn for the chat's input queue (Codex's pending
//! steers): a message the chat sends with an id gets one answer, the
//! sequence the turn took it under or that it did not reach the turn; the
//! turn's `SteeringConsumed` events then say which sequences it read. The
//! chat keeps each message until then, since the Agent drops the steering
//! a cancelled or failed turn had not read; it removes a steer's clipboard
//! images only once the turn read it.

use hanafubuki_agent::AgentSteeringRequest;

use super::{AgentBridge, AgentReply, TurnUpdate};
use crate::user_message::UserMessage;

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
    /// Steers the running turn with `message`. With an `id` the chat hears
    /// back either way; without one (a message that was to start a turn,
    /// sent before the chat heard of a subagent's turn) only a failure is
    /// said, with the message, which goes back to the composer.
    pub(super) fn steer(&self, id: Option<u64>, message: UserMessage) -> Option<TurnUpdate> {
        let not_delivered = |message| match id {
            Some(id) => TurnUpdate::Steered {
                id,
                outcome: SteerOutcome::NotDelivered,
            },
            None => TurnUpdate::Undelivered(message),
        };
        let Some(turn) = self.turn.filter(|_| self.turn_running) else {
            return Some(not_delivered(message));
        };
        let unsent = id.is_none().then(|| message.clone());
        let sink = self.sink();
        self.spawn(move |agent| async move {
            let request = AgentSteeringRequest {
                images: message.request_images(),
                text: message.text,
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
                // Written into the conversation as it was sent.
                (None, Ok(_)) => {
                    if let Some(message) = &unsent {
                        message.remove_pasted_files();
                    }
                    return Ok(None);
                }
                (None, Err(_)) => TurnUpdate::Undelivered(unsent.unwrap_or_default()),
            };
            Ok(Some(AgentReply::Update(Box::new(update))))
        });
        None
    }
}
