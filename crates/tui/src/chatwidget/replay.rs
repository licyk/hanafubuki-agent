// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! A reopened session shown from its start: its header, then the cells of
//! what it said so far; and the cells of an older page of its transcript,
//! which the owned transcript loads as the reader nears its start.

use hanafubuki_agent::AgentToolCallDisplay;

use super::ChatWidget;
use crate::agent_bridge::{HooksCall, OpenedSession, ReplayedItem};
use crate::app_event::{AgentCommand, AppEvent, Op};
use crate::history_cell::{self, HistoryCell, ToolCallCell, ToolGroup};
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
        self.tool_group = None;
        self.live_thinking.clear();
        self.work_entries.clear();
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
        for item in &opened.transcript {
            if let ReplayedItem::PermissionReview(work) = item {
                self.work_entries.insert(
                    work.id.clone(),
                    history_cell::LiveCell::new(history_cell::WorkCell::new(work.clone())),
                );
            }
        }
        for cell in self.replayed_cells(opened.transcript) {
            self.add_boxed_history(cell);
        }
        if let Some(work) = opened.work {
            if work.committed {
                // The committed summary above already occupies its history position.
                self.work_entries.insert(
                    work.id.clone(),
                    history_cell::LiveCell::new(history_cell::WorkCell::new(work)),
                );
            } else {
                self.on_work(work);
            }
        }
        self.had_work_activity = false;
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
    /// calls of the same group share one, as they did live.
    pub(crate) fn replayed_cells(&self, items: Vec<ReplayedItem>) -> Vec<Box<dyn HistoryCell>> {
        let mut cells: Vec<Box<dyn HistoryCell>> = Vec::new();
        let mut tool_group: Option<ToolCallCell> = None;
        for item in items {
            let group = match &item {
                ReplayedItem::Tool { name, .. } => ToolGroup::for_name(name),
                _ => None,
            };
            if group.is_none()
                || tool_group
                    .as_ref()
                    .is_some_and(|cell| cell.group() != group)
            {
                if let Some(cell) = tool_group.take() {
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
                // An answer and a plan as they are once their stream ended.
                ReplayedItem::Assistant(text) => {
                    if !text.trim().is_empty() {
                        cells.push(Box::new(history_cell::AgentMarkdownCell::new(
                            text,
                            self.directory.clone(),
                        )));
                    }
                }
                ReplayedItem::Plan(text) => cells.push(Box::new(
                    history_cell::ProposedPlanCell::new(text, self.directory.clone()),
                )),
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
                    match tool_group.as_mut() {
                        Some(run) if group.is_some() => run.join(cell),
                        _ if group.is_some() => tool_group = Some(cell),
                        _ => cells.push(Box::new(cell)),
                    }
                }
                ReplayedItem::UserShellCommand { call_id, display } => cells.push(Box::new(
                    history_cell::new_user_shell_command(call_id, display),
                )),
                ReplayedItem::BackgroundUpdates(summaries) => {
                    cells.push(Box::new(history_cell::new_background_updates(summaries)))
                }
                ReplayedItem::PermissionReview(work) => {
                    cells.push(Box::new(history_cell::WorkCell::new(work)))
                }
                ReplayedItem::Compacted { item_id, summary } => {
                    cells.push(Box::new(history_cell::WorkCell::replayed(item_id, summary)))
                }
            }
        }
        if let Some(cell) = tool_group {
            cells.push(Box::new(cell));
        }
        cells
    }
}
