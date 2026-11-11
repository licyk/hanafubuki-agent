//! The reasoning a session's requests carry from what the model metadata
//! knows of its route: the configured effort narrowed to the listed levels,
//! none for a model known to take none, the route's largest output; a
//! refusal of the parameter retried once without it and learned; and a
//! compaction's request narrowed, with its images sanitized, as a turn's.

use std::collections::VecDeque;
use std::sync::Arc;

use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_model_api::{
    request_contains_images, ModelApiProtocol, ModelCatalogEntry, ModelCompactionCapability,
    ModelEndpoint, ModelEndpointKey, ModelError, ModelEvent, ModelEventStream, ModelFuture,
    ModelImageInputPolicy, ModelInputCapabilityPolicy, ModelMessagePhase, ModelMetadataSource,
    ModelOutputItem, ModelPort, ModelReasoningEffort, ModelRequest, ModelResponseMetadata,
    ModelResult, ModelRouteKey, ModelStream,
};
use parking_lot::Mutex;

use crate::model_metadata::ConfiguredModelFacts;
use crate::{
    AgentError, AgentEventKind, AgentImageDetail, AgentKernelRegistries, AgentModelRetryStage,
    AgentPermissionMode, AgentService, AgentSessionConfig, AgentSessionId, AgentTurnInput,
    ModelReasoningSupport, PreparedAgentImage,
};

const MODEL: &str = "reasoning-model";

/// The Provider's endpoint at the test route: it refuses each request
/// while refusals are left, answers otherwise, and keeps the requests.
struct Reasoner {
    refusals: Mutex<VecDeque<ModelError>>,
    requests: Mutex<Vec<ModelRequest>>,
}

impl Reasoner {
    fn new(refusals: impl IntoIterator<Item = ModelError>) -> Arc<Self> {
        Arc::new(Self {
            refusals: Mutex::new(refusals.into_iter().collect()),
            requests: Mutex::default(),
        })
    }

    fn requests(&self) -> Vec<ModelRequest> {
        self.requests.lock().clone()
    }

    fn efforts(&self) -> Vec<Option<String>> {
        self.requests()
            .iter()
            .map(|request| {
                request
                    .reasoning
                    .as_ref()
                    .map(|reasoning| reasoning.effort.to_string())
            })
            .collect()
    }
}

impl ModelPort for Reasoner {
    fn route_key(&self, model: &str) -> Option<ModelRouteKey> {
        Some(endpoint().route(model))
    }

    fn stream(
        &self,
        request: ModelRequest,
        _cancellation: CancellationToken,
    ) -> ModelFuture<'_, ModelResult<ModelStream>> {
        self.requests.lock().push(request);
        let refusal = self.refusals.lock().pop_front();
        Box::pin(async move {
            match refusal {
                Some(error) => Err(error),
                None => Ok(answer()),
            }
        })
    }
}

struct Answer(VecDeque<ModelResult<ModelEvent>>);

impl ModelEventStream for Answer {
    fn next(&mut self) -> ModelFuture<'_, Option<ModelResult<ModelEvent>>> {
        let event = self.0.pop_front();
        Box::pin(async move { event })
    }
}

fn answer() -> ModelStream {
    let item = |text: &str| ModelOutputItem::AssistantMessage {
        item_id: "answer".to_string(),
        phase: Some(ModelMessagePhase::FinalAnswer),
        text: text.to_string(),
    };
    Box::new(Answer(VecDeque::from([
        Ok(ModelEvent::ResponseStarted {
            response_id: "response".to_string(),
            metadata: ModelResponseMetadata::default(),
        }),
        Ok(ModelEvent::OutputItemAdded { item: item("") }),
        Ok(ModelEvent::OutputItemCompleted {
            item: item("Done."),
        }),
        Ok(ModelEvent::ResponseCompleted {
            response_id: "response".to_string(),
            usage: None,
            end_turn: Some(true),
        }),
    ])))
}

fn refusal() -> ModelError {
    ModelError::UnsupportedReasoning {
        status: Some(400),
        code: Some("unsupported_parameter".to_string()),
        message: "Unsupported parameter: 'reasoning_effort' is not supported with this model."
            .to_string(),
        request_id: None,
    }
}

fn endpoint() -> ModelEndpointKey {
    ModelEndpointKey::new(
        ModelApiProtocol::ChatCompletions,
        &ModelEndpoint::parse("https://reasoning.test/v1").unwrap(),
    )
}

fn effort(value: &str) -> ModelReasoningEffort {
    ModelReasoningEffort::parse(value).unwrap()
}

fn listed(reasoning: Option<bool>, levels: &[&str], max_output: Option<u64>) -> ModelCatalogEntry {
    ModelCatalogEntry {
        id: MODEL.to_string(),
        display_name: None,
        default_reasoning_effort: None,
        supported_reasoning_efforts: levels.iter().map(|level| effort(level)).collect(),
        context_window: None,
        max_output_tokens: max_output,
        image_input: None,
        reasoning,
    }
}

