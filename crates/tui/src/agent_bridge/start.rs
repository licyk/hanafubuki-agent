//! Starting a turn: with a message the person sent, or with a command the
//! person typed after `!`, which runs with no model. Either answer comes
//! back as a reply: the turn accepted, or why it was not.

use hanafubuki_agent::{AgentSessionId, AgentTurnRequest};

use super::{AgentBridge, AgentReply};
use crate::app_event::AppEvent;

/// What a turn starts with.
pub(super) enum Start {
    Message(String),
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
                Start::Message(text) => {
                    let request = AgentTurnRequest {
                        text,
                        images: Vec::new(),
                        skills: Vec::new(),
                        host: Default::default(),
                    };
                    agent.submit_turn_with_sink(session_id, request, sink).await
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
