//! The one adapter between the interface and the Agent (it stands where
//! Codex's TUI has its `ConversationManager` and `CodexConversation`): it
//! maps the Agent's turn events onto what the chat widget shows
//! ([`TurnUpdate`]), and the interface's [`Op`]s onto calls of the facade.
//!
//! It runs on the app loop's thread. The Agent's sink only forwards events
//! into the loop's channel (`emit` must not block); a call that waits runs
//! as a task of the Agent's runtime and reports back with an
//! [`AgentReply`].

use std::sync::Arc;

use hanafubuki_agent::{
    AgentAvailability, AgentEvent, AgentEventKind, AgentEventSink, AgentHandle,
    AgentHistorySortDirection, AgentMessageId, AgentMessagePhase, AgentSessionId, AgentSlot,
    AgentStatusEventSink, AgentSteeringRequest, AgentTaskPlanSnapshot, AgentTurnAccepted,
    AgentTurnPageRequest, AgentTurnRequest, AgentTurnTerminalReason, AgentUserInputRequest,
    AgentUserInputRequestId, ApprovalRequest, ApprovalRequestId, ConversationItem,
    ToolOutputContent,
};

use crate::app_event::{AppEvent, AppEventSender, Op};
use crate::history_cell::ToolResultView;

/// The tool whose arguments are a patch the transcript shows as a diff.
const APPLY_PATCH: &str = "apply_patch";
/// How many of the turn's latest items are read for a finished call.
const TOOL_ITEMS_PAGE: usize = 64;

/// What the chat widget is told of the turn.
#[derive(Debug)]
pub(crate) enum TurnUpdate {
    Started,
    /// What the turn is doing now, for the status line.
    Status(String),
    MessageDelta {
        id: AgentMessageId,
        delta: String,
    },
    MessageCompleted {
        id: AgentMessageId,
        final_answer: bool,
    },
    PlanDelta(String),
    PlanCompleted,
    ToolStarted {
        call_id: String,
        name: String,
        subject: Option<String>,
    },
    ToolCompleted {
        call_id: String,
        is_error: bool,
        result: ToolResultView,
    },
    ApprovalRequested(ApprovalRequest),
    /// Answered, by the person or by a policy; its overlay closes.
    ApprovalResolved(ApprovalRequestId),
    UserInputRequested(AgentUserInputRequest),
    UserInputResolved(AgentUserInputRequestId),
    TaskPlan(AgentTaskPlanSnapshot),
    Retry(String),
    Ended(TurnEnd),
    /// A call into the Agent failed.
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TurnEnd {
    Completed,
    Interrupted,
    Failed(String),
}

/// The outcome of a call into the Agent that ran as a task.
pub(crate) enum AgentReply {
    TurnAccepted(AgentTurnAccepted),
    TurnRejected(String),
    Failed(String),
}

pub(crate) struct AgentBridge {
    agent: AgentHandle,
    slot: AgentSlot,
    session_id: AgentSessionId,
    app_event_tx: AppEventSender,
    turn: Option<AgentTurnAccepted>,
    /// A turn was submitted and has not ended; `turn` is unknown until the
    /// submission is accepted.
    turn_running: bool,
    /// What the person sent while the submission was being accepted, to
    /// steer the turn with once it is.
    pending_steering: Vec<String>,
}

impl AgentBridge {
    pub(crate) fn new(
        agent: AgentHandle,
        slot: AgentSlot,
        session_id: AgentSessionId,
        app_event_tx: AppEventSender,
    ) -> Self {
        Self {
            agent,
            slot,
            session_id,
            app_event_tx,
            turn: None,
            turn_running: false,
            pending_steering: Vec::new(),
        }
    }

    /// Carries out what the interface asked; what it leads to arrives as
    /// events and replies.
    pub(crate) fn submit(&mut self, op: Op) -> Option<TurnUpdate> {
        match op {
            Op::UserInput(text) => {
                match (self.turn_running, self.turn) {
                    (false, _) => self.start_turn(text),
                    (true, Some(turn)) => self.steer(turn, text),
                    (true, None) => self.pending_steering.push(text),
                }
                None
            }
            Op::Interrupt => {
                let turn = self.turn?;
                self.spawn(move |agent| async move {
                    agent
                        .cancel_turn(turn.session_id, turn.turn_id, turn.generation)
                        .await
                        .map(|_| None)
                        .map_err(|error| format!("the turn was not interrupted: {error}"))
                });
                None
            }
            Op::Approval {
                request_id,
                decision,
            } => {
                let turn = self.turn?;
                self.agent
                    .resolve_approval(
                        turn.session_id,
                        turn.turn_id,
                        turn.generation,
                        request_id,
                        decision,
                    )
                    .err()
                    .map(|error| {
                        TurnUpdate::Error(format!("the approval was not answered: {error}"))
                    })
            }
            Op::UserInputAnswer {
                request_id,
                answers,
            } => {
                let turn = self.turn?;
                self.agent
                    .resolve_user_input(
                        turn.session_id,
                        turn.turn_id,
                        turn.generation,
                        request_id,
                        answers,
                    )
                    .err()
                    .map(|error| TurnUpdate::Error(format!("the answer was not taken: {error}")))
            }
        }
    }

