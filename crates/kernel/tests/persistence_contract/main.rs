use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Barrier, Mutex};
use std::thread;
use std::time::Duration;

use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_kernel::test_support::FakeTools;
use hanafubuki_agent_kernel::{
    AgentActivityAction, AgentConversationCommit, AgentEvent, AgentEventKind, AgentEventSink,
    AgentExecutionProgressStatus, AgentHistoryAppendAck, AgentHistoryBase, AgentHistoryItem,
    AgentHistoryMutation, AgentHistoryStore, AgentHistoryStoreError, AgentKernelRegistries,
    AgentRecoveredSession, AgentRevision, AgentService, AgentSessionConfig, AgentSessionId,
    AgentSessionListRequest, AgentSessionModelConfigurationUpdate, AgentSessionOpenResult,
    AgentSessionPage, AgentToolProgressUpdate, AgentTranscriptPage, AgentTranscriptPageRequest,
    AgentTurnId, AgentTurnInput, AgentTurnPageRequest, ConversationItem,
    EphemeralAgentHistoryStore, ModelCompactionCapability,
};
use hanafubuki_agent_model_api::{
    ModelError, ModelEvent, ModelEventStream, ModelFuture, ModelMessagePhase, ModelOutputItem,
    ModelPort, ModelRequest, ModelResponseMetadata, ModelResult, ModelStream, ModelToolCall,
};
use serde_json::json;

struct RequestRetriesExhaustedModel;

struct ResponseStartedThenErrorModel;

struct ResponseStartedThenErrorStream {
    events: VecDeque<ModelResult<ModelEvent>>,
}

struct ScriptedPersistenceModel {
    responses: Mutex<VecDeque<Vec<ModelResult<ModelEvent>>>>,
    requests: AtomicUsize,
}

struct ScriptedPersistenceStream {
    events: VecDeque<ModelResult<ModelEvent>>,
}

impl ModelEventStream for ScriptedPersistenceStream {
    fn next(&mut self) -> ModelFuture<'_, Option<ModelResult<ModelEvent>>> {
        let event = self.events.pop_front();
        Box::pin(async move { event })
    }
}

impl ModelPort for ScriptedPersistenceModel {
    fn stream(
        &self,
        _request: ModelRequest,
        _cancellation: CancellationToken,
    ) -> ModelFuture<'_, ModelResult<ModelStream>> {
        self.requests.fetch_add(1, Ordering::SeqCst);
        let events = self.responses.lock().unwrap().pop_front();
        Box::pin(async move {
            let events = events.ok_or_else(|| {
                ModelError::Protocol("scripted persistence response exhausted".to_string())
            })?;
            Ok(Box::new(ScriptedPersistenceStream {
                events: events.into(),
            }) as ModelStream)
        })
    }
}

impl ModelEventStream for ResponseStartedThenErrorStream {
    fn next(&mut self) -> ModelFuture<'_, Option<ModelResult<ModelEvent>>> {
        let event = self.events.pop_front();
        Box::pin(async move { event })
    }
}

impl ModelPort for ResponseStartedThenErrorModel {
    fn stream(
        &self,
        _request: ModelRequest,
        _cancellation: CancellationToken,
    ) -> ModelFuture<'_, ModelResult<ModelStream>> {
        Box::pin(async {
            Ok(Box::new(ResponseStartedThenErrorStream {
                events: VecDeque::from([
                    Ok(ModelEvent::ResponseStarted {
                        response_id: "transient-response".to_string(),
                        metadata: ModelResponseMetadata::default(),
                    }),
                    Err(ModelError::Protocol(
                        "fixture stopped after start".to_string(),
                    )),
                ]),
            }) as ModelStream)
        })
    }
}

struct OpenSessionOnResponseStartedSink {
    service: AgentService,
    session_id: AgentSessionId,
    opened: Mutex<Option<Result<AgentSessionOpenResult, hanafubuki_agent_kernel::AgentError>>>,
}

impl AgentEventSink for OpenSessionOnResponseStartedSink {
    fn emit(&self, event: AgentEvent) {
        if matches!(event.kind, AgentEventKind::ResponseStarted { .. }) {
            *self.opened.lock().unwrap() = Some(self.service.open_session(self.session_id, 75));
        }
    }
}

/// Reports `update` as a tool call of `session_id` would, and waits until
/// it is applied.
async fn report_progress(
    service: &AgentService,
    session_id: AgentSessionId,
    update: AgentToolProgressUpdate,
) {
    service.execution_progress_sink(session_id).report(update);
    service
        .flush_session_execution_progress(session_id)
        .await
        .unwrap();
}

/// Waits, off the runtime, at one of `gate`'s barriers.
async fn meet(
    barrier: impl FnOnce(&AppendGate) -> &Barrier + Send + 'static,
    gate: &Arc<AppendGate>,
) {
    let gate = gate.clone();
    tokio::task::spawn_blocking(move || {
        barrier(&gate).wait();
    })
    .await
    .unwrap();
}

