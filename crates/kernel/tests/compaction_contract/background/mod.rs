//! The `background` strategy (Codex's async compaction): the launch and its
//! gates, adoption before a turn and between its steps, the wait of a
//! required compaction, the rebasing install, usage, and the session's
//! lifecycle around a running compaction. The fake model answers the turn's
//! requests and the compactions from two queues, and holds a compaction
//! behind a gate the test opens, so nothing depends on timing.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use hanafubuki_agent_kernel::{
    AgentBackgroundCompactionState, AgentBackgroundCompactionThreshold, AgentCompactionPhase,
    AgentCompactionStrategy, AgentEvent, AgentEventSink, AgentSessionId, AgentTurnPhase,
    AgentWorldStateSection, AgentWorldStateSnapshot, EphemeralAgentHistoryStore,
};

use super::*;

/// The context window, and the two thresholds over it: the background
/// compaction starts at 5 000 tokens, the foreground one at 5 904 (80% would
/// be 8 000, but a window this small keeps 4 096 tokens for a reply).
const WINDOW: u64 = 10_000;
const BACKGROUND_PERCENT: u8 = 50;
const IN_BAND: u64 = 5_500;
const OVER_THRESHOLD: u64 = 9_000;
/// How long a test waits for an outcome before it fails.
const OUTCOME_DEADLINE: Duration = Duration::from_secs(60);

/// Held closed until the test opens it.
#[derive(Clone, Default)]
struct Gate(Arc<AtomicBool>);

