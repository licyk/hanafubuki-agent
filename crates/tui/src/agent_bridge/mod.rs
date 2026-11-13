//! The one adapter between the interface and the Agent (it stands where
//! Codex's TUI has its `ConversationManager` and `CodexConversation`): it
//! maps the Agent's turn events onto what the chat widget shows
//! ([`TurnUpdate`]), and the interface's [`Op`]s onto calls of the facade:
//! the turn's (here), the slash commands' (`commands`, `sessions`,
//! `permissions`, `workspace`; `/review`, `/goal`, `/hooks` and
//! `/subagents` in modules of their own, the side conversation in `side`),
//! the steering of a running turn (`steering`), the reading of a session's
//! transcript into what the chat replays and of a finished call's display
//! (`replay`), and the editing of an earlier message (`edit`). What the person sends while the Agent works
//! is queued by the chat; the bridge only starts and steers turns.
//!
//! The interface starts with no session: the first message creates it, or
//! a command that needs one (`Op::EnsureSession`), and `/new` drops back to
//! none. What is asked while it is being created waits for it.
//!
//! It runs on the app loop's thread. The Agent's sink only forwards events
//! into the loop's channel (`emit` must not block); a call that waits runs
//! as a task of the Agent's runtime and reports back with an
//! [`AgentReply`].

mod agents;
mod catalog;
mod center_reads;
mod commands;
mod edit;
mod goal;
mod hooks;
mod mcp;
mod permissions;
mod plugins;
mod providers;
mod replay;
mod review;
mod sessions;
mod settings;
mod side;
mod status;
mod steering;
mod subagents;
mod workspace;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use hanafubuki_agent::{
    AgentAvailability, AgentCompactionStatus, AgentCompactionTrigger, AgentEvent, AgentEventKind,
    AgentEventSink, AgentGoal, AgentHandle, AgentPermissionMode, AgentReasoningId,
    AgentReasoningSnapshot, AgentRuntime, AgentSessionContext, AgentSessionId, AgentSessionMode,
    AgentSlot, AgentStatusEventSink, AgentTaskPlanSnapshot, AgentToolCallDisplay,
    AgentToolProgressUpdate, AgentTurnAccepted, AgentTurnRequest, AgentTurnTerminalReason,
    AgentUserInputRequest, AgentUserInputRequestId, ApprovalRequest, ApprovalRequestId,
    SkillCatalogSnapshot,
};
use tokio::sync::oneshot;

use crate::app_event::{AppEvent, AppEventSender, Op};
pub(crate) use catalog::{CatalogCall, CatalogOutcome, CatalogReply};
pub(crate) use center_reads::{
    CenterJob, CenterRead, CenterReader, Exchange, LiveSession, RootsRead, Waiting,
};
pub(crate) use commands::{setup_needed, with_model, ModelChoices, RoleChoices, StatusReport};
pub(crate) use goal::{GoalCall, GoalReply};
pub(crate) use hooks::{needing_review, HooksCall, HooksReply};
pub(crate) use mcp::{McpCall, McpReply};
pub(crate) use permissions::DeniedAction;
pub(crate) use plugins::{PluginsCall, PluginsDone, PluginsReply, PluginsState};
pub(crate) use providers::ProviderModels;
pub(crate) use replay::{OlderTranscript, OpenedSession, ReplayedItem};
pub(crate) use review::{ReviewCall, ReviewEnd, ReviewReply};
pub(crate) use sessions::{ExportDestination, SessionCall, SessionReply};
pub(crate) use settings::{MemoriesState, SettingsCall, SettingsReply, Warning, WarningSource};
pub(crate) use side::OpenedSide;
pub(crate) use status::{SessionStatus, UsageSummary};
pub(crate) use steering::SteerOutcome;
pub(crate) use subagents::{ChildApproval, SubagentsCall, SubagentsReply};