fn config(reasoning_effort: Option<&str>) -> AgentSessionConfig {
    AgentSessionConfig {
        provider_id: "provider-1".to_string(),
        provider_protocol: ModelApiProtocol::ChatCompletions,
        model: MODEL.to_string(),
        instructions: "test".to_string(),
        mode: crate::AgentSessionMode::Default,
        reasoning_effort: reasoning_effort.map(effort),
        context_window_tokens: None,
        input_capabilities: ModelInputCapabilityPolicy {
            image_input: ModelImageInputPolicy::Auto,
        },
        auto_compaction_enabled: true,
        auto_compact_threshold_percent: 80,
        compaction_capability: ModelCompactionCapability::LocalSummary,
        sampling_retry_policy: crate::AgentSamplingRetryPolicy::default(),
        context_window_source: None,
        compaction_strategy: Default::default(),
        scope: Default::default(),
        configured_facts: Default::default(),
        role_instructions: None,
    }
}

struct Fixture {
    service: AgentService,
    model: Arc<Reasoner>,
}

impl Fixture {
    fn new(refusals: impl IntoIterator<Item = ModelError>) -> Self {
        let model = Reasoner::new(refusals);
        let service = AgentService::new(model.clone(), AgentKernelRegistries::default());
        Self { service, model }
    }

    fn report(&self, entry: ModelCatalogEntry) {
        self.service.inner.model_metadata.record_listing(
            &endpoint(),
            &[entry],
            super::now_millis_u64(),
        );
    }

    fn session(&self, reasoning_effort: Option<&str>) -> AgentSessionId {
        self.service
            .create_session(config(reasoning_effort))
            .unwrap()
            .id
    }

    async fn turn(&self, session_id: AgentSessionId) -> Result<(), AgentError> {
        self.service
            .start_turn(session_id, "think")
            .await
            .map(|_| ())
    }

    fn fell_back(&self, session_id: AgentSessionId) -> usize {
        self.service
            .events_since(session_id, None)
            .unwrap()
            .events
            .iter()
            .filter(|event| {
                matches!(
                    event.kind,
                    AgentEventKind::ModelRetryScheduled {
                        stage: AgentModelRetryStage::ReasoningFallback,
                        ..
                    }
                )
            })
            .count()
    }

    fn learned(&self) -> Option<(ModelReasoningSupport, ModelMetadataSource)> {
        self.service
            .inner
            .model_metadata
            .resolve(
                Some(&endpoint().route(MODEL)),
                MODEL,
                ConfiguredModelFacts::default(),
                super::now_millis_u64(),
            )
            .reasoning
            .map(|fact| (fact.value, fact.source))
    }
}

#[tokio::test]
async fn the_configured_effort_is_narrowed_to_the_listed_levels() {
    let fixture = Fixture::new([]);
    let xhigh = fixture.session(Some("xhigh"));
    let minimal = fixture.session(Some("minimal"));
    let unset = fixture.session(None);
    // Nothing known: as configured.
    fixture.turn(xhigh).await.unwrap();

    fixture.report(listed(Some(true), &["low", "medium", "high"], Some(64_000)));
    for session_id in [xhigh, minimal, unset] {
        fixture.turn(session_id).await.unwrap();
    }

    assert_eq!(
        fixture.model.efforts(),
        [Some("xhigh"), Some("high"), Some("low"), None].map(|effort| effort.map(String::from))
    );
    let requests = fixture.model.requests();
    assert_eq!(
        requests[0].reasoning.as_ref().unwrap().max_output_tokens,
        None
    );
    assert_eq!(
        requests[1].reasoning.as_ref().unwrap().max_output_tokens,
        Some(64_000),
        "the route's largest output goes with the effort"
    );
}

#[tokio::test]
async fn a_model_reasoning_without_listed_levels_takes_the_configured_effort() {
    let fixture = Fixture::new([]);
    fixture.report(listed(Some(true), &[], None));
    let session_id = fixture.session(Some("vendor.tier-2"));
    fixture.turn(session_id).await.unwrap();
    assert_eq!(fixture.model.efforts(), [Some("vendor.tier-2".to_string())]);
}

#[tokio::test]
async fn a_model_listed_without_reasoning_is_sent_none() {
    let fixture = Fixture::new([]);
    fixture.report(listed(Some(false), &[], None));
    let session_id = fixture.session(Some("high"));
    fixture.turn(session_id).await.unwrap();
    assert_eq!(fixture.model.efforts(), [None]);
    assert_eq!(fixture.fell_back(session_id), 0);
}

