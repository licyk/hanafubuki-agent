// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! What the chat does with the messages the person sends (Codex's
//! `input_flow.rs` and `input_restore.rs`) through its queue
//! (`input_queue`): a message starts a turn, steers the running one or
//! waits; the bridge's answers move it along; a turn's end starts the next
//! one or puts the messages back in the composer; Esc and Shift+Left act
//! on what waits. A message is written into the conversation when it
//! reaches it: as it starts a turn, or once the turn read it as steering;
//! a steer's clipboard images are then removed (the session keeps its
//! copy), a turn's once the turn is accepted (`agent_bridge::start`). A
//! message with images the model is known not to read stays in the
//! composer (`images`).

use super::input_queue::{EscAction, Queued, Settled, Submission};
use super::ChatWidget;
use crate::agent_bridge::SteerOutcome;
use crate::app_event::{AppEvent, Op};
use crate::bottom_pane::QueuedInput;
use crate::history_cell;
use crate::user_message::UserMessage;

const REVIEW_HOLDS_MESSAGES: &str = "Messages sent during /review wait for it to end. \
     Press Ctrl-C now to cancel the review.";

impl ChatWidget {
    /// Enter: sends `message` to the session, created by it when there is
    /// none; with no Provider or model set up yet, the message goes back to
    /// the composer and the setup opens.
    pub(crate) fn submit_user_message(&mut self, message: UserMessage) {
        self.send_user_message(message.into(), false);
    }

    /// Tab while a turn runs: `message` waits for the turns before it.
    pub(super) fn queue_user_message(&mut self, message: impl Into<Queued>) {
        let queued = message.into();
        match queued.kind {
            QueuedInput::Command => self.queue_command(queued.message),
            QueuedInput::Shell | QueuedInput::Message => {
                self.send_user_message(queued, true);
            }
        }
    }

    /// A `/name` queued with Tab: its command runs when the queue reaches
    /// it, and what a command that takes no argument cannot do yet waits as
    /// a message for the turn after.
    fn queue_command(&mut self, message: UserMessage) {
        let running = self.bottom_pane.is_task_running();
        match self
            .input_queue
            .queue(Queued::with_kind(message, QueuedInput::Command), running)
        {
            Submission::Start(queued) => self.run_queued(queued),
            Submission::Steer { .. } | Submission::Waits => {}
        }
        self.sync_pending_input();
    }

    pub(super) fn send_user_message(&mut self, queued: Queued, queue: bool) -> bool {
        let kind = queued.kind;
        let message = queued.message;
        if message.text.trim().is_empty() {
            return false;
        }
        if self.setup_needed && !self.session_open {
            self.bottom_pane.set_composer_message(message);
            self.add_info_message(
                "Set up a model Provider and choose a model first; your message is kept."
                    .to_string(),
            );
            self.open_setup();
            return false;
        }
        // A message that runs now runs its `!` command; one that waits is
        // given the kind the composer saw, so its command runs untaken when
        // the queue reaches it.
        let message = if queue || self.user_shell {
            message
        } else {
            match self.submit_shell_command(message) {
                Some(message) => message,
                None => return true,
            }
        };
        if let Some(refusal) = self.images_refusal(&message) {
            self.add_error_message(refusal);
            self.bottom_pane.restore_draft(message);
            return false;
        }
        self.session_open = true;
        let running = self.bottom_pane.is_task_running();
        // A `!command` reads no messages: they wait for it.
        let submission = if queue || self.user_shell {
            self.input_queue
                .queue(Queued::with_kind(message, kind), running)
        } else {
            self.input_queue.submit(message, running)
        };
        match submission {
            Submission::Start(queued) => self.run_queued(queued),
            Submission::Steer { id, message } => self.send_steer(id, message),
            Submission::Waits => {}
        }
        self.sync_pending_input();
        true
    }

    /// Runs the item the queue hands over: a turn starts with a message, a
    /// `!` command runs, and a `/` command dispatches — each once nothing
    /// else runs, which is what leaving the queue means.
    fn run_queued(&mut self, queued: Queued) {
        match queued.kind {
            QueuedInput::Message => self.start_turn(queued.message),
            QueuedInput::Shell => {
                if let Some(message) = self.submit_shell_command(queued.message) {
                    self.start_turn(message);
                }
            }
            QueuedInput::Command => {
                let message = queued.message;
                match crate::commands::parse_input(&message.text) {
                    crate::commands::ParsedInput::Command(command, args) => {
                        self.dispatch_command(command, args)
                    }
                    crate::commands::ParsedInput::Unrecognized(name) => self.add_to_history(
                        history_cell::new_info_event(
                            format!(
                                r#"Unrecognized command '/{name}'. Type "/" for a list of supported commands."#
                            ),
                            None,
                        ),
                    ),
                    // A slash in the name is a path, and a message after all.
                    crate::commands::ParsedInput::Message => self.start_turn(message),
                }
            }
        }
    }

    fn start_turn(&mut self, message: UserMessage) {
        self.welcome.dismiss();
        self.add_to_history(history_cell::new_user_prompt(&message));
        self.app_event_tx.send(AppEvent::Op(Op::UserInput(message)));
    }

    fn send_steer(&self, id: u64, message: UserMessage) {
        self.app_event_tx
            .send(AppEvent::Op(Op::Steer { id, message }));
    }

    pub(super) fn on_steered(&mut self, id: u64, outcome: SteerOutcome) {
        if let Some(message) = self.input_queue.steered(id, outcome) {
            self.write_steer(&message);
        }
        self.advance_input_queue();
    }

    pub(super) fn on_steering_consumed(&mut self, through: u64) {
        for message in self.input_queue.consumed(through) {
            self.write_steer(&message);
        }
        self.sync_pending_input();
    }

    /// A steer the turn read: into the conversation, which keeps its
    /// images.
    fn write_steer(&mut self, message: &UserMessage) {
        message.remove_pasted_files();
        self.add_to_history(history_cell::new_steering_prompt(message));
    }

    /// A message sent to start a turn met a turn the chat had not heard of
    /// (a subagent's), and did not reach it.
    pub(super) fn on_undelivered(&mut self, message: UserMessage) {
        // A `!command` that met such a turn did not start either.
        self.user_shell = false;
        self.add_error_message(
            "The message did not reach the turn; it is back in the composer.".to_string(),
        );
        self.bottom_pane.restore_draft(message);
    }

    /// Sends the next steer, and once the ended turn has settled, starts
    /// the next turn or puts the messages back in the composer.
    pub(super) fn advance_input_queue(&mut self) {
        if let Some((id, message)) = self.input_queue.next_steer() {
            self.send_steer(id, message);
        }
        match self.input_queue.settle() {
            Some(Settled::Start(queued)) => self.run_queued(queued),
            Some(Settled::Restore(message)) => self.bottom_pane.restore_draft(message),
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

    /// Shift+Left: the last message no turn has been sent back in the
    /// composer, before the draft; `false` with none waiting.
    pub(super) fn edit_queued_message(&mut self) -> bool {
        let Some(message) = self.input_queue.pop_editable() else {
            return false;
        };
        self.bottom_pane.restore_draft(message);
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
        let mut preview = self.input_queue.preview();
        preview.questions = self.async_questions.summary();
        preview.question_deadline = self.async_questions.deadline();
        self.bottom_pane.set_pending_input(preview);
    }
}