/// What the chat widget is told: the turn's progress, and what the
/// commands it asked for found.
#[derive(Debug)]
pub(crate) enum TurnUpdate {
    Started,
    ReasoningStarted(AgentReasoningId),
    ReasoningDelta {
        reasoning_id: AgentReasoningId,
        delta: String,
    },
    ReasoningCompleted {
        reasoning_id: AgentReasoningId,
        duration_millis: u64,
    },
    /// The turn's thinking time, for the status line.
    Reasoning(AgentReasoningSnapshot),
    MessageDelta(String),
    MessageCompleted,
    PlanDelta(String),
    PlanCompleted,
    ToolStarted {
        call_id: String,
        name: String,
        subject: Option<String>,
    },
    /// The call finished; what it shows follows as `ToolDisplayed`.
    ToolCompleted {
        call_id: String,
        is_error: bool,
    },
    /// What a finished call shows, read off the interface's thread; `None`
    /// when it could not be read.
    ToolDisplayed {
        call_id: String,
        display: Option<AgentToolCallDisplay>,
    },
    /// Output of the process a call started, as it streams: terminal text
    /// (a PTY's as its screen's rows), after a gap when `truncated`.
    ToolOutput {
        call_id: String,
        text: String,
        truncated: bool,
    },
    ApprovalRequested(ApprovalRequest),
    /// Answered, by the person or by a policy; its overlay closes.
    ApprovalResolved(ApprovalRequestId),
    UserInputRequested(AgentUserInputRequest),
    UserInputResolved(AgentUserInputRequestId),
    TaskPlan(AgentTaskPlanSnapshot),
    /// What became of the steering message `id` the chat sent.
    Steered {
        id: u64,
        outcome: SteerOutcome,
    },
    /// The turn took the steering it was sent through this sequence.
    SteeringConsumed(u64),
    /// A message that was to start a turn reached none: it goes back to
    /// the composer.
    Undelivered(String),
    Retry(String),
    Ended(TurnEnd),
    /// A call into the Agent failed.
    Error(String),
    /// The interface now shows this session: a new one, or one reopened
    /// with its transcript.
    SessionOpened(OpenedSession),
    ModelChoices(ModelChoices),
    ModelChanged(String),
    /// What `/role` offers.
    RoleChoices(RoleChoices),
    /// The session's role is now this one (`None` for `default`).
    RoleChanged(Option<String>),
    ModeChanged(AgentSessionMode),
    /// The session's context was compacted: by `/compact`, automatically
    /// or in the background.
    Compacted,
    /// The hierarchical compression failed for a request: the turn goes on
    /// with the full history.
    CompressionFailed(String),
    /// An automatic compaction failed, with its error.
    CompactionFailed(String),
    /// The Provider refused a request as too long, and the session's model
    /// is taken to have a window of this many tokens from now on.
    ContextWindowLearned(u64),
    /// What was learned of `model` was forgotten (`forgotten`: whether
    /// there was anything).
    LearnedModelFactsForgotten {
        model: String,
        forgotten: bool,
    },
    StatusReport(Box<StatusReport>),
    ContextReport(Box<AgentSessionContext>),
    ProviderModels(ProviderModels),
    Mcp(McpReply),
    Skills(SkillCatalogSnapshot),
    Session(SessionReply),
    /// What `/warnings` and `/memories` asked for.
    Settings(SettingsReply),
    /// What the status line and the terminal title show of the session.
    SessionStatus(Box<SessionStatus>),
    Review(ReviewReply),
    Goal(GoalReply),
    Hooks(HooksReply),
    Plugins(PluginsReply),
    Subagents(SubagentsReply),
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
    /// The session the interface now shows: one reopened, or none yet.
    SessionOpened(OpenedSession),
    /// The session the first message (or a command) asked for exists.
    SessionCreated(AgentSessionId),
    SessionNotCreated(String),
    /// What a command found, for the chat to show as it is.
    Update(Box<TurnUpdate>),
    /// What the session's status reads once, at open.
    StatusLoaded {
        session_id: AgentSessionId,
        permission_mode: Option<AgentPermissionMode>,
        goal: Option<AgentGoal>,
    },
    /// The review that ran ended.
    ReviewEnded(ReviewEnd),
    /// The working directory moved into a new worktree (`/worktree`).
    EnteredWorktree(workspace::EnteredWorktree),
    /// What a finished call of `session_id` shows.
    ToolDisplayed {
        session_id: AgentSessionId,
        call_id: String,
        display: Option<AgentToolCallDisplay>,
    },
    /// A subagent of `parent` waits on an approval.
    ChildApproval(Box<ChildApproval>),
    /// It was answered, or its turn ended.
    ChildApprovalResolved {
        parent: AgentSessionId,
        request_id: ApprovalRequestId,
    },
}

