//! A reasoning item whose provider done comes after the response moved on
//! (the Responses adapter forwards it after text or a tool call may have
//! started): the first other output ends its thinking, and the late done
//! only completes what is kept.

use super::reasoning::{reasoning_blocks, reasoning_items};
use super::*;

use hanafubuki_agent_kernel::{AgentReasoningPhase, AgentReasoningSnapshot};
use hanafubuki_agent_model_api::{ModelReasoningReplay, ModelReasoningTextKind};

/// Long enough that a clock still running through it cannot pass for one
/// that stopped.
const LATE_DONE_DELAY: Duration = Duration::from_millis(300);

const REASONING_ITEM: &str = "reason-late";

/// One step of a scripted response: an event, or a pause before the next.
enum Step {
    Event(ModelResult<ModelEvent>),
    Pause(Duration),
}

#[derive(Clone)]
struct PausingModel {
    responses: Arc<Mutex<VecDeque<Vec<Step>>>>,
}

impl PausingModel {
    fn new(responses: impl IntoIterator<Item = Vec<Step>>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(responses.into_iter().collect())),
        }
    }
}

impl ModelPort for PausingModel {
    fn stream(
        &self,
        _request: ModelRequest,
        _cancellation: CancellationToken,
    ) -> ModelFuture<'_, ModelResult<ModelStream>> {
        let steps = self.responses.lock().unwrap().pop_front();
        Box::pin(async move {
            let steps = steps
                .ok_or_else(|| ModelError::Protocol("scripted response exhausted".to_string()))?;
            Ok(Box::new(PausingStream {
                steps: steps.into(),
            }) as ModelStream)
        })
    }
}

struct PausingStream {
    steps: VecDeque<Step>,
}

impl ModelEventStream for PausingStream {
    fn next(&mut self) -> ModelFuture<'_, Option<ModelResult<ModelEvent>>> {
        Box::pin(async move {
            loop {
                match self.steps.pop_front()? {
                    Step::Event(event) => return Some(event),
                    Step::Pause(duration) => tokio::time::sleep(duration).await,
                }
            }
        })
    }
}

fn steps(events: Vec<ModelResult<ModelEvent>>) -> Vec<Step> {
    events.into_iter().map(Step::Event).collect()
}

fn reasoning_added() -> Step {
    Step::Event(Ok(ModelEvent::OutputItemAdded {
        item: ModelOutputItem::Reasoning {
            item_id: REASONING_ITEM.to_string(),
            summary: Vec::new(),
            content: Vec::new(),
            replay: None,
        },
    }))
}

fn reasoning_delta(index: u32, delta: &str) -> Step {
    Step::Event(Ok(ModelEvent::ReasoningDelta {
        item_id: REASONING_ITEM.to_string(),
        kind: ModelReasoningTextKind::Summary,
        index,
        delta: delta.to_string(),
    }))
}

fn late_replay() -> ModelReasoningReplay {
    ModelReasoningReplay::ReasoningContent {
        text: "the block to send back".to_string(),
    }
}

/// The provider's done, after a pause: its summary goes on past what
/// streamed, and it carries a block to send back.
fn late_reasoning_done() -> [Step; 2] {
    [
        Step::Pause(LATE_DONE_DELAY),
        Step::Event(Ok(ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::Reasoning {
                item_id: REASONING_ITEM.to_string(),
                summary: vec!["Weighing it.".to_string(), "Late part.".to_string()],
                content: Vec::new(),
                replay: Some(late_replay()),
            },
        })),
    ]
}

/// `events` with the reasoning item started and streaming first, and its
/// late done after the event at `done_after`.
fn with_late_reasoning(events: Vec<ModelResult<ModelEvent>>, done_after: usize) -> Vec<Step> {
    let mut steps = steps(events);
    steps.splice(done_after + 1..done_after + 1, late_reasoning_done());
    steps.splice(
        1..1,
        [reasoning_added(), reasoning_delta(0, "Weighing it.")],
    );
    steps
}

fn position(events: &[AgentEvent], matches: impl Fn(&AgentEventKind) -> bool) -> usize {
    events
        .iter()
        .position(|event| matches(&event.kind))
        .expect("the event was published")
}

fn count(events: &[AgentEvent], matches: impl Fn(&AgentEventKind) -> bool) -> usize {
    events.iter().filter(|event| matches(&event.kind)).count()
}