/// One response with the final answer `text`.
fn final_answer_events(text: &str) -> VecDeque<ModelResult<ModelEvent>> {
    VecDeque::from([
        Ok(ModelEvent::ResponseStarted {
            response_id: "answer".to_string(),
            metadata: ModelResponseMetadata::default(),
        }),
        Ok(ModelEvent::OutputItemAdded {
            item: ModelOutputItem::AssistantMessage {
                item_id: "answer-item".to_string(),
                phase: Some(ModelMessagePhase::FinalAnswer),
                text: String::new(),
            },
        }),
        Ok(ModelEvent::AssistantTextDelta {
            item_id: "answer-item".to_string(),
            delta: text.to_string(),
        }),
        Ok(ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::AssistantMessage {
                item_id: "answer-item".to_string(),
                phase: Some(ModelMessagePhase::FinalAnswer),
                text: text.to_string(),
            },
        }),
        Ok(ModelEvent::ResponseCompleted {
            response_id: "answer".to_string(),
            usage: None,
            end_turn: Some(true),
        }),
    ])
}

/// Runs `test` on a single-threaded runtime of its own thread, so a session
/// left waiting fails the test instead of hanging it.
fn on_a_current_thread_runtime<T, F>(test: impl FnOnce() -> F + Send + 'static) -> T
where
    T: Send + 'static,
    F: std::future::Future<Output = T>,
{
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _ = sender.send(runtime.block_on(test()));
    });
    receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("the test's runtime was left waiting")
}

/// Yields to the runtime until `ready` holds; `false` after two seconds.
async fn until(ready: impl Fn() -> bool) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while !ready() {
        if std::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    true
}

fn text_session_config() -> AgentSessionConfig {
    AgentSessionConfig {
        provider_id: "provider-1".to_string(),
        provider_protocol: hanafubuki_agent_model_api::ModelApiProtocol::Responses,
        model: "text-only".to_string(),
        instructions: "test".to_string(),
        mode: hanafubuki_agent_kernel::AgentSessionMode::Default,
        reasoning_effort: None,
        context_window_tokens: None,
        input_capabilities: hanafubuki_agent_model_api::ModelInputCapabilityPolicy {
            image_input: hanafubuki_agent_model_api::ModelImageInputPolicy::Disabled,
        },
        auto_compaction_enabled: true,
        auto_compact_threshold_percent: 80,
        compaction_capability: ModelCompactionCapability::LocalSummary,
        sampling_retry_policy: hanafubuki_agent_kernel::AgentSamplingRetryPolicy::default(),
        compaction_strategy: Default::default(),
    }
}

impl ModelPort for RequestRetriesExhaustedModel {
    fn stream(
        &self,
        _request: ModelRequest,
        _cancellation: CancellationToken,
    ) -> ModelFuture<'_, ModelResult<ModelStream>> {
        Box::pin(async {
            Err(ModelError::RequestRetriesExhausted {
                attempts: 3,
                status: Some(503),
                code: Some("overloaded".to_string()),
                message: "fixture exhausted".to_string(),
                request_id: Some("request-fixture".to_string()),
            })
        })
    }
}

#[derive(Default)]
struct RecordingHistoryStore {
    inner: Arc<EphemeralAgentHistoryStore>,
    commits: Arc<Mutex<Vec<AgentConversationCommit>>>,
    deltas: Mutex<Vec<hanafubuki_agent_kernel::AgentConversationDelta>>,
    fail_next_append: AtomicBool,
    fail_permission_review_completion: AtomicBool,
    append_gate: Mutex<Option<Arc<AppendGate>>>,
    /// Holds the acknowledgement of the next queued delta until the gate
    /// opens; the append itself is made at once, as by a writer.
    delta_gate: Mutex<Option<Arc<AppendGate>>>,
    /// `delta_gate` for the next queued commit.
    commit_gate: Mutex<Option<Arc<AppendGate>>>,
    /// Holds the next queued attachment load until the gate opens, on a
    /// thread of its own, as a store's worker would.
    attachment_gate: Mutex<Option<Arc<AppendGate>>>,
    /// The attachments of each queued load, in order.
    attachment_loads: Mutex<Vec<Vec<hanafubuki_agent_kernel::AgentImageAttachmentRef>>>,
    /// Fails the next queued delta.
    fail_next_queued_delta: AtomicBool,
    /// Drops the next queued delta with its acknowledgement unanswered, as
    /// a writer that stopped would.
    drop_next_queued_delta: AtomicBool,
    load_model_context_calls: AtomicUsize,
}

