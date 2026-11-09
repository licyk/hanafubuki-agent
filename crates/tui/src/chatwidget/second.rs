//! The chat of the second conversation (the side conversation, a subagent
//! shown): this chat's settings over a sender of its own, which runs only
//! the commands that make sense there.

use super::{ChatWidget, ChatWidgetInit};
use crate::app_event::AppEventSender;
use crate::commands::SECOND_CONVERSATION_COMMANDS;
use crate::tui::FrameRequester;

impl ChatWidget {
    /// A chat like this one, sending through `app_event_tx`.
    pub(crate) fn second(&self, app_event_tx: AppEventSender) -> ChatWidget {
        let mut chat = ChatWidget::new(ChatWidgetInit {
            frame_requester: FrameRequester::new(app_event_tx.clone()),
            app_event_tx,
            enhanced_keys_supported: self.bottom_pane.enhanced_keys_supported(),
            directory: self.directory.clone(),
            owned_transcript: self.owned_transcript,
            fullscreen_preference: self.preferences.fullscreen_transcript,
            model_override: self.model_override,
            appearance: self.appearance.clone(),
        });
        chat.preferences = self.preferences.clone();
        // The first chat looked for the hooks to review already.
        chat.hooks_checked = true;
        chat.bottom_pane
            .set_command_scope(Some(SECOND_CONVERSATION_COMMANDS));
        chat
    }
}
