// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! What the chat does with each update of the turn: Codex's
//! `dispatch_event_msg` and its `on_*` handlers, over [`TurnUpdate`].

use hanafubuki_agent::{AgentReasoningId, AgentTaskPlanSnapshot};

use super::ChatWidget;
use crate::agent_bridge::{TurnEnd, TurnUpdate, Warning, WarningSource};
use crate::app_event::AppEvent;
use crate::history_cell::{self, ThinkingCell, ToolResultView};
use crate::streaming::controller::{PlanStreamController, StreamController};

const WORKING: &str = "Working";

impl ChatWidget {
    pub(crate) fn handle_turn_update(&mut self, update: TurnUpdate) {
        self.live_revision += 1;
        let Some(update) = self.handle_command_update(update) else {
            self.request_redraw();
            return;
        };
        if self.defers(&update) {
            self.deferred.push_back(update);
        } else {
            self.handle_now(update);
        }
        self.request_redraw();
    }

    /// While a message streams, what would be written below it waits for
    /// the stream to finish; once anything waits, everything after it
    /// waits too, so the order holds. A thinking cell written on its
    /// completion would otherwise land above the message's lines still to
    /// be committed.
    fn defers(&self, update: &TurnUpdate) -> bool {
        let streaming = self.stream_controller.is_some() || !self.deferred.is_empty();
        streaming
            && matches!(
                update,
                TurnUpdate::ReasoningStarted(_)
                    | TurnUpdate::ReasoningDelta { .. }
                    | TurnUpdate::ReasoningCompleted { .. }
                    | TurnUpdate::ToolStarted { .. }
                    | TurnUpdate::ToolCompleted { .. }
                    | TurnUpdate::ApprovalRequested(_)
                    | TurnUpdate::UserInputRequested(_)
                    | TurnUpdate::Compacted
                    | TurnUpdate::CompressionFailed(_)
                    | TurnUpdate::CompactionFailed(_)
            )
    }

    fn handle_now(&mut self, update: TurnUpdate) {
        match update {
            TurnUpdate::Started => self.on_task_started(),
            TurnUpdate::ReasoningStarted(reasoning_id) => self.on_reasoning_started(reasoning_id),
            TurnUpdate::ReasoningDelta {
                reasoning_id,
                delta,
            } => {
                if let Some((_, cell)) = self
                    .live_thinking
                    .iter_mut()
                    .find(|(id, _)| *id == reasoning_id)
                {
                    cell.push(&delta);
                }
            }
            TurnUpdate::ReasoningCompleted {
                reasoning_id,
                duration_millis,
            } => self.on_reasoning_completed(reasoning_id, duration_millis),
            TurnUpdate::MessageDelta(delta) => self.on_message_delta(delta),
            TurnUpdate::MessageCompleted => self.on_message_completed(),
            TurnUpdate::PlanDelta(delta) => self.on_plan_delta(delta),
            TurnUpdate::PlanCompleted => {
                if let Some(cell) = self
                    .plan_stream_controller
                    .take()
                    .and_then(|mut controller| controller.finalize())
                {
                    self.add_boxed_history(cell);
                }
            }
            TurnUpdate::ToolStarted {
                call_id,
                name,
                subject,
            } => self.on_tool_started(call_id, name, subject),
            TurnUpdate::ToolCompleted {
                call_id,
                is_error,
                result,
            } => self.on_tool_completed(call_id, is_error, result),
            TurnUpdate::ApprovalRequested(request) => {
                self.bottom_pane.push_approval_request(request)
            }
            TurnUpdate::ApprovalResolved(request_id) => {
                self.deferred.retain(|update| {
                    !matches!(update, TurnUpdate::ApprovalRequested(request) if request.request_id == request_id)
                });
                self.bottom_pane.dismiss_approval(request_id)
            }
            TurnUpdate::UserInputRequested(request) => {
                self.bottom_pane.push_user_input_request(request)
            }
            TurnUpdate::UserInputResolved(request_id) => {
                self.bottom_pane.dismiss_user_input(request_id)
            }
            TurnUpdate::TaskPlan(plan) => self.on_plan_update(&plan),
            TurnUpdate::Retry(header) => {
                self.retry_status_header = Some(header.clone());
                self.bottom_pane.update_status(header, None);
            }
            TurnUpdate::Compacted => self.add_to_history(history_cell::new_info_event(
                "Context compacted".to_string(),
                None,
            )),
            TurnUpdate::CompressionFailed(message) => {
                let warning = format!(
                    "Context compression failed; this turn goes on with the full history: {message}"
                );
                self.add_to_history(history_cell::new_warning_event(warning.clone()));
                self.warnings
                    .push(Warning::new(WarningSource::Panics, warning));
            }
            TurnUpdate::CompactionFailed(message) => {
                let warning = format!("Context compaction failed: {message}");
                self.add_to_history(history_cell::new_warning_event(warning.clone()));
                self.warnings
                    .push(Warning::new(WarningSource::Compaction, warning));
            }
            TurnUpdate::Ended(end) => self.on_turn_ended(end),
            TurnUpdate::Error(message) => {
                self.add_to_history(history_cell::new_error_event(message))
            }
            // The commands' updates are handled before an update is deferred.
            TurnUpdate::SessionOpened(_)
            | TurnUpdate::ModelChoices(_)
            | TurnUpdate::ModelChanged(_)
            | TurnUpdate::ModeChanged(_)
            | TurnUpdate::StatusReport(_)
            | TurnUpdate::Providers(_)
            | TurnUpdate::Mcp(_)
            | TurnUpdate::Skills(_)
            | TurnUpdate::Session(_)
            | TurnUpdate::Review(_)
            | TurnUpdate::Goal(_)
            | TurnUpdate::Hooks(_)
            | TurnUpdate::Plugins(_)
            | TurnUpdate::Settings(_)
            | TurnUpdate::Subagents(_) => {}
            TurnUpdate::SessionStatus(_) => {}
        }
    }

