// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! A reopened session shown from its start: its header, then the cells of
//! what it said so far; and the cells of an older page of its transcript,
//! which the owned transcript loads as the reader nears its start.

use super::ChatWidget;
use crate::agent_bridge::{OpenedSession, ReplayedItem};
use crate::history_cell::{self, HistoryCell};
use crate::streaming::controller::{PlanStreamController, StreamController};

impl ChatWidget {
    /// Shows the session from its start. What the previous session left
    /// running is gone with it.
    pub(super) fn on_session_opened(&mut self, opened: OpenedSession) {
        self.running_tools.clear();
        self.live_thinking.clear();
        self.live_revision += 1;
        self.sync_thinking_toggle();
        self.stream_controller = None;
        self.plan_stream_controller = None;
        self.deferred.clear();
        self.bottom_pane.dismiss_all_requests();
        self.bottom_pane.set_task_running(false);
        self.retry_status_header = None;
        self.had_work_activity = false;
        self.mode = opened.mode;
        self.session_open = opened.session_id.is_some();
        self.setup_needed = opened.setup_needed;
        let values = self.appearance.values_mut();
        values.model = opened.model.clone();
        values.status = None;
        self.sync_status_line();
        self.add_to_history(history_cell::SessionHeaderHistoryCell::new(
            opened.model,
            self.directory.clone(),
            env!("CARGO_PKG_VERSION"),
        ));
        // The owned transcript loads the earlier messages instead.
        if opened.older.is_some() && !self.owned_transcript {
            self.add_to_history(history_cell::new_info_event(
                "Earlier messages are not shown.".to_string(),
                None,
            ));
        }
        for cell in self.replayed_cells(opened.transcript) {
            self.add_boxed_history(cell);
        }
        self.needs_final_message_separator = false;
        if self.setup_needed {
            self.open_setup();
        }
    }

    /// The cells of replayed transcript items, oldest first.
    pub(crate) fn replayed_cells(&self, items: Vec<ReplayedItem>) -> Vec<Box<dyn HistoryCell>> {
        let width = self.last_rendered_width.get();
        let mut cells: Vec<Box<dyn HistoryCell>> = Vec::new();
        for item in items {
            match item {
                ReplayedItem::User(text) => {
                    cells.push(Box::new(history_cell::new_user_prompt(text)))
                }
                ReplayedItem::Reasoning {
                    text,
                    duration_millis,
                    truncated,
                } => cells.push(Box::new(history_cell::ThinkingCell::completed(
                    text,
                    duration_millis,
                    truncated,
                    self.thinking_expanded,
                ))),
                ReplayedItem::Assistant(text) => {
                    let mut controller = StreamController::new(width.map(|w| w.saturating_sub(2)));
                    controller.push(&text);
                    cells.extend(controller.finalize());
                }
                ReplayedItem::Plan(text) => {
                    let mut controller =
                        PlanStreamController::new(width.map(|w| w.saturating_sub(4)));
                    controller.push(&text);
                    cells.extend(controller.finalize());
                }
                ReplayedItem::Tool {
                    call_id,
                    name,
                    is_error,
                    result,
                } => {
                    let mut cell = history_cell::new_tool_call(call_id, name, None);
                    cell.complete(is_error, result);
                    cells.push(Box::new(cell));
                }
                ReplayedItem::Compacted => cells.push(Box::new(history_cell::new_info_event(
                    "Context compacted".to_string(),
                    None,
                ))),
            }
        }
        cells
    }
}
