//! Starting a turn: with a message the person sent, or with a command the
//! person typed after `!`, which runs with no model. Either answer comes
//! back as a reply: the turn accepted, or why it was not. A message's
//! images go with it, read from their files by the Agent, which keeps its
//! own copy: once the turn is accepted, the clipboard images' copies are
//! removed.

use hanafubuki_agent::{AgentSessionId, AgentTurnRequest};

use super::{AgentBridge, AgentReply, TurnUpdate};
use crate::app_event::AppEvent;
use crate::user_message::UserMessage;

/// What a turn starts with.
pub(super) enum Start {
    Message(UserMessage),
    ShellCommand(String),
}

impl AgentBridge {
    pub(super) fn start_turn(&mut self, session_id: AgentSessionId, start: Start) {
        self.turn_running = true;
        let sink = self.sink();
        let app_event_tx = self.app_event_tx.clone();
        let agent = self.agent.clone();
        self.agent.runtime().handle().spawn(async move {
            let accepted = match start {
                Start::Message(message) => {
                    let request = AgentTurnRequest {
                        text: message.text.clone(),
                        images: message.request_images(),
                        skills: Vec::new(),
                        host: Default::default(),
                    };
                    let accepted = agent.submit_turn_with_sink(session_id, request, sink).await;
                    if accepted.is_ok() {
                        message.remove_pasted_files();
                    }
                    accepted
                }
                Start::ShellCommand(command) => {
                    agent
                        .submit_user_shell_command_with_sink(session_id, command, sink)
                        .await
                }
            };
            let reply = match accepted {
                Ok(turn) => AgentReply::TurnAccepted(turn),
                Err(error) => AgentReply::TurnRejected(error),
            };
            app_event_tx.send(AppEvent::AgentReply(reply));
        });
    }
}

/// A `!command` that met a running turn: back to the composer as typed.
pub(super) fn undelivered_command(command: &str) -> TurnUpdate {
    TurnUpdate::Undelivered(UserMessage::from_text(format!("!{command}")))
}