pub(crate) struct AgentBridge {
    /// The standalone runtime, which moves the working directory (`/cd`).
    runtime: Arc<AgentRuntime>,
    agent: AgentHandle,
    slot: AgentSlot,
    /// The session the interface drives, once there is one.
    session_id: Option<AgentSessionId>,
    /// The model of the sessions the interface creates, in place of the
    /// model policy's (`-m`).
    model: Option<String>,
    /// The role of the sessions the interface creates (`--role`).
    role: Option<String>,
    /// The name `/clear NAME` gives the session the next message creates.
    pending_title: Option<String>,
    /// While a session is being created: what was asked meanwhile, in
    /// order, carried out once it exists.
    creating: Option<Vec<Op>>,
    app_event_tx: AppEventSender,
    turn: Option<AgentTurnAccepted>,
    /// A turn was submitted and has not ended; `turn` is unknown until the
    /// submission is accepted.
    turn_running: bool,
    /// The transcript items a reopened session replays, and each earlier
    /// page holds.
    page_size: usize,
    /// Dropped to cancel the MCP login waiting for the browser.
    mcp_login: Option<oneshot::Sender<()>>,
    status: status::StatusCache,
    /// The review that runs, if one does.
    review: Option<review::RunningReview>,
    /// The latest denials of automatic review, for `/approve`.
    denials: permissions::RecentDenials,
    /// The worktree the next session the interface creates is bound to
    /// (`/worktree`'s new conversation).
    pending_worktree: Option<PathBuf>,
    /// The watch over the session's subagents while they run.
    watch: Option<subagents::Watch>,
    /// The subagents' approvals shown as the session's, by request.
    child_approvals: HashMap<ApprovalRequestId, subagents::ChildTarget>,
    /// The journal of the subagent this bridge shows, followed.
    following: Option<subagents::Following>,
}

impl AgentBridge {
    pub(crate) fn new(
        runtime: Arc<AgentRuntime>,
        agent: AgentHandle,
        model: Option<String>,
        role: Option<String>,
        app_event_tx: AppEventSender,
    ) -> Self {
        Self {
            slot: runtime.slot().clone(),
            runtime,
            agent,
            session_id: None,
            model,
            role,
            pending_title: None,
            creating: None,
            app_event_tx,
            turn: None,
            turn_running: false,
            page_size: commands::REPLAYED_ITEMS,
            mcp_login: None,
            status: status::StatusCache::default(),
            review: None,
            denials: permissions::RecentDenials::default(),
            pending_worktree: None,
            watch: None,
            child_approvals: HashMap::new(),
            following: None,
        }
    }

    pub(crate) fn session_id(&self) -> Option<AgentSessionId> {
        self.session_id
    }

    #[cfg(test)]
    pub(crate) fn set_page_size(&mut self, page_size: usize) {
        self.page_size = page_size;
    }

    /// Carries out what the interface asked; what it leads to arrives as
    /// events and replies.
    pub(crate) fn submit(&mut self, op: Op) -> Option<TurnUpdate> {
        if let Some(waiting) = self.creating.as_mut() {
            waiting.push(op);
            return None;
        }
        match op {
            Op::UserInput(text) => match (self.session_id, self.turn_running) {
                (None, _) => {
                    self.create_session(vec![Op::UserInput(text)]);
                    None
                }
                (Some(session_id), false) => {
                    self.start_turn(session_id, text);
                    None
                }
                // A subagent's turn the chat has not heard of yet.
                (Some(_), true) => self.steer(None, text),
            },
            Op::Steer { id, text } => self.steer(Some(id), text),
            Op::EnsureSession => {
                if self.session_id.is_none() {
                    self.create_session(Vec::new());
                }
                None
            }
            // No turn runs while a review does.
            Op::Interrupt if self.review_running() => self.review_call(ReviewCall::Cancel),
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
                if let Some(answered) = self.resolve_child_approval(request_id, decision) {
                    return answered;
                }
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
            Op::Command(command) => self.command(command),
        }
    }

