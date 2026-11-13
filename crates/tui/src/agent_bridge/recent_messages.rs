//! What the person sent in the Agent's sessions, read for the composer's
//! history search (Ctrl+R) on a blocking thread of the Agent's runtime. A
//! reply always comes, empty when the read failed or never ran, so a search
//! waiting for it ends.

use hanafubuki_agent::RECENT_USER_MESSAGES_MAX;

use super::AgentBridge;
use crate::app_event::{AppEvent, AppEventSender};

impl AgentBridge {
    pub(crate) fn read_recent_messages(&self) {
        let agent = self.agent.clone();
        let mut reply = MessagesReply {
            app_event_tx: self.app_event_tx.clone(),
            messages: Vec::new(),
        };
        // A runtime that has shut down drops the closure, and the reply
        // with it.
        drop(self.agent.runtime().handle().spawn_blocking(move || {
            if let Ok(messages) = agent.recent_user_messages(RECENT_USER_MESSAGES_MAX, None) {
                reply.read(messages);
            }
        }));
    }
}

/// Sent when dropped: after the read, or in place of one.
struct MessagesReply {
    app_event_tx: AppEventSender,
    messages: Vec<String>,
}

impl MessagesReply {
    fn read(&mut self, messages: Vec<String>) {
        self.messages = messages;
    }
}

impl Drop for MessagesReply {
    fn drop(&mut self) {
        self.app_event_tx
            .send(AppEvent::RecentMessagesRead(std::mem::take(
                &mut self.messages,
            )));
    }
}