fn is_reasoning_completed(kind: &AgentEventKind) -> bool {
    matches!(kind, AgentEventKind::ReasoningCompleted { .. })
}

fn reasoning_updates(events: &[AgentEvent]) -> Vec<AgentReasoningSnapshot> {
    events
        .iter()
        .filter_map(|event| match event.kind {
            AgentEventKind::ReasoningUpdated { reasoning } => Some(reasoning),
            _ => None,
        })
        .collect()
}

/// The turn thought once, from the item's start to the output that ended
/// it, and stopped there.
fn assert_thought_once(service: &AgentService, session_id: AgentSessionId, events: &[AgentEvent]) {
    let updates = reasoning_updates(events);
    let thinking = updates
        .iter()
        .find(|update| update.phase == AgentReasoningPhase::Thinking)
        .expect("the item started thinking");
    let completions = updates
        .iter()
        .filter(|update| update.phase == AgentReasoningPhase::Completed)
        .collect::<Vec<_>>();
    assert_eq!(completions.len(), 1, "{updates:?}");
    let turn = service
        .snapshot(session_id)
        .unwrap()
        .turn
        .unwrap()
        .reasoning;
    assert_eq!(
        turn.turn_completed_millis,
        completions[0].turn_completed_millis
    );
    assert_eq!(
        turn.turn_completed_millis,
        completions[0].last_completed_at.unwrap() - thinking.active_since.unwrap(),
        "the interval is counted once"
    );
    assert!(
        turn.turn_completed_millis < LATE_DONE_DELAY.as_millis(),
        "the thinking stopped when the answer started: {turn:?}"
    );
}

/// The item's one completion, with the duration it froze, which the
/// committed item keeps along with the late done's text and replay.
fn assert_completed_once_and_kept(service: &AgentService, session_id: AgentSessionId) -> u64 {
    let events = service.events_since(session_id, None).unwrap().events;
    assert_eq!(count(&events, is_reasoning_completed), 1);
    let blocks = reasoning_blocks(&events);
    assert_eq!(blocks.len(), 1);
    assert_eq!(
        blocks[0].2, "Weighing it.",
        "the late done's text is not shown after the completion"
    );
    let duration_millis = blocks[0].3.unwrap();
    assert!(u128::from(duration_millis) < LATE_DONE_DELAY.as_millis());
    let history = test_session_view(service, session_id)
        .unwrap()
        .model_context;
    assert_eq!(
        reasoning_items(&history),
        [(
            "Weighing it.\n\nLate part.".to_string(),
            duration_millis,
            1,
            false
        )]
    );
    let replay = history.iter().find_map(|item| match item {
        ConversationItem::Reasoning { replay, .. } => replay.clone(),
        _ => None,
    });
    assert_eq!(replay, Some(late_replay()));
    assert_thought_once(service, session_id, &events);
    duration_millis
}

#[tokio::test]
async fn the_answer_starting_ends_the_thinking_before_the_late_reasoning_done() {
    // Started, message added, text delta, [late done], message done, completed.
    let model = PausingModel::new([with_late_reasoning(
        deterministic_assistant_events("The answer."),
        2,
    )]);
    let service = AgentService::new(Arc::new(model), FakeTools::new([]).await.registries());
    let session = service.create_session(config()).unwrap();

    let outcome = service.start_turn(session.id, "finish").await.unwrap();

    assert_eq!(outcome.final_answer, "The answer.");
    assert_completed_once_and_kept(&service, session.id);
    let events = service.events_since(session.id, None).unwrap().events;
    let completed = position(&events, is_reasoning_completed);
    let thinking_ended = position(&events, |kind| {
        matches!(
            kind,
            AgentEventKind::ReasoningUpdated { reasoning }
                if reasoning.phase == AgentReasoningPhase::Completed
        )
    });
    let answer_started = position(&events, |kind| {
        matches!(kind, AgentEventKind::AssistantMessageStarted { .. })
    });
    let first_text = position(&events, |kind| {
        matches!(kind, AgentEventKind::AssistantTextDelta { .. })
    });
    assert!(completed < answer_started && thinking_ended < answer_started);
    assert!(answer_started < first_text);
}