impl RecordingHistoryStore {
    /// Acknowledges a queued append, once `gate` opens when there is one,
    /// and appends the follow-ups the acknowledgement hands back.
    fn acknowledge_queued(
        &self,
        gate: Option<Arc<AppendGate>>,
        ack: AgentHistoryAppendAck,
        appended: Result<(), AgentHistoryStoreError>,
    ) {
        let (inner, commits) = (self.inner.clone(), self.commits.clone());
        let follow_up = move |commit: AgentConversationCommit| {
            commits.lock().unwrap().push(commit.clone());
            inner.append_commit(commit)
        };
        match gate {
            Some(gate) => {
                thread::spawn(move || {
                    gate.started.wait();
                    gate.release.wait();
                    ack.acknowledge(appended, follow_up);
                });
            }
            None => ack.acknowledge(appended, follow_up),
        }
    }
}

struct AppendGate {
    started: Barrier,
    release: Barrier,
}

impl AppendGate {
    fn new() -> Self {
        Self {
            started: Barrier::new(2),
            release: Barrier::new(2),
        }
    }
}

impl AgentHistoryStore for RecordingHistoryStore {
    fn allocate_session_id(&self) -> Result<AgentSessionId, AgentHistoryStoreError> {
        self.inner.allocate_session_id()
    }

    fn create_session(
        &self,
        commit: AgentConversationCommit,
    ) -> Result<(), AgentHistoryStoreError> {
        self.commits.lock().unwrap().push(commit.clone());
        self.inner.create_session(commit)
    }

    fn append_commit(&self, commit: AgentConversationCommit) -> Result<(), AgentHistoryStoreError> {
        if let Some(gate) = self.append_gate.lock().unwrap().take() {
            gate.started.wait();
            gate.release.wait();
        }
        if self.fail_next_append.swap(false, Ordering::SeqCst) {
            return Err(AgentHistoryStoreError::Io(
                "injected durable append failure".to_string(),
            ));
        }
        let completes_permission_review = commit.history_mutations.iter().any(|mutation| {
            matches!(
                mutation,
                AgentHistoryMutation::UpsertItemSnapshot {
                    item: AgentHistoryItem {
                        content: ConversationItem::PermissionReview {
                            status: hanafubuki_agent_kernel::PermissionReviewStatus::Approved,
                            ..
                        },
                        ..
                    }
                }
            )
        });
        if completes_permission_review
            && self
                .fail_permission_review_completion
                .swap(false, Ordering::SeqCst)
        {
            return Err(AgentHistoryStoreError::Io(
                "injected permission review completion failure".to_string(),
            ));
        }
        self.commits.lock().unwrap().push(commit.clone());
        self.inner.append_commit(commit)
    }

    fn append_delta(
        &self,
        delta: hanafubuki_agent_kernel::AgentConversationDelta,
    ) -> Result<(), AgentHistoryStoreError> {
        if let Some(gate) = self.append_gate.lock().unwrap().take() {
            gate.started.wait();
            gate.release.wait();
        }
        if self.fail_next_append.swap(false, Ordering::SeqCst) {
            return Err(AgentHistoryStoreError::Io(
                "injected durable append failure".to_string(),
            ));
        }
        self.deltas.lock().unwrap().push(delta.clone());
        self.inner.append_delta(delta)
    }

    fn enqueue_commit(&self, commit: AgentConversationCommit, ack: AgentHistoryAppendAck) {
        let appended = self.append_commit(commit);
        let gate = self.commit_gate.lock().unwrap().take();
        self.acknowledge_queued(gate, ack, appended);
    }

    fn enqueue_delta(
        &self,
        delta: hanafubuki_agent_kernel::AgentConversationDelta,
        ack: AgentHistoryAppendAck,
    ) {
        if self.drop_next_queued_delta.swap(false, Ordering::SeqCst) {
            drop(ack);
            return;
        }
        let appended = if self.fail_next_queued_delta.swap(false, Ordering::SeqCst) {
            Err(AgentHistoryStoreError::Io(
                "injected queued append failure".to_string(),
            ))
        } else {
            self.append_delta(delta)
        };
        let gate = self.delta_gate.lock().unwrap().take();
        self.acknowledge_queued(gate, ack, appended);
    }

    fn stage_image_attachment(
        &self,
        session_id: AgentSessionId,
        image: hanafubuki_agent_kernel::PreparedAgentImage,
    ) -> Result<hanafubuki_agent_kernel::AgentImageAttachmentRef, AgentHistoryStoreError> {
        self.inner.stage_image_attachment(session_id, image)
    }

    fn load_image_attachment(
        &self,
        attachment: &hanafubuki_agent_kernel::AgentImageAttachmentRef,
    ) -> Result<Vec<u8>, AgentHistoryStoreError> {
        self.inner.load_image_attachment(attachment)
    }