    pub(crate) fn reply(&mut self, reply: AgentReply) -> Option<TurnUpdate> {
        match reply {
            AgentReply::TurnAccepted(turn) => {
                self.turn = Some(turn);
                for text in std::mem::take(&mut self.pending_steering) {
                    self.steer(turn, text);
                }
                None
            }
            AgentReply::TurnRejected(message) => {
                self.turn_running = false;
                self.pending_steering.clear();
                Some(TurnUpdate::Ended(TurnEnd::Failed(message)))
            }
            AgentReply::Failed(message) => Some(TurnUpdate::Error(message)),
        }
    }

    /// What the chat widget makes of one of the Agent's events, if anything.
    pub(crate) fn event(&mut self, event: AgentEvent) -> Option<TurnUpdate> {
        if event.session_id != self.session_id {
            return None;
        }
        let update = match event.kind {
            AgentEventKind::TurnStarted => TurnUpdate::Started,
            AgentEventKind::ReasoningHeadlineUpdated { headline } => TurnUpdate::Status(headline),
            AgentEventKind::AssistantTextDelta { message_id, delta } => TurnUpdate::MessageDelta {
                id: message_id,
                delta,
            },
            AgentEventKind::AssistantMessageCompleted { message_id, phase } => {
                TurnUpdate::MessageCompleted {
                    id: message_id,
                    final_answer: phase != Some(AgentMessagePhase::Commentary),
                }
            }
            AgentEventKind::ProposedPlanDelta { delta, .. } => TurnUpdate::PlanDelta(delta),
            AgentEventKind::ProposedPlanCompleted { .. } => TurnUpdate::PlanCompleted,
            AgentEventKind::ToolStarted {
                call_id,
                name,
                presentation,
            } => TurnUpdate::ToolStarted {
                call_id,
                name,
                subject: presentation.subject,
            },
            AgentEventKind::ToolCompleted { call_id, is_error } => {
                let result = event
                    .turn_id
                    .map(|turn_id| self.tool_result(turn_id, &call_id))
                    .unwrap_or_default();
                TurnUpdate::ToolCompleted {
                    call_id,
                    is_error,
                    result,
                }
            }
            AgentEventKind::ApprovalRequested { request } => TurnUpdate::ApprovalRequested(request),
            AgentEventKind::ApprovalResolved { request_id, .. } => {
                TurnUpdate::ApprovalResolved(request_id)
            }
            AgentEventKind::UserInputRequested { request } => {
                TurnUpdate::UserInputRequested(request)
            }
            AgentEventKind::UserInputResolved { resolution } => {
                TurnUpdate::UserInputResolved(resolution.request_id)
            }
            AgentEventKind::TaskPlanUpdated { plan } => TurnUpdate::TaskPlan(plan),
            AgentEventKind::ModelRetryScheduled {
                current_retry,
                maximum_retries,
                reason,
                ..
            } => TurnUpdate::Retry(format!(
                "Reconnecting {current_retry}/{maximum_retries}: {reason}"
            )),
            AgentEventKind::TurnCompleted { .. } => self.ended(TurnEnd::Completed),
            AgentEventKind::TurnCancelled {
                reason: AgentTurnTerminalReason::UserInterrupted,
            } => self.ended(TurnEnd::Interrupted),
            AgentEventKind::TurnCancelled { reason } => self.ended(TurnEnd::Failed(format!(
                "the turn was cancelled ({reason:?})"
            ))),
            AgentEventKind::TurnFailed { message, .. } => self.ended(TurnEnd::Failed(message)),
            _ => return None,
        };
        Some(update)
    }

    /// When the Agent stopped (a kernel fault) during a turn, the turn's
    /// events stop with it: the turn is told it failed.
    pub(crate) fn status_changed(&mut self) -> Option<TurnUpdate> {
        let status = self.slot.status();
        let running = matches!(status.availability, AgentAvailability::Running)
            && status.generation == self.agent.generation();
        if running || !self.turn_running {
            return None;
        }
        let message = match status.availability {
            AgentAvailability::Faulted { reason, .. } => {
                format!("the Agent stopped after an internal error: {reason}")
            }
            _ => "the Agent stopped".to_string(),
        };
        Some(self.ended(TurnEnd::Failed(message)))
    }