#[tokio::test]
async fn a_tool_call_seen_only_at_its_completion_ends_the_thinking_before_the_late_done() {
    // Started, tool call completed (never added), [late done], completed.
    let mut events = deterministic_tool_call_events();
    assert!(matches!(
        events.remove(1),
        Ok(ModelEvent::OutputItemAdded { .. })
    ));
    let tools = FakeTools::new([FakeRegisteredExchange::success(
        "list_instances",
        json!({}),
        hanafubuki_agent_tool_contracts::ToolOutput::structured(json!({ "instances": [] }), false),
    )])
    .await;
    let model = PausingModel::new([
        with_late_reasoning(events, 1),
        steps(deterministic_assistant_events("done")),
    ]);
    let service = AgentService::new(Arc::new(model), tools.registries());
    let session = service.create_session(config()).unwrap();

    let outcome = service.start_turn(session.id, "finish").await.unwrap();

    assert_eq!(outcome.final_answer, "done");
    assert_completed_once_and_kept(&service, session.id);
    let events = service.events_since(session.id, None).unwrap().events;
    let completed = position(&events, is_reasoning_completed);
    let tool_started = position(&events, |kind| {
        matches!(kind, AgentEventKind::ToolStarted { .. })
    });
    assert!(completed < tool_started);
}

#[tokio::test]
async fn a_reasoning_item_ended_by_the_answer_and_never_done_keeps_what_streamed() {
    let mut steps = steps(deterministic_assistant_events("The answer."));
    steps.splice(
        1..1,
        [reasoning_added(), reasoning_delta(0, "Half a thought.")],
    );
    let model = PausingModel::new([steps]);
    let service = AgentService::new(Arc::new(model), FakeTools::new([]).await.registries());
    let session = service.create_session(config()).unwrap();

    let outcome = service.start_turn(session.id, "finish").await.unwrap();

    assert_eq!(outcome.final_answer, "The answer.");
    let events = service.events_since(session.id, None).unwrap().events;
    let blocks = reasoning_blocks(&events);
    let history = test_session_view(&service, session.id)
        .unwrap()
        .model_context;
    assert_eq!(
        reasoning_items(&history),
        [(
            "Half a thought.".to_string(),
            blocks[0].3.unwrap(),
            1,
            false
        )]
    );
}

#[tokio::test]
async fn a_reasoning_delta_after_the_answer_started_is_kept_but_not_shown() {
    // Started, message added, [late delta], text delta, done without text…
    let mut steps = steps(deterministic_assistant_events("The answer."));
    steps.insert(2, reasoning_delta(1, "Said late."));
    steps.splice(1..1, [reasoning_added(), reasoning_delta(0, "Said first.")]);
    steps.insert(
        steps.len() - 2,
        Step::Event(Ok(ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::Reasoning {
                item_id: REASONING_ITEM.to_string(),
                summary: Vec::new(),
                content: Vec::new(),
                replay: None,
            },
        })),
    );
    let model = PausingModel::new([steps]);
    let service = AgentService::new(Arc::new(model), FakeTools::new([]).await.registries());
    let session = service.create_session(config()).unwrap();

    service.start_turn(session.id, "finish").await.unwrap();

    let events = service.events_since(session.id, None).unwrap().events;
    assert_eq!(reasoning_blocks(&events)[0].2, "Said first.");
    assert_eq!(count(&events, is_reasoning_completed), 1);
    let history = test_session_view(&service, session.id)
        .unwrap()
        .model_context;
    assert_eq!(reasoning_items(&history)[0].0, "Said first.\n\nSaid late.");
}

#[tokio::test]
async fn reasoning_that_starts_after_the_answer_item_ends_at_the_answers_next_text() {
    // Started, message added, [reasoning added, delta], text delta, [late
    // done], message done, completed.
    let mut steps = steps(deterministic_assistant_events("The answer."));
    steps.splice(3..3, late_reasoning_done());
    steps.splice(
        2..2,
        [reasoning_added(), reasoning_delta(0, "Weighing it.")],
    );
    let model = PausingModel::new([steps]);
    let service = AgentService::new(Arc::new(model), FakeTools::new([]).await.registries());
    let session = service.create_session(config()).unwrap();

    let outcome = service.start_turn(session.id, "finish").await.unwrap();

    assert_eq!(outcome.final_answer, "The answer.");
    assert_completed_once_and_kept(&service, session.id);
    let events = service.events_since(session.id, None).unwrap().events;
    let completed = position(&events, is_reasoning_completed);
    let first_text = position(&events, |kind| {
        matches!(kind, AgentEventKind::AssistantTextDelta { .. })
    });
    assert!(completed < first_text);
}
