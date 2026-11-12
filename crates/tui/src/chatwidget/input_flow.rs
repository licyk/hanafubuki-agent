// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! What the chat does with the messages the person sends (Codex's
//! `input_flow.rs` and `input_restore.rs`) through its queue
//! (`input_queue`): a message starts a turn, steers the running one or
//! waits; the bridge's answers move it along; a turn's end starts the next
//! one or puts the messages back in the composer; Esc and Shift+Left act
//! on what waits. A message is written into the conversation when it
//! reaches it: as it starts a turn, or once the turn read it as steering.

use super::input_queue::{EscAction, Settled, Submission};
use super::ChatWidget;
use crate::agent_bridge::SteerOutcome;
use crate::app_event::{AppEvent, Op};
use crate::history_cell;

const REVIEW_HOLDS_MESSAGES: &str = "Messages sent during /review wait for it to end. \
     Press Ctrl-C now to cancel the review.";

impl ChatWidget {
    /// Enter: sends `text` to the session, created by it when there is
    /// none; with no Provider or model set up yet, the text goes back to
    /// the composer and the setup opens.
    pub(crate) fn submit_user_message(&mut self, text: String) {
        self.send_user_message(text, false);
    }

    /// Tab while a turn runs: `text` waits for the turns before it.
    pub(super) fn queue_user_message(&mut self, text: String) {
        self.send_user_message(text, true);
    }

    fn send_user_message(&mut self, text: String, queue: bool) {
        if text.trim().is_empty() {
            return;
        }
        if self.setup_needed && !self.session_open {
            self.bottom_pane.set_composer_text(text);
            self.add_info_message(
                "Set up a model Provider and choose a model first; your message is kept."
                    .to_string(),
            );
            self.open_setup();
            return;
        }
        self.session_open = true;
        let running = self.bottom_pane.is_task_running();
        let submission = if queue {
            self.input_queue.queue(text, running)
        } else {
            self.input_queue.submit(text, running)
        };
        match submission {
            Submission::Start(text) => self.start_turn(text),
            Submission::Steer { id, text } => self.send_steer(id, text),
            Submission::Waits => {}
        }
        self.sync_pending_input();
    }

    fn start_turn(&mut self, text: String) {
        self.add_to_history(history_cell::new_user_prompt(text.clone()));
        self.app_event_tx.send(AppEvent::Op(Op::UserInput(text)));
    }

    fn send_steer(&self, id: u64, text: String) {
        self.app_event_tx.send(AppEvent::Op(Op::Steer { id, text }));
    }

    pub(super) fn on_steered(&mut self, id: u64, outcome: SteerOutcome) {
        if let Some(text) = self.input_queue.steered(id, outcome) {
            self.add_to_history(history_cell::new_user_prompt(text));
        }
        self.advance_input_queue();
    }

    pub(super) fn on_steering_consumed(&mut self, through: u64) {
        for text in self.input_queue.consumed(through) {
            self.add_to_history(history_cell::new_user_prompt(text));
        }
        self.sync_pending_input();
    }

    /// A message sent to start a turn met a turn the chat had not heard of
    /// (a subagent's), and did not reach it.
    pub(super) fn on_undelivered(&mut self, text: String) {
        self.add_error_message(
            "The message did not reach the turn; it is back in the composer.".to_string(),
        );
        self.bottom_pane.restore_draft(text);
    }

    /// Sends the next steer, and once the ended turn has settled, starts
    /// the next turn or puts the messages back in the composer.
    pub(super) fn advance_input_queue(&mut self) {
        if let Some((id, text)) = self.input_queue.next_steer() {
            self.send_steer(id, text);
        }
        match self.input_queue.settle() {
            Some(Settled::Start(text)) => self.start_turn(text),
            Some(Settled::Restore(text)) => self.bottom_pane.restore_draft(text),
            None => {}
        }
        self.sync_pending_input();
    }

    /// Esc while a turn runs: `true` when it only said why the review is
    /// not cancelled; otherwise the bottom pane interrupts.
    pub(super) fn esc_handled(&mut self) -> bool {
        if self.input_queue.esc() == EscAction::WarnReview {
            self.add_to_history(history_cell::new_warning_event(
                REVIEW_HOLDS_MESSAGES.to_string(),
            ));
            return true;
        }
        false
    }

    /// Shift+Left: the last queued message back in the composer, before
    /// the draft; `false` with none queued.
    pub(super) fn edit_queued_message(&mut self) -> bool {
        let Some(text) = self.input_queue.pop_queued() else {
            return false;
        };
        self.bottom_pane.restore_draft(text);
        self.sync_pending_input();
        true
    }

    /// Whether a message waits or is about to start a turn.
    pub(crate) fn has_pending_input(&self) -> bool {
        self.input_queue.is_busy()
    }

    #[cfg(test)]
    pub(crate) fn composer_text(&self) -> String {
        self.bottom_pane.composer_text()
    }

    pub(super) fn sync_pending_input(&mut self) {
        self.bottom_pane
            .set_pending_input(self.input_queue.preview());
    }
}
