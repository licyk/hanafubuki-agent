//! Tab in the composer (Codex's `composer.queue`): what was typed waits for
//! the running turn instead of being sent, whatever it is.
//!
//! A message, a `!` shell command and a `/` command are queued alike; each
//! runs the way Enter would have run it once the queue reaches it, which is
//! the chat's to do (the composer only says what it is). A command that runs
//! at once is not queued — it would be in the transcript twice — and with no
//! turn running Tab sends as Enter does.

use std::time::Instant;

use super::{ChatComposer, InputResult, QueuedInput, QueuedMessage};
use crate::user_message::UserMessage;

impl ChatComposer {
    /// Tab (Codex's `composer.queue`): while a turn runs what was typed is
    /// queued for after it — a message, a `!` shell command and a `/` command
    /// alike, each running as Enter would have when the queue reaches it;
    /// with nothing running it is sent as Enter sends it. Within a paste it
    /// is the paste's.
    pub(super) fn handle_queue_key(&mut self, now: Instant) -> (InputResult, bool) {
        if self
            .paste_burst
            .newline_should_insert_instead_of_submit(now)
        {
            if !self.paste_burst.try_append_char_if_active('\t', now) {
                self.paste_burst.extend_window(now);
            }
            return (InputResult::None, true);
        }
        if !self.is_task_running {
            return self.handle_submission(now);
        }
        // A `/name` that runs at once is not queued: it would be in the
        // transcript twice.
        if let Some(result) = self.try_dispatch_slash_command() {
            if !matches!(result, InputResult::None) {
                return (result, true);
            }
        }
        let text = self.current_text_with_pending().trim().to_string();
        let Some(message) = (!text.is_empty()).then(|| {
            let message = UserMessage {
                text,
                images: std::mem::take(&mut self.images),
            };
            self.textarea.set_text_clearing_elements("");
            self.pending_pastes.clear();
            self.history.record_local_submission(message.clone());
            message
        }) else {
            return (InputResult::None, true);
        };
        let kind = QueuedInput::of(&message.text);
        (InputResult::Queued(QueuedMessage { message, kind }), true)
    }
}
