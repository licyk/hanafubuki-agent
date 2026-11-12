// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! What the chat does with each update of the turn: Codex's
//! `dispatch_event_msg` and its `on_*` handlers, over [`TurnUpdate`].

use std::sync::Arc;

use hanafubuki_agent::{AgentReasoningId, AgentTaskPlanSnapshot, AgentToolCallDisplay};

use super::input_queue::Ending;
use super::ChatWidget;
use crate::agent_bridge::{TurnEnd, TurnUpdate, Warning, WarningSource};
use crate::app_event::AppEvent;
use crate::history_cell::{self, explores, LiveCell, ThinkingCell, ToolCallCell};

use crate::status_indicator_widget::StatusIndicatorWidget;
use crate::streaming::controller::{PlanStreamController, StreamController};

const WORKING: &str = "Working";

impl ChatWidget {
    pub(crate) fn handle_turn_update(&mut self, update: TurnUpdate) {
        let Some(update) = self.handle_command_update(update) else {
            self.request_redraw();
            return;
        };
        // An entry goes in its place at once, after the message so far, whose
        // lines still to be committed are committed first.
        if self.stream_controller.is_some() && adds_entry(&update) {
            self.finish_message();
        }
        match update {
            TurnUpdate::Started => self.on_task_started(),
            TurnUpdate::ReasoningStarted(reasoning_id) => self.on_reasoning_started(reasoning_id),
            TurnUpdate::ReasoningDelta {
                reasoning_id,
                delta,
            } => {
                if let Some((_, cell)) = self
                    .live_thinking
                    .iter()
                    .find(|(id, _)| *id == reasoning_id)
                {
                    cell.update(|cell| cell.push(&delta));
                }
            }
            TurnUpdate::ReasoningCompleted {
                reasoning_id,
                duration_millis,
            } => self.on_reasoning_completed(reasoning_id, duration_millis),
            TurnUpdate::Reasoning(reasoning) => self.bottom_pane.update_reasoning(&reasoning),
            TurnUpdate::MessageDelta(delta) => self.on_message_delta(delta),
            TurnUpdate::MessageCompleted => self.finish_message(),
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
            TurnUpdate::ToolCompleted { call_id, is_error } => {
                self.on_tool_completed(call_id, is_error)
            }
            TurnUpdate::ToolDisplayed { call_id, display } => {
                self.on_tool_displayed(&call_id, display)
            }
            TurnUpdate::ApprovalRequested(request) => {
                self.bottom_pane.push_approval_request(request)
            }
            TurnUpdate::ApprovalResolved(request_id) => {
                self.bottom_pane.dismiss_approval(request_id)
            }
            TurnUpdate::UserInputRequested(request) => {
                self.bottom_pane.push_user_input_request(request)
            }
            TurnUpdate::UserInputResolved(request_id) => {
                self.bottom_pane.dismiss_user_input(request_id)
            }
            TurnUpdate::TaskPlan(plan) => self.on_plan_update(&plan),
            TurnUpdate::Steered { id, outcome } => self.on_steered(id, outcome),
            TurnUpdate::SteeringConsumed(through) => self.on_steering_consumed(through),
            TurnUpdate::Undelivered(text) => self.on_undelivered(text),
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
            TurnUpdate::ContextWindowLearned(tokens) => {
                self.add_to_history(history_cell::new_info_event(
                    format!("The Provider refused the request as too long; this model's window is taken as {tokens} from now on."),
                    None,
                ))
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
            // The commands' updates are handled before.
            TurnUpdate::SessionOpened(_)
            | TurnUpdate::ModelChoices(_)
            | TurnUpdate::ModelChanged(_)
            | TurnUpdate::LearnedModelFactsForgotten { .. }
            | TurnUpdate::RoleChoices(_)
            | TurnUpdate::RoleChanged(_)
            | TurnUpdate::ModeChanged(_)
            | TurnUpdate::StatusReport(_)
            | TurnUpdate::ProviderModels(_)
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
        self.input_queue.turn_started();
    }

    fn on_reasoning_started(&mut self, reasoning_id: AgentReasoningId) {
        self.restore_retry_status_header();
        let cell = LiveCell::new(ThinkingCell::live(reasoning_id));
        self.insert_live_entry(cell.clone());
        self.live_thinking.push((reasoning_id, cell));
    }

    fn on_reasoning_completed(&mut self, reasoning_id: AgentReasoningId, duration_millis: u64) {
        let Some(index) = self
            .live_thinking
            .iter()
            .position(|(id, _)| *id == reasoning_id)
        else {
            return;
        };
        let (_, cell) = self.live_thinking.remove(index);
        cell.update(|cell| cell.complete(Some(duration_millis)));
        self.complete_live_entry(cell);
    }

    fn on_message_delta(&mut self, delta: String) {
        self.restore_retry_status_header();
        if self.stream_controller.is_none() {
            // After the turn wrote something other than a streamed message
            // (tool calls, a plan), a separator comes before the next one.
            if self.needs_final_message_separator && self.had_work_activity {
                let status = self.bottom_pane.status_widget();
                let separator = history_cell::FinalMessageSeparator::new(
                    status.map(StatusIndicatorWidget::elapsed_seconds),
                    status.and_then(StatusIndicatorWidget::thinking_duration),
                );
                self.add_to_history(separator);
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

    /// Commits what is left of the streaming message at once.
    fn finish_message(&mut self) {
        if let Some(cell) = self
            .stream_controller
            .take()
            .and_then(|mut controller| controller.finalize())
        {
            self.add_boxed_history(cell);
        }
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

    /// An exploring call joins the exploring calls just before it;
    /// another call is an entry of its own.
    fn on_tool_started(&mut self, call_id: String, name: String, subject: Option<String>) {
        self.had_work_activity = true;
        let explores = explores(&name);
        let call = history_cell::new_tool_call(call_id.clone(), name, subject);
        if let Some(run) = self.exploration.clone().filter(|_| explores) {
            run.update(|cell| cell.join(call));
            self.running_tools.push((call_id, run));
            return;
        }
        let cell = LiveCell::new(call);
        self.insert_live_entry(cell.clone());
        if explores {
            self.exploration = Some(cell.clone());
        }
        self.running_tools.push((call_id, cell));
    }

    /// The call keeps its place and shape, marked finished, until what it
    /// shows arrives.
    fn on_tool_completed(&mut self, call_id: String, is_error: bool) {
        let index = self.running_tools.iter().position(|(id, _)| *id == call_id);
        let cell = match index {
            Some(index) => self.running_tools.remove(index).1,
            None => {
                let cell = LiveCell::new(history_cell::new_tool_call(
                    call_id.clone(),
                    "tool".to_string(),
                    None,
                ));
                self.insert_live_entry(cell.clone());
                cell
            }
        };
        cell.update(|cell| cell.complete(&call_id, is_error));
        self.awaiting_display.push((call_id, cell));
    }

    /// What a finished call shows is its last revision.
    fn on_tool_displayed(&mut self, call_id: &str, display: Option<AgentToolCallDisplay>) {
        let index = self
            .awaiting_display
            .iter()
            .position(|(id, _)| id == call_id);
        let Some(index) = index else {
            return;
        };
        let (_, cell) = self.awaiting_display.remove(index);
        if let Some(display) = display {
            cell.update(|cell| cell.show(call_id, display));
        }
        self.complete_settled_tool(cell);
    }

    /// No exploring call joins the run of them from now on.
    pub(super) fn close_exploration(&mut self) {
        if let Some(run) = self.exploration.take() {
            self.complete_settled_tool(run);
        }
    }

    /// A tool entry no call runs or waits in, and no call may join, is
    /// complete: Scrollback's tail may write it from then on.
    fn complete_settled_tool(&self, cell: Arc<LiveCell<ToolCallCell>>) {
        let same = |other: &Arc<LiveCell<ToolCallCell>>| Arc::ptr_eq(other, &cell);
        let unsettled = self.exploration.as_ref().is_some_and(same)
            || self.running_tools.iter().any(|(_, other)| same(other))
            || self.awaiting_display.iter().any(|(_, other)| same(other));
        if !unsettled {
            self.complete_live_entry(cell);
        }
    }

    fn on_plan_update(&mut self, plan: &AgentTaskPlanSnapshot) {
        self.had_work_activity = true;
        self.add_to_history(history_cell::new_plan_update(plan));
    }

    fn on_turn_ended(&mut self, end: TurnEnd) {
        self.finish_message();
        // A turn that failed mid-stream sends no completion: its thinking
        // ends with the time so far.
        for (_, cell) in std::mem::take(&mut self.live_thinking) {
            cell.update(|cell| cell.complete(None));
            self.complete_live_entry(cell);
        }
        if let Some(cell) = self
            .plan_stream_controller
            .take()
            .and_then(|mut controller| controller.finalize())
        {
            self.add_boxed_history(cell);
        }
        // A call the turn's end overtook is written as it stood.
        let mut ended: Vec<Arc<LiveCell<ToolCallCell>>> =
            self.exploration.take().into_iter().collect();
        for (_, cell) in std::mem::take(&mut self.running_tools) {
            if !ended.iter().any(|other| Arc::ptr_eq(other, &cell)) {
                ended.push(cell);
            }
        }
        for cell in ended {
            self.complete_settled_tool(cell);
        }
        self.bottom_pane.dismiss_all_requests();
        self.bottom_pane.set_task_running(false);
        self.retry_status_header = None;
        self.had_work_activity = false;
        let ending = match end {
            TurnEnd::Completed => Ending::Finished,
            TurnEnd::Interrupted if self.input_queue.sends_steers_after_interrupt() => {
                self.add_info_message(
                    "Model interrupted to submit steer instructions.".to_string(),
                );
                Ending::Interrupted
            }
            TurnEnd::Interrupted => {
                self.add_to_history(history_cell::new_error_event(
                    "Conversation interrupted - tell the model what to do differently.".to_string(),
                ));
                Ending::Interrupted
            }
            TurnEnd::Failed(message) => {
                self.add_to_history(history_cell::new_error_event(message));
                Ending::Finished
            }
        };
        self.input_queue.turn_ended(ending);
        self.advance_input_queue();
    }

    fn restore_retry_status_header(&mut self) {
        if self.retry_status_header.take().is_some() {
            self.bottom_pane.update_status(WORKING.to_string(), None);
        }
    }
}

/// Whether `update` adds an entry to the conversation, rather than change
/// one in place or touch only the bottom pane.
fn adds_entry(update: &TurnUpdate) -> bool {
    matches!(
        update,
        TurnUpdate::ReasoningStarted(_)
            | TurnUpdate::ToolStarted { .. }
            | TurnUpdate::SteeringConsumed(_)
            | TurnUpdate::Compacted
            | TurnUpdate::CompressionFailed(_)
            | TurnUpdate::CompactionFailed(_)
            | TurnUpdate::ContextWindowLearned(_)
    )
}
