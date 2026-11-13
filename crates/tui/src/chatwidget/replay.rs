// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! A reopened session shown from its start: its header, then the cells of
//! what it said so far; and the cells of an older page of its transcript,
//! which the owned transcript loads as the reader nears its start.

use hanafubuki_agent::AgentToolCallDisplay;

use super::ChatWidget;
use crate::agent_bridge::{HooksCall, OpenedSession, ReplayedItem};
use crate::app_event::{AgentCommand, AppEvent, Op};
use crate::history_cell::{self, explores, HistoryCell, ToolCallCell};
use crate::streaming::controller::{PlanStreamController, StreamController};
use crate::user_message::image_rows;

impl ChatWidget {
    /// Shows the session from its start. What the previous session left
    /// running is gone with it, and so is what waited for it: a call
    /// waiting for its display will have none.
    pub(super) fn on_session_opened(&mut self, opened: OpenedSession) {
        self.input_queue = super::input_queue::InputQueue::default();
        self.user_shell = false;
        self.sync_pending_input();
        self.running_tools.clear();
        for (call_id, cell) in std::mem::take(&mut self.awaiting_display) {
            cell.update(|cell| {
                cell.show(
                    &call_id,
                    AgentToolCallDisplay {
                        subject: None,
                        display: None,
                    },
                )
            });
        }
        self.exploration = None;
        self.live_thinking.clear();
        self.stream_controller = None;
        self.plan_stream_controller = None;
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
        if !self.hooks_checked {
            self.hooks_checked = true;
            self.app_event_tx
                .send(AppEvent::Op(Op::Command(AgentCommand::Hooks(
                    HooksCall::Check,
                ))));
        }
    }

    /// The cells of replayed transcript items, oldest first. Consecutive
    /// exploring calls share one, as they did live.
    pub(crate) fn replayed_cells(&self, items: Vec<ReplayedItem>) -> Vec<Box<dyn HistoryCell>> {
        let width = self.last_rendered_width.get();
        let mut cells: Vec<Box<dyn HistoryCell>> = Vec::new();
        let mut exploration: Option<ToolCallCell> = None;
        for item in items {
            let explores = matches!(&item, ReplayedItem::Tool { name, .. } if explores(name));
            if !explores {
                if let Some(cell) = exploration.take() {
                    cells.push(Box::new(cell));
                }
            }
            match item {
                ReplayedItem::User { text, images } => cells.push(Box::new(
                    history_cell::user_prompt(text, image_rows(images.iter().map(String::as_str))),
                )),
                ReplayedItem::Reasoning {
                    item_id,
                    text,
                    duration_millis,
                    truncated,
                } => cells.push(Box::new(history_cell::ThinkingCell::replayed(
                    item_id,
                    text,
                    duration_millis,
                    truncated,
                ))),
                ReplayedItem::Assistant(text) => {
                    let mut controller = StreamController::new(
                        width.map(|w| w.saturating_sub(2)),
                        Some(self.directory.clone()),
                    );
                    controller.push(&text);
                    cells.extend(controller.finalize());
                }
                ReplayedItem::Plan(text) => {
                    let mut controller = PlanStreamController::new(
                        width.map(|w| w.saturating_sub(4)),
                        Some(self.directory.clone()),
                    );
                    controller.push(&text);
                    cells.extend(controller.finalize());
                }
                ReplayedItem::Tool {
                    call_id,
                    name,
                    subject,
                    is_error,
                    display,
                } => {
                    let mut cell = history_cell::new_tool_call(call_id.clone(), name, subject);
                    cell.complete(&call_id, is_error);
                    cell.show(
                        &call_id,
                        AgentToolCallDisplay {
                            subject: None,
                            display,
                        },
                    );
                    match exploration.as_mut() {
                        Some(run) if explores => run.join(cell),
                        _ if explores => exploration = Some(cell),
                        _ => cells.push(Box::new(cell)),
                    }
                }
                ReplayedItem::UserShellCommand { call_id, display } => cells.push(Box::new(
                    history_cell::new_user_shell_command(call_id, display),
                )),
                ReplayedItem::Compacted => cells.push(Box::new(history_cell::new_info_event(
                    "Context compacted".to_string(),
                    None,
                ))),
            }
        }
        if let Some(cell) = exploration {
            cells.push(Box::new(cell));
        }
        cells
    }
}
