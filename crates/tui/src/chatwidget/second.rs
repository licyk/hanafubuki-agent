//! The chat of the second conversation (the side conversation, a subagent
//! shown): this chat's settings over a sender of its own, which runs only
//! the commands that make sense there.

use super::{ChatWidget, ChatWidgetInit};
use crate::app_event::AppEventSender;
use crate::commands::SECOND_CONVERSATION_COMMANDS;
use crate::tui::FrameRequester;

impl ChatWidget {
    pub(crate) fn has_unsent_input(&self) -> bool {
        !self.composer_is_empty()
            || self.bottom_pane.is_in_paste_burst()
            || self.has_pending_input()
    }

    /// Paint the retained composer and live work, returning the transcript's rows.
    pub(crate) fn paint_parked(
        &self,
        area: ratatui::layout::Rect,
        buf: &mut ratatui::buffer::Buffer,
        watch_only: bool,
    ) -> ratatui::layout::Rect {
        use crate::render::renderable::Renderable;
        use ratatui::layout::Rect;

        let mut transcript = self
            .bottom_pane
            .composer
            .paint_parked(area, buf, watch_only);
        if let Some(status) = self.bottom_pane.status_widget() {
            let height = status.desired_height(area.width).min(transcript.height);
            transcript.height -= height;
            status.render(
                Rect {
                    y: transcript.bottom(),
                    height,
                    ..area
                },
                buf,
            );
        }
        transcript
    }

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