    pub(crate) fn reply(&mut self, reply: AgentReply) -> Option<TurnUpdate> {
        match reply {
            AgentReply::TurnAccepted(turn) => {
                self.turn = Some(turn);
                None
            }
            AgentReply::TurnRejected(message) => {
                self.turn_running = false;
                Some(TurnUpdate::Ended(TurnEnd::Failed(message)))
            }
            AgentReply::Failed(message) => Some(TurnUpdate::Error(message)),
            AgentReply::SessionCreated(session_id) => {
                self.session_id = Some(session_id);
                self.load_session_status();
                for op in self.creating.take().unwrap_or_default() {
                    if let Some(update) = self.submit(op) {
                        self.app_event_tx
                            .send(AppEvent::AgentReply(AgentReply::Update(Box::new(update))));
                    }
                }
                None
            }
            // A message waiting for the session started no turn.
            AgentReply::SessionNotCreated(message) => match self.creating.take() {
                Some(waiting) if waiting.iter().any(|op| matches!(op, Op::UserInput(_))) => {
                    Some(TurnUpdate::Ended(TurnEnd::Failed(message)))
                }
                _ => Some(TurnUpdate::Error(message)),
            },
            AgentReply::SessionOpened(opened) => {
                if opened.session_id.is_some() {
                    self.pending_worktree = None;
                }
                self.session_id = opened.session_id;
                self.watch = None;
                self.child_approvals.clear();
                self.turn = None;
                self.turn_running = false;
                self.load_session_status();
                Some(TurnUpdate::SessionOpened(opened))
            }
            AgentReply::Update(update) => match *update {
                // The interface has moved to another session since.
                TurnUpdate::SessionStatus(status) if Some(status.session_id) != self.session_id => {
                    None
                }
                TurnUpdate::ModelChanged(model) => {
                    self.send_session_status();
                    Some(TurnUpdate::ModelChanged(model))
                }
                TurnUpdate::RoleChanged(role) => {
                    self.send_session_status();
                    Some(TurnUpdate::RoleChanged(role))
                }
                TurnUpdate::Goal(reply) => self.goal_reply(reply),
                TurnUpdate::Subagents(SubagentsReply::Listed(statuses)) => {
                    self.subagents_listed(&statuses);
                    Some(TurnUpdate::Subagents(SubagentsReply::Listed(statuses)))
                }
                // `/cd` moved: the status line's branch is the new root's.
                TurnUpdate::Session(reply) if reply.working_directory().is_some() => {
                    self.send_session_status();
                    Some(TurnUpdate::Session(reply))
                }
                update => Some(update),
            },
            AgentReply::StatusLoaded {
                session_id,
                permission_mode,
                goal,
            } => self.status_loaded(session_id, permission_mode, goal),
            AgentReply::ReviewEnded(end) => self.review_ended(end),
            // The interface has moved to another session since.
            AgentReply::ToolDisplayed { session_id, .. } if Some(session_id) != self.session_id => {
                None
            }
            AgentReply::ToolDisplayed {
                call_id, display, ..
            } => Some(TurnUpdate::ToolDisplayed { call_id, display }),
            AgentReply::EnteredWorktree(entered) => self.entered_worktree(entered),
            AgentReply::ChildApproval(approval) => self.child_approval(*approval),
            AgentReply::ChildApprovalResolved { parent, request_id } => {
                self.child_approval_resolved(parent, request_id)
            }
        }
    }