/// The levels the person configured for the model (`reasoningEfforts`)
/// replace the listed ones, an empty list sends none, and the largest
/// output configured goes with the effort; a refusal under configured
/// levels still falls back for its request, and the next turn sends the
/// effort again.
#[tokio::test]
async fn configured_levels_replace_the_listed_ones() {
    let fixture = Fixture::new([refusal()]);
    fixture.report(listed(Some(true), &["low", "medium"], Some(64_000)));
    let configured = |levels: &[&str]| {
        let mut config = config(Some("xhigh"));
        config.configured_facts.reasoning_efforts =
            Some(levels.iter().map(|level| effort(level)).collect());
        config.configured_facts.max_output_tokens = Some(8_000);
        fixture.service.create_session(config).unwrap().id
    };
    let high = configured(&["high"]);
    let none = configured(&[]);

    fixture.turn(high).await.unwrap();
    fixture.turn(high).await.unwrap();
    fixture.turn(none).await.unwrap();

    assert_eq!(
        fixture.model.efforts(),
        [Some("high"), None, Some("high"), None].map(|effort| effort.map(String::from))
    );
    assert_eq!(fixture.fell_back(high), 1);
    assert_eq!(
        fixture.model.requests()[0]
            .reasoning
            .as_ref()
            .unwrap()
            .max_output_tokens,
        Some(8_000)
    );
}

/// The Provider refuses the parameter: the request is sent once more
/// without it, the route learns "no reasoning", which beats the listing's
/// "yes", and the session's next turn sends none from its first request.
#[tokio::test]
async fn a_refused_reasoning_parameter_is_retried_without_it_and_learned() {
    let fixture = Fixture::new([refusal()]);
    fixture.report(listed(Some(true), &[], None));
    let session_id = fixture.session(Some("high"));
    fixture.turn(session_id).await.unwrap();
    assert_eq!(fixture.model.efforts(), [Some("high".to_string()), None]);
    assert_eq!(fixture.fell_back(session_id), 1);
    assert_eq!(
        fixture.learned(),
        Some((
            ModelReasoningSupport::Unsupported,
            ModelMetadataSource::Learned
        ))
    );

    fixture.turn(session_id).await.unwrap();
    assert_eq!(fixture.model.efforts().len(), 3, "no request was refused");
    assert_eq!(fixture.model.efforts()[2], None);
    assert_eq!(fixture.fell_back(session_id), 1);
}

#[tokio::test]
async fn a_second_refusal_or_one_of_a_request_without_reasoning_fails_the_turn() {
    let fixture = Fixture::new([refusal(), refusal()]);
    let session_id = fixture.session(Some("high"));
    assert!(matches!(
        fixture.turn(session_id).await,
        Err(AgentError::UnsupportedReasoning { .. })
    ));
    assert_eq!(fixture.model.efforts(), [Some("high".to_string()), None]);

    let fixture = Fixture::new([refusal()]);
    let session_id = fixture.session(None);
    assert!(matches!(
        fixture.turn(session_id).await,
        Err(AgentError::UnsupportedReasoning { .. })
    ));
    assert_eq!(
        fixture.model.requests().len(),
        1,
        "nothing to retry without"
    );
    assert_eq!(fixture.learned(), None);
}

/// A compaction's own request takes the narrowed effort, and placeholders
/// for the images of a model known to read none, as a turn's request does.
#[tokio::test]
async fn a_compaction_request_is_narrowed_and_sanitized_as_a_turns() {
    let fixture = Fixture::new([]);
    let session_id = fixture.session(Some("xhigh"));
    let image = fixture
        .service
        .stage_image_attachment(
            session_id,
            PreparedAgentImage {
                bytes: vec![1, 2, 3],
                mime_type: "image/png".to_string(),
                width: 1,
                height: 1,
                display_name: "fixture.png".to_string(),
                detail: AgentImageDetail::High,
            },
        )
        .unwrap();
    let input = AgentTurnInput {
        text: "look".to_string(),
        images: vec![image],
        skills: Vec::new(),
        turn: Default::default(),
        world_state: Default::default(),
        permission_mode: AgentPermissionMode::Manual,
        origin: Default::default(),
        provisional_title: None,
    };
    fixture.service.start_turn(session_id, input).await.unwrap();
    assert!(request_contains_images(&fixture.model.requests()[0]));

    let mut entry = listed(Some(true), &["low", "high"], None);
    entry.image_input = Some(false);
    fixture.report(entry);
    let read = fixture.service.snapshot(session_id).unwrap();
    fixture
        .service
        .compact_session(session_id, read.generation, read.revision)
        .await
        .unwrap();

    let compaction = fixture.model.requests().pop().unwrap();
    assert!(!request_contains_images(&compaction));
    assert_eq!(compaction.reasoning.unwrap().effort, effort("high"));
}