    fn ended(&mut self, end: TurnEnd) -> TurnUpdate {
        self.turn = None;
        self.turn_running = false;
        self.pending_steering.clear();
        TurnUpdate::Ended(end)
    }

    fn start_turn(&mut self, text: String) {
        self.turn_running = true;
        let session_id = self.session_id;
        let sink = self.sink();
        let app_event_tx = self.app_event_tx.clone();
        self.agent.runtime().handle().spawn({
            let agent = self.agent.clone();
            async move {
                let request = AgentTurnRequest {
                    text,
                    images: Vec::new(),
                    skills: Vec::new(),
                    host: Default::default(),
                };
                let reply = match agent.submit_turn_with_sink(session_id, request, sink).await {
                    Ok(turn) => AgentReply::TurnAccepted(turn),
                    Err(error) => AgentReply::TurnRejected(error),
                };
                app_event_tx.send(AppEvent::AgentReply(reply));
            }
        });
    }

    fn steer(&self, turn: AgentTurnAccepted, text: String) {
        let sink = self.sink();
        self.spawn(move |agent| async move {
            let request = AgentSteeringRequest {
                text,
                images: Vec::new(),
                host: Default::default(),
            };
            agent
                .steer_turn_with_sink(
                    turn.session_id,
                    turn.turn_id,
                    turn.generation,
                    request,
                    sink,
                )
                .await
                .map(|_| None)
                .map_err(|error| format!("the message did not reach the turn: {error}"))
        });
    }

    /// Runs a call on the Agent's runtime; its failure comes back as a
    /// reply.
    fn spawn<F>(&self, call: impl FnOnce(AgentHandle) -> F)
    where
        F: std::future::Future<Output = Result<Option<AgentReply>, String>> + Send + 'static,
    {
        let app_event_tx = self.app_event_tx.clone();
        let call = call(self.agent.clone());
        self.agent.runtime().handle().spawn(async move {
            match call.await {
                Ok(Some(reply)) => app_event_tx.send(AppEvent::AgentReply(reply)),
                Ok(None) => {}
                Err(message) => {
                    app_event_tx.send(AppEvent::AgentReply(AgentReply::Failed(message)))
                }
            }
        });
    }

    fn sink(&self) -> Arc<dyn AgentEventSink> {
        Arc::new(EventSink(self.app_event_tx.clone()))
    }

    /// What a finished call left in the turn's history: its output, and for
    /// a patch the patch read as a diff. Tool events carry neither.
    fn tool_result(&self, turn_id: hanafubuki_agent::AgentTurnId, call_id: &str) -> ToolResultView {
        let page = self.agent.read_turn_page(AgentTurnPageRequest {
            session_id: self.session_id,
            turn_id,
            page_size: TOOL_ITEMS_PAGE,
            cursor: None,
            direction: AgentHistorySortDirection::Descending,
            through_revision: None,
        });
        let Ok(page) = page else {
            return ToolResultView::default();
        };
        let mut view = ToolResultView::default();
        for item in page.items {
            match item.content {
                ConversationItem::ToolCall {
                    call_id: id,
                    name,
                    arguments,
                    ..
                } if id == call_id && name == APPLY_PATCH => {
                    let patch = match &arguments {
                        serde_json::Value::String(patch) => Some(patch.as_str()),
                        value => value.get("patch").and_then(serde_json::Value::as_str),
                    };
                    view.patch = patch.map(|patch| self.agent.preview_patch(patch));
                }
                ConversationItem::ToolOutput {
                    call_id: id,
                    output,
                    ..
                } if id == call_id => {
                    for content in output.content {
                        match content {
                            ToolOutputContent::Text { text } => view.text = Some(text),
                            ToolOutputContent::Structured { value } => {
                                if let Some(output) = value.get("output").and_then(|v| v.as_str()) {
                                    view.text = Some(output.to_string());
                                }
                                view.exit_code = value
                                    .get("exitCode")
                                    .and_then(serde_json::Value::as_i64)
                                    .and_then(|code| i32::try_from(code).ok());
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        view
    }
}

/// The Agent's events into the app loop: `emit` must not block.
struct EventSink(AppEventSender);

impl AgentEventSink for EventSink {
    fn emit(&self, event: AgentEvent) {
        self.0.send(AppEvent::Agent(Box::new(event)));
    }
}

/// The Agent slot's changes into the app loop.
pub(crate) struct StatusSink(pub(crate) AppEventSender);

impl AgentStatusEventSink for StatusSink {
    fn status_changed(&self) {
        self.0.send(AppEvent::AgentStatusChanged);
    }
}