    fn on_task_started(&mut self) {
        self.bottom_pane.clear_quit_shortcut_hint();
        self.quit_shortcut_expires_at = None;
        self.quit_shortcut_key = None;
        self.bottom_pane.set_task_running(true);
        self.bottom_pane.set_interrupt_hint_visible(true);
        self.retry_status_header = None;
        self.plan_stream_controller = None;
        self.bottom_pane.update_status(WORKING.to_string(), None);
    }

    fn on_reasoning_started(&mut self, reasoning_id: AgentReasoningId) {
        self.restore_retry_status_header();
        self.live_thinking
            .push((reasoning_id, ThinkingCell::live(self.thinking_expanded)));
        self.sync_thinking_toggle();
    }

    fn on_reasoning_completed(&mut self, reasoning_id: AgentReasoningId, duration_millis: u64) {
        let Some(index) = self
            .live_thinking
            .iter()
            .position(|(id, _)| *id == reasoning_id)
        else {
            return;
        };
        let (_, mut cell) = self.live_thinking.remove(index);
        cell.complete(Some(duration_millis));
        self.add_to_history(cell);
        self.sync_thinking_toggle();
    }

    fn on_message_delta(&mut self, delta: String) {
        self.restore_retry_status_header();
        if self.stream_controller.is_none() {
            // After the turn wrote something other than a streamed message
            // (tool calls, a plan), a separator comes before the next one.
            if self.needs_final_message_separator && self.had_work_activity {
                let elapsed_seconds = self
                    .bottom_pane
                    .status_widget()
                    .map(crate::status_indicator_widget::StatusIndicatorWidget::elapsed_seconds);
                self.add_to_history(history_cell::FinalMessageSeparator::new(elapsed_seconds));
                self.had_work_activity = false;
            }
            self.needs_final_message_separator = false;
            self.stream_controller = Some(StreamController::new(
                self.last_rendered_width.get().map(|w| w.saturating_sub(2)),
            ));
        }
        if let Some(controller) = self.stream_controller.as_mut() {
            if controller.push(&delta) {
                self.app_event_tx.send(AppEvent::StartCommitAnimation);
            }
        }
    }

    fn on_message_completed(&mut self) {
        if let Some(cell) = self
            .stream_controller
            .take()
            .and_then(|mut controller| controller.finalize())
        {
            self.add_boxed_history(cell);
        }
        self.flush_deferred();
    }

    fn on_plan_delta(&mut self, delta: String) {
        self.restore_retry_status_header();
        let width = self.last_rendered_width.get().map(|w| w.saturating_sub(4));
        let controller = self
            .plan_stream_controller
            .get_or_insert_with(|| PlanStreamController::new(width));
        if controller.push(&delta) {
            self.app_event_tx.send(AppEvent::StartCommitAnimation);
        }
    }

    fn on_tool_started(&mut self, call_id: String, name: String, subject: Option<String>) {
        self.had_work_activity = true;
        self.running_tools
            .push(history_cell::new_tool_call(call_id, name, subject));
    }

    fn on_tool_completed(&mut self, call_id: String, is_error: bool, result: ToolResultView) {
        let index = self
            .running_tools
            .iter()
            .position(|cell| cell.call_id() == call_id);
        let mut cell = match index {
            Some(index) => self.running_tools.remove(index),
            None => history_cell::new_tool_call(call_id, "tool".to_string(), None),
        };
        cell.complete(is_error, result);
        self.add_to_history(cell);
    }

    fn on_plan_update(&mut self, plan: &AgentTaskPlanSnapshot) {
        self.had_work_activity = true;
        self.add_to_history(history_cell::new_plan_update(plan));
    }

    fn on_turn_ended(&mut self, end: TurnEnd) {
        self.on_message_completed();
        // A turn that failed mid-stream sends no completion: its thinking
        // ends with the time so far.
        for (_, mut cell) in std::mem::take(&mut self.live_thinking) {
            cell.complete(None);
            self.add_to_history(cell);
        }
        self.sync_thinking_toggle();
        if let Some(cell) = self
            .plan_stream_controller
            .take()
            .and_then(|mut controller| controller.finalize())
        {
            self.add_boxed_history(cell);
        }
        // A call the turn's end overtook is written as it stood.
        for cell in std::mem::take(&mut self.running_tools) {
            self.add_to_history(cell);
        }
        self.bottom_pane.dismiss_all_requests();
        self.bottom_pane.set_task_running(false);
        self.retry_status_header = None;
        self.had_work_activity = false;
        match end {
            TurnEnd::Completed => {}
            TurnEnd::Interrupted => self.add_to_history(history_cell::new_error_event(
                "Conversation interrupted - tell the model what to do differently.".to_string(),
            )),
            TurnEnd::Failed(message) => self.add_to_history(history_cell::new_error_event(message)),
        }
    }

    fn flush_deferred(&mut self) {
        while let Some(update) = self.deferred.pop_front() {
            self.handle_now(update);
        }
    }

    fn restore_retry_status_header(&mut self) {
        if self.retry_status_header.take().is_some() {
            self.bottom_pane.update_status(WORKING.to_string(), None);
        }
    }
}