    /// What the chat widget makes of one of the Agent's events, if anything.
    pub(crate) fn event(&mut self, event: AgentEvent) -> Option<TurnUpdate> {
        if Some(event.session_id) != self.session_id {
            return None;
        }
        self.denials.observe(event.session_id, &event.kind);
        self.adopt_followed_turn(&event.kind, event.turn_id);
        // The turn's events can arrive before the reply that accepted it.
        if let (None, Some(turn_id), true) = (self.turn, event.turn_id, self.turn_running) {
            self.turn = Some(AgentTurnAccepted {
                session_id: event.session_id,
                turn_id,
                generation: event.generation,
            });
        }
        let update = match event.kind {
            AgentEventKind::TurnStarted { .. } => TurnUpdate::Started,
            AgentEventKind::ReasoningStarted { reasoning_id, .. } => {
                TurnUpdate::ReasoningStarted(reasoning_id)
            }
            AgentEventKind::ReasoningDelta {
                reasoning_id,
                delta,
            } => TurnUpdate::ReasoningDelta {
                reasoning_id,
                delta,
            },
            AgentEventKind::ReasoningCompleted {
                reasoning_id,
                duration_millis,
            } => TurnUpdate::ReasoningCompleted {
                reasoning_id,
                duration_millis,
            },
            AgentEventKind::ReasoningUpdated { reasoning } => TurnUpdate::Reasoning(reasoning),
            AgentEventKind::AssistantTextDelta { delta, .. } => TurnUpdate::MessageDelta(delta),
            AgentEventKind::AssistantMessageCompleted { .. } => TurnUpdate::MessageCompleted,
            AgentEventKind::ProposedPlanDelta { delta, .. } => TurnUpdate::PlanDelta(delta),
            AgentEventKind::ProposedPlanCompleted { .. } => TurnUpdate::PlanCompleted,
            AgentEventKind::ToolStarted {
                call_id,
                name,
                presentation,
            } => {
                self.observe_tool(&name);
                TurnUpdate::ToolStarted {
                    call_id,
                    name,
                    subject: presentation.subject,
                }
            }
            AgentEventKind::ToolCompleted { call_id, is_error } => {
                self.read_tool_display(event.session_id, call_id.clone());
                TurnUpdate::ToolCompleted { call_id, is_error }
            }
            // Only output names its call; a process the Agent reports itself
            // has none.
            AgentEventKind::ExecutionProgress {
                call_id: Some(call_id),
                update:
                    AgentToolProgressUpdate::ExecutionOutput {
                        text, truncated, ..
                    },
            } => TurnUpdate::ToolOutput {
                call_id,
                text,
                truncated,
            },
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
            AgentEventKind::TaskPlanUpdated { plan } => {
                self.send_session_status();
                TurnUpdate::TaskPlan(plan)
            }
            AgentEventKind::SteeringConsumed {
                through_sequence, ..
            } => TurnUpdate::SteeringConsumed(through_sequence.value()),
            // Every install, `/compact`'s included, is shown from its event.
            AgentEventKind::CompactionUpdated { compaction }
                if compaction.status == AgentCompactionStatus::Completed =>
            {
                self.send_session_status();
                TurnUpdate::Compacted
            }
            // `/compact`'s failure is its reply; a cancelled one is no failure.
            AgentEventKind::CompactionUpdated { compaction }
                if compaction.status == AgentCompactionStatus::Failed
                    && compaction.trigger == AgentCompactionTrigger::Automatic =>
            {
                self.send_session_status();
                TurnUpdate::CompactionFailed(compaction.last_error.unwrap_or_default())
            }
            AgentEventKind::ContextCompressionFailed { message } => {
                TurnUpdate::CompressionFailed(message)
            }
            AgentEventKind::ModelContextWindowLearned { tokens, .. } => {
                self.send_session_status();
                TurnUpdate::ContextWindowLearned(tokens)
            }
            // The `compress` call's cell shows the fold, and the commit's
            // `HistoryCommitted` already refreshed the status line.
            AgentEventKind::ContextCompressed { .. } => return None,
            AgentEventKind::HistoryCommitted { .. }
            | AgentEventKind::SessionTitleUpdated { .. }
            | AgentEventKind::CompactionUpdated { .. } => return self.session_status(),
            AgentEventKind::SessionGoalUpdated { goal } => return self.goal_updated(goal),
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

    /// When the Agent stopped (a kernel fault) during a turn or a review,
    /// their events and replies stop with it: they are told they failed.
    pub(crate) fn status_changed(&mut self) -> Option<TurnUpdate> {
        let status = self.slot.status();
        let running = matches!(status.availability, AgentAvailability::Running)
            && status.generation == self.agent.generation();
        if running || !(self.turn_running || self.review_running()) {
            return None;
        }
        let message = match status.availability {
            AgentAvailability::Faulted { reason, .. } => {
                format!("the Agent stopped after an internal error: {reason}")
            }
            _ => "the Agent stopped".to_string(),
        };
        if self.review_running() {
            return self.review_ended(ReviewEnd::Failed(message));
        }
        Some(self.ended(TurnEnd::Failed(message)))
    }

    fn ended(&mut self, end: TurnEnd) -> TurnUpdate {
        self.turn = None;
        self.turn_running = false;
        self.send_session_status();
        self.reshow_child_approvals();
        TurnUpdate::Ended(end)
    }

    fn start_turn(&mut self, session_id: AgentSessionId, text: String) {
        self.turn_running = true;
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

    /// Where a call's events go. A subagent this bridge follows has every
    /// event of its own from its journal, so a call's copies, which would
    /// show (and end) its turn twice, are dropped.
    fn sink(&self) -> Arc<dyn AgentEventSink> {
        match self.following {
            Some(_) => Arc::new(DiscardSink),
            None => Arc::new(EventSink(self.app_event_tx.clone())),
        }
    }
}

/// The Agent's events into the app loop: `emit` must not block.
struct EventSink(AppEventSender);

impl AgentEventSink for EventSink {
    fn emit(&self, event: AgentEvent) {
        self.0.send(AppEvent::Agent(Box::new(event)));
    }
}

struct DiscardSink;

impl AgentEventSink for DiscardSink {
    fn emit(&self, _event: AgentEvent) {}
}

/// The Agent slot's changes into the app loop.
pub(crate) struct StatusSink(pub(crate) AppEventSender);

impl AgentStatusEventSink for StatusSink {
    fn status_changed(&self) {
        self.0.send(AppEvent::AgentStatusChanged);
    }
}
