// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A message that starts with `!` (Codex's `submit_shell_command`): the
//! rest is a command the Agent runs as a turn of its own, with no model, in
//! the session's sandbox; the model reads what it did with the next
//! message. `!` alone shows how. One runs only when nothing else does
//! (Codex's running beside a turn is not ported): during a turn it is
//! refused and stays in the composer. While it runs, a message waits for
//! it, as Tab queues one.

use super::ChatWidget;
use crate::app_event::{AppEvent, Op};
use crate::history_cell;

const USER_SHELL_COMMAND_HELP_TITLE: &str = "Prefix a command with ! to run it locally";
const USER_SHELL_COMMAND_HELP_HINT: &str = "Example: !ls";
const TURN_RUNNING: &str = "A turn is running; wait for it or interrupt it first.";

impl ChatWidget {
    /// Sends `text` as a command when it starts with `!`; `false` when it
    /// does not, and it is a message.
    pub(super) fn submit_shell_command(&mut self, text: &str) -> bool {
        let Some(command) = text.strip_prefix('!') else {
            return false;
        };
        if self.user_shell || self.bottom_pane.is_task_running() || self.input_queue.is_busy() {
            self.add_error_message(TURN_RUNNING.to_string());
            self.bottom_pane.restore_draft(text.to_string());
            return true;
        }
        let command = command.trim();
        if command.is_empty() {
            self.add_to_history(history_cell::new_info_event(
                USER_SHELL_COMMAND_HELP_TITLE.to_string(),
                Some(USER_SHELL_COMMAND_HELP_HINT.to_string()),
            ));
            return true;
        }
        self.user_shell = true;
        self.session_open = true;
        self.app_event_tx
            .send(AppEvent::Op(Op::UserShellCommand(command.to_string())));
        true
    }
}