    fn enqueue_image_attachment_load(
        &self,
        _session_id: AgentSessionId,
        attachments: Vec<hanafubuki_agent_kernel::AgentImageAttachmentRef>,
        ack: hanafubuki_agent_kernel::AgentImageAttachmentLoadAck,
    ) {
        self.attachment_loads
            .lock()
            .unwrap()
            .push(attachments.clone());
        let inner = self.inner.clone();
        let load = move || {
            ack.acknowledge(
                attachments
                    .iter()
                    .map(|attachment| inner.load_image_attachment(attachment))
                    .collect(),
            );
        };
        match self.attachment_gate.lock().unwrap().take() {
            Some(gate) => {
                thread::spawn(move || {
                    gate.started.wait();
                    gate.release.wait();
                    load();
                });
            }
            None => load(),
        }
    }

    fn create_fork(
        &self,
        source_session_id: AgentSessionId,
        before_turn_id: AgentTurnId,
        commit: AgentConversationCommit,
    ) -> Result<AgentHistoryBase, AgentHistoryStoreError> {
        self.commits.lock().unwrap().push(commit.clone());
        self.inner
            .create_fork(source_session_id, before_turn_id, commit)
    }

    fn flush_session(&self, session_id: AgentSessionId) -> Result<(), AgentHistoryStoreError> {
        self.inner.flush_session(session_id)
    }

    fn shutdown_session(&self, session_id: AgentSessionId) -> Result<(), AgentHistoryStoreError> {
        self.inner.shutdown_session(session_id)
    }

    fn discard_live_session(&self, session_id: AgentSessionId) {
        self.inner.discard_live_session(session_id);
    }

    fn load_recovery_candidates(
        &self,
    ) -> Result<
        Vec<Result<AgentRecoveredSession, hanafubuki_agent_kernel::AgentRecoveryLoadFailure>>,
        AgentHistoryStoreError,
    > {
        self.inner.load_recovery_candidates()
    }

    fn load_runtime_state(
        &self,
        session_id: AgentSessionId,
    ) -> Result<hanafubuki_agent_kernel::PersistedAgentRuntimeState, AgentHistoryStoreError> {
        self.inner.load_runtime_state(session_id)
    }

    fn load_latest_model_context(
        &self,
        session_id: AgentSessionId,
    ) -> Result<Vec<AgentHistoryItem>, AgentHistoryStoreError> {
        self.load_model_context_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.load_latest_model_context(session_id)
    }

    fn list_sessions_page(
        &self,
        request: AgentSessionListRequest,
    ) -> Result<AgentSessionPage, AgentHistoryStoreError> {
        self.inner.list_sessions_page(request)
    }

    fn list_items_page(
        &self,
        request: AgentTranscriptPageRequest,
    ) -> Result<AgentTranscriptPage, AgentHistoryStoreError> {
        if request.through_revision.is_some_and(|through_revision| {
            self.inner
                .load_runtime_state(request.session_id)
                .is_ok_and(|runtime| through_revision.value() > runtime.revision.value())
        }) {
            return Err(AgentHistoryStoreError::InvalidCursor(
                "transcript revision is newer than the session".to_string(),
            ));
        }
        self.inner.list_items_page(request)
    }

    fn list_turn_items_page(
        &self,
        request: AgentTurnPageRequest,
    ) -> Result<AgentTranscriptPage, AgentHistoryStoreError> {
        self.inner.list_turn_items_page(request)
    }

    fn archive_session(
        &self,
        session_id: AgentSessionId,
        expected_revision: AgentRevision,
    ) -> Result<(), AgentHistoryStoreError> {
        self.inner.archive_session(session_id, expected_revision)
    }

    fn restore_session(
        &self,
        session_id: AgentSessionId,
        expected_revision: AgentRevision,
    ) -> Result<(), AgentHistoryStoreError> {
        self.inner.restore_session(session_id, expected_revision)
    }

    fn check_sessions_deletable(
        &self,
        session_ids: &[AgentSessionId],
    ) -> Result<(), AgentHistoryStoreError> {
        self.inner.check_sessions_deletable(session_ids)
    }

    fn delete_sessions_confirmed(
        &self,
        session_ids: &[AgentSessionId],
    ) -> Result<(), AgentHistoryStoreError> {
        self.inner.delete_sessions_confirmed(session_ids)
    }

    fn repair_session_projection(
        &self,
        session_id: AgentSessionId,
    ) -> Result<(), AgentHistoryStoreError> {
        self.inner.repair_session_projection(session_id)
    }

    fn shutdown(&self, deadline: std::time::Instant) -> Vec<AgentHistoryStoreError> {
        self.inner.shutdown(deadline)
    }
}

mod children;
mod commit_rollback;
mod compaction_recovery;
mod compression_chain;
mod creation_deletion;
mod drop_guards;
mod off_worker;
mod paging;
mod progress;
mod recovery;
mod request_images;
mod session_config;