impl Gate {
    fn open(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// Whether it opened before `cancellation` fired.
    async fn wait(&self, cancellation: &CancellationToken) -> bool {
        loop {
            if self.0.load(Ordering::SeqCst) {
                return true;
            }
            if cancellation.is_cancelled() {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }
}

/// What the fake answers one compaction request with.
enum Answer {
    /// A local summary.
    Summary(&'static str),
    /// A v2 compaction item.
    Payload(&'static str),
    /// A v1 replacement history.
    Replacement(Vec<ModelInputItem>),
    Fail,
}

struct ScriptedCompaction {
    gate: Option<Gate>,
    /// Answers even when cancelled while held, as a response that
    /// completes at the moment of the cancellation does.
    answer_when_cancelled: bool,
    answer: Answer,
}

fn compaction(answer: Answer) -> ScriptedCompaction {
    ScriptedCompaction {
        gate: None,
        answer_when_cancelled: false,
        answer,
    }
}

fn held(gate: &Gate, answer: Answer) -> ScriptedCompaction {
    ScriptedCompaction {
        gate: Some(gate.clone()),
        answer_when_cancelled: false,
        answer,
    }
}

/// The usage every compaction answer reports.
const COMPACTION_USAGE: ModelUsage = ModelUsage {
    input_tokens: 700,
    output_tokens: 30,
};

struct ScriptedTurnResponse {
    gate: Option<Gate>,
    response: ScriptedResponse,
}

/// A model that tells the turn's requests from its compactions: a local
/// summary may call no tool, a v2 compaction ends with its trigger, a v1
/// one is `compact`.
#[derive(Clone)]
struct BackgroundModel {
    turns: Arc<Mutex<VecDeque<ScriptedTurnResponse>>>,
    compactions: Arc<Mutex<VecDeque<ScriptedCompaction>>>,
    requests: Arc<Mutex<Vec<ModelRequest>>>,
    compaction_requests: Arc<Mutex<Vec<ModelRequest>>>,
    capability: ModelCompactionCapability,
}

impl BackgroundModel {
    fn new(
        capability: ModelCompactionCapability,
        turns: impl IntoIterator<Item = ScriptedTurnResponse>,
        compactions: impl IntoIterator<Item = ScriptedCompaction>,
    ) -> Self {
        Self {
            turns: Arc::new(Mutex::new(turns.into_iter().collect())),
            compactions: Arc::new(Mutex::new(compactions.into_iter().collect())),
            requests: Arc::new(Mutex::new(Vec::new())),
            compaction_requests: Arc::new(Mutex::new(Vec::new())),
            capability,
        }
    }

    fn local(
        turns: impl IntoIterator<Item = ScriptedTurnResponse>,
        compactions: impl IntoIterator<Item = ScriptedCompaction>,
    ) -> Self {
        Self::new(ModelCompactionCapability::LocalSummary, turns, compactions)
    }

    fn requests(&self) -> Vec<ModelRequest> {
        self.requests.lock().unwrap().clone()
    }

    fn compaction_requests(&self) -> Vec<ModelRequest> {
        self.compaction_requests.lock().unwrap().clone()
    }

    fn next_compaction(&self, request: ModelRequest) -> Result<ScriptedCompaction, ModelError> {
        self.compaction_requests.lock().unwrap().push(request);
        self.compactions
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| ModelError::Protocol("compaction fixture exhausted".to_string()))
    }
}

/// Holds `scripted` behind its gate; `Err` when it is cancelled there.
async fn release(
    scripted: &ScriptedCompaction,
    cancellation: &CancellationToken,
) -> Result<(), ModelError> {
    match &scripted.gate {
        Some(gate) if !gate.wait(cancellation).await && !scripted.answer_when_cancelled => {
            Err(ModelError::Cancelled)
        }
        _ => Ok(()),
    }
}

impl ModelPort for BackgroundModel {
    fn compaction_capability(&self) -> ModelCompactionCapability {
        self.capability
    }

    fn stream(
        &self,
        request: ModelRequest,
        cancellation: CancellationToken,
    ) -> ModelFuture<'_, ModelResult<ModelStream>> {
        let compacting = request.tool_choice == hanafubuki_agent_model_api::ModelToolChoice::None
            || matches!(
                request.input.last(),
                Some(ModelInputItem::CompactionTrigger)
            );
        if compacting {
            let scripted = self.next_compaction(request);
            return Box::pin(async move {
                let scripted = scripted?;
                release(&scripted, &cancellation).await?;
                let events = match scripted.answer {
                    Answer::Summary(text) => summary_events(text),
                    Answer::Payload(payload) => payload_events(payload),
                    Answer::Replacement(_) | Answer::Fail => {
                        return Err(ModelError::Protocol(
                            "background fixture failed".to_string(),
                        ))
                    }
                };
                Ok(Box::new(ScriptedStream {
                    events: events.into(),
                }) as ModelStream)
            });
        }
        self.requests.lock().unwrap().push(request);
        let scripted = self.turns.lock().unwrap().pop_front();
        Box::pin(async move {
            let scripted =
                scripted.ok_or_else(|| ModelError::Protocol("fixture exhausted".to_string()))?;
            if let Some(gate) = &scripted.gate {
                if !gate.wait(&cancellation).await {
                    return Err(ModelError::Cancelled);
                }
            }
            match scripted.response {
                ScriptedResponse::Events(events) => Ok(Box::new(ScriptedStream {
                    events: events.into(),
                }) as ModelStream),
                ScriptedResponse::WaitForCancellation => {
                    Ok(Box::new(CancellationStream { cancellation }) as ModelStream)
                }
            }
        })
    }

    fn compact(
        &self,
        request: ModelRequest,
        cancellation: CancellationToken,
    ) -> ModelFuture<'_, ModelResult<ModelCompactionResult>> {
        let scripted = self.next_compaction(request);
        Box::pin(async move {
            let scripted = scripted?;
            release(&scripted, &cancellation).await?;
            match scripted.answer {
                Answer::Replacement(replacement_history) => Ok(ModelCompactionResult {
                    replacement_history,
                    usage: Some(COMPACTION_USAGE),
                }),
                _ => Err(ModelError::Protocol(
                    "background fixture failed".to_string(),
                )),
            }
        })
    }
}

fn summary_events(text: &str) -> Vec<ModelResult<ModelEvent>> {
    let item = ModelOutputItem::AssistantMessage {
        item_id: "summary".to_string(),
        phase: Some(ModelMessagePhase::FinalAnswer),
        text: text.to_string(),
    };
    vec![
        Ok(ModelEvent::ResponseStarted {
            response_id: "summary".to_string(),
            metadata: ModelResponseMetadata::default(),
        }),
        Ok(ModelEvent::OutputItemCompleted { item }),
        Ok(ModelEvent::ResponseCompleted {
            response_id: "summary".to_string(),
            usage: Some(COMPACTION_USAGE),
            end_turn: Some(true),
        }),
    ]
}

fn payload_events(payload: &str) -> Vec<ModelResult<ModelEvent>> {
    vec![
        Ok(ModelEvent::ResponseStarted {
            response_id: "v2".to_string(),
            metadata: ModelResponseMetadata::default(),
        }),
        Ok(ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::Compaction {
                item_id: "v2-item".to_string(),
                payload: ModelOpaqueCompactionPayload::parse(payload.to_string()).unwrap(),
            },
        }),
        Ok(ModelEvent::ResponseCompleted {
            response_id: "v2".to_string(),
            usage: Some(COMPACTION_USAGE),
            end_turn: Some(true),
        }),
    ]
}

/// A final answer whose request the provider counted as `input_tokens`.
fn answer(text: &str, input_tokens: u64) -> ScriptedTurnResponse {
    ScriptedTurnResponse {
        gate: None,
        response: response("answer", text, input_tokens),
    }
}

fn gated(gate: &Gate, response: ScriptedResponse) -> ScriptedTurnResponse {
    ScriptedTurnResponse {
        gate: Some(gate.clone()),
        response,
    }
}

fn tool_call(input_tokens: u64) -> ScriptedTurnResponse {
    ScriptedTurnResponse {
        gate: None,
        response: tool_response_with_usage(input_tokens),
    }
}

async fn list_tools() -> FakeTools {
    FakeTools::new([FakeRegisteredExchange::success(
        "list_instances",
        json!({}),
        json!({"instances": []}),
    )])
    .await
}

fn background_config(capability: ModelCompactionCapability) -> AgentSessionConfig {
    AgentSessionConfig {
        context_window_tokens: Some(WINDOW),
        auto_compact_threshold_percent: 80,
        compaction_capability: capability,
        compaction_strategy: AgentCompactionStrategy::Background,
        ..config()
    }
}

/// A turn as the facade admits one: with the background threshold.
fn turn(text: &str) -> AgentTurnInput {
    let mut input = AgentTurnInput::from(text);
    input.turn = TurnExtensions::default().with(Arc::new(AgentBackgroundCompactionThreshold {
        percent: BACKGROUND_PERCENT,
    }));
    input
}

fn turn_at(text: &str, location: &str) -> AgentTurnInput {
    let mut input = turn(text);
    input.world_state = AgentWorldStateSnapshot {
        sections: vec![AgentWorldStateSection::new(
            hanafubuki_agent_kernel::ContextSectionKind::fixed("host_location"),
            location.to_string(),
        )],
    };
    input
}

/// Waits until `ready` holds, failing the test past a generous deadline.
async fn eventually(what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + OUTCOME_DEADLINE;
    while !ready() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

/// Waits until the background compaction sent its request, so a compaction
/// sent later takes the fake's next answer.
async fn background_sent(model: &BackgroundModel) {
    eventually("the background compaction's request", || {
        model.compaction_requests().len() == 1
    })
    .await;
}

fn background_state(
    service: &AgentService,
    session: AgentSessionId,
) -> Option<AgentBackgroundCompactionState> {
    service.snapshot(session).unwrap().background_compaction
}

async fn background_ready(service: &AgentService, session: AgentSessionId) {
    eventually("a ready background compaction", || {
        background_state(service, session) == Some(AgentBackgroundCompactionState::Ready)
    })
    .await;
}

/// Two turns, the second reported in the background band: its end
/// launches a compaction of the first.
async fn seed(service: &AgentService, session: AgentSessionId) {
    service.start_turn(session, turn("first")).await.unwrap();
    service.start_turn(session, turn("second")).await.unwrap();
}

/// The answers of [`seed`], then `rest`. The first answer is long enough
/// that compacting it reduces the context.
fn seed_then(rest: impl IntoIterator<Item = ScriptedTurnResponse>) -> Vec<ScriptedTurnResponse> {
    [answer(&FIRST_ANSWER, 100), answer("second answer", IN_BAND)]
        .into_iter()
        .chain(rest)
        .collect()
}

static FIRST_ANSWER: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| "a long first answer ".repeat(200));

/// The model context's items, by kind and text, in order.
fn layout(context: &[ConversationItem]) -> Vec<String> {
    context
        .iter()
        .map(|item| match item {
            ConversationItem::UserMessage { text, .. } => format!("user:{text}"),
            ConversationItem::AssistantMessage { text, .. } => format!("assistant:{text}"),
            ConversationItem::CompactionSummary { summary, .. } => {
                format!("summary:{}", summary.rsplit("\n\n").next().unwrap())
            }
            ConversationItem::ProviderCompaction { payload } => {
                format!("payload:{}", payload.as_str())
            }
            ConversationItem::TurnStart { .. } => "turn".to_string(),
            ConversationItem::WorldStateUpdate { .. } => "world".to_string(),
            ConversationItem::ToolCall { .. } => "tool_call".to_string(),
            ConversationItem::ToolOutput { .. } => "tool_output".to_string(),
            other => format!("{other:?}"),
        })
        .collect()
}

fn background_completed(service: &AgentService, session: AgentSessionId) -> bool {
    service
        .snapshot(session)
        .unwrap()
        .compaction
        .is_some_and(|compaction| {
            compaction.phase == AgentCompactionPhase::Background
                && compaction.trigger == AgentCompactionTrigger::Automatic
                && compaction.status == AgentCompactionStatus::Completed
        })
}

#[derive(Default)]
struct RecordingSink(Mutex<Vec<AgentEvent>>);

impl AgentEventSink for RecordingSink {
    fn emit(&self, event: AgentEvent) {
        self.0.lock().unwrap().push(event);
    }
}

impl RecordingSink {
    fn compaction_phases(&self) -> Vec<AgentCompactionPhase> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match &event.kind {
                AgentEventKind::CompactionUpdated { compaction } => Some(compaction.phase),
                _ => None,
            })
            .collect()
    }
}

mod adoption;
mod launch;
mod lifecycle;
mod wait;
