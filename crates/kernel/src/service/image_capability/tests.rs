//! A session's image input from what the model metadata knows of its
//! route: a model reported (or learned) not to read images is never sent
//! one, with no refused request, and a host's image is refused at a turn's
//! start and in a steering; a model reported to read them gets them until
//! it refuses one; the person's `disabled` wins over every source; and a
//! session takes what is known when it is created, recovered or given a
//! model, not "unknown".

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_model_api::{
    request_contains_images, ModelApiProtocol, ModelCatalogEntry, ModelCompactionCapability,
    ModelEndpoint, ModelEndpointKey, ModelError, ModelEvent, ModelEventStream, ModelFuture,
    ModelImageInputCapability, ModelImageInputPolicy, ModelInputCapabilityPolicy,
    ModelMessagePhase, ModelOutputItem, ModelPort, ModelRequest, ModelResponseMetadata,
    ModelResult, ModelRouteKey, ModelStream, IMAGE_INPUT_OMITTED_PLACEHOLDER,
};
use parking_lot::Mutex;
use tokio::sync::Notify;

use crate::{
    AgentError, AgentEventKind, AgentImageDetail, AgentKernelRegistries, AgentModelRetryStage,
    AgentPermissionMode, AgentService, AgentSessionConfig, AgentSessionId, AgentSteeringSubmission,
    AgentTurnInput, AgentTurnOutcome, EphemeralAgentHistoryStore, PreparedAgentImage,
};

use super::super::NoopAgentEventSink;

const MODEL: &str = "vision-model";
const REFUSED: &str = "the configured model does not support image input";
const LOCAL: &str = "http://127.0.0.1:1/v1";

/// What the endpoint answers a request with.
enum Reply {
    Answer,
    /// A refusal of the request's images.
    RefuseImages,
    /// An answer once the test notifies.
    Held(Arc<Notify>),
}

/// The Provider's endpoint at the test route: it answers each request with
/// the next of its replies (an answer when none is left), and keeps them.
#[derive(Default)]
struct Vision {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<ModelRequest>>,
}

impl Vision {
    fn new(replies: impl IntoIterator<Item = Reply>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies.into_iter().collect()),
            requests: Mutex::default(),
        })
    }

    fn requests(&self) -> Vec<ModelRequest> {
        self.requests.lock().clone()
    }
}

impl ModelPort for Vision {
    fn route_key(&self, model: &str) -> Option<ModelRouteKey> {
        Some(endpoint().route(model))
    }

    fn stream(
        &self,
        request: ModelRequest,
        _cancellation: CancellationToken,
    ) -> ModelFuture<'_, ModelResult<ModelStream>> {
        self.requests.lock().push(request);
        let reply = self.replies.lock().pop_front().unwrap_or(Reply::Answer);
        Box::pin(async move {
            match reply {
                Reply::Answer => Ok(answer()),
                Reply::RefuseImages => Err(ModelError::UnsupportedImageInput {
                    status: Some(400),
                    code: None,
                    message: "image input is not supported".to_string(),
                    request_id: None,
                }),
                Reply::Held(release) => {
                    release.notified().await;
                    Ok(answer())
                }
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

fn endpoint() -> ModelEndpointKey {
    ModelEndpointKey::new(
        ModelApiProtocol::ChatCompletions,
        &ModelEndpoint::parse("https://vision.test/v1").unwrap(),
    )
}

fn listed(model: &str, image_input: Option<bool>) -> ModelCatalogEntry {
    ModelCatalogEntry {
        id: model.to_string(),
        display_name: None,
        default_reasoning_effort: None,
        supported_reasoning_efforts: Vec::new(),
        context_window: None,
        max_output_tokens: None,
        image_input,
        reasoning: None,
    }
}

fn config(image_input: ModelImageInputPolicy) -> AgentSessionConfig {
    AgentSessionConfig {
        provider_id: "provider-1".to_string(),
        provider_protocol: ModelApiProtocol::Responses,
        model: MODEL.to_string(),
        instructions: "test".to_string(),
        mode: crate::AgentSessionMode::Default,
        reasoning_effort: None,
        context_window_tokens: None,
        input_capabilities: ModelInputCapabilityPolicy { image_input },
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

fn now_ms() -> u64 {
    super::now_millis_u64()
}

fn takes_images(service: &AgentService, session_id: AgentSessionId) -> bool {
    service
        .snapshot(session_id)
        .unwrap()
        .input_capabilities
        .image_input
}

fn image(service: &AgentService, session_id: AgentSessionId) -> crate::AgentImageAttachmentRef {
    service
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
        .unwrap()
}

/// Whether the session fell back from a refusal of its images.
fn fell_back(service: &AgentService, session_id: AgentSessionId) -> bool {
    service
        .events_since(session_id, None)
        .unwrap()
        .events
        .iter()
        .any(|event| {
            matches!(
                event.kind,
                AgentEventKind::ModelRetryScheduled {
                    stage: AgentModelRetryStage::ImageInputFallback,
                    ..
                }
            )
        })
}

struct Fixture {
    service: AgentService,
    model: Arc<Vision>,
}

impl Fixture {
    fn new(replies: impl IntoIterator<Item = Reply>) -> Self {
        let model = Vision::new(replies);
        let service = AgentService::new(model.clone(), AgentKernelRegistries::default());
        Self { service, model }
    }

    /// The route's listing says whether the model reads images.
    fn report(&self, image_input: Option<bool>) {
        self.service.inner.model_metadata.record_listing(
            &endpoint(),
            &[listed(MODEL, image_input)],
            now_ms(),
        );
    }

    fn session(&self, image_input: ModelImageInputPolicy) -> AgentSessionId {
        self.service.create_session(config(image_input)).unwrap().id
    }

    async fn turn_with_image(
        &self,
        session_id: AgentSessionId,
        text: &str,
    ) -> Result<AgentTurnOutcome, AgentError> {
        let input = AgentTurnInput {
            text: text.to_string(),
            images: vec![image(&self.service, session_id)],
            skills: Vec::new(),
            turn: Default::default(),
            world_state: Default::default(),
            permission_mode: AgentPermissionMode::Manual,
            origin: Default::default(),
            provisional_title: None,
        };
        self.service.start_turn(session_id, input).await
    }
}

/// A listing that says the model reads no images, landed after a turn sent
/// one: the next request carries a placeholder in the image's place at
/// once (no request is refused first, nothing falls back), and a turn
/// started with an image is refused before it reaches the model.
#[tokio::test]
async fn a_model_reported_without_image_input_is_never_sent_an_image() {
    let fixture = Fixture::new([]);
    let session_id = fixture.session(ModelImageInputPolicy::Auto);
    assert!(takes_images(&fixture.service, session_id), "nothing known");
    fixture
        .turn_with_image(session_id, "look")
        .await
        .expect("an unknown model is sent images");
    assert!(request_contains_images(&fixture.model.requests()[0]));

    fixture.report(Some(false));

    assert!(!takes_images(&fixture.service, session_id));
    assert_eq!(
        fixture.turn_with_image(session_id, "look again").await,
        Err(AgentError::InvalidInput(REFUSED.to_string()))
    );
    fixture
        .service
        .start_turn(session_id, "and now?")
        .await
        .unwrap();
    let requests = fixture.model.requests();
    assert_eq!(requests.len(), 2, "no request was refused");
    assert!(!request_contains_images(&requests[1]));
    assert!(format!("{:?}", requests[1].input).contains(IMAGE_INPUT_OMITTED_PLACEHOLDER));
    assert!(!fell_back(&fixture.service, session_id));
}

/// The person configures the model as reading images (`imageInput =
/// "supported"`) over a listing that says it reads none: it is sent
/// images; a refusal still falls back for its request, and the next turn
/// is sent images again, while a session without the setting takes the
/// learned "no". The policy's `disabled` still wins, and a model
/// configured as reading none is sent none over a listing's "yes".
#[tokio::test]
async fn a_configured_image_input_beats_what_is_listed_and_learned() {
    let fixture = Fixture::new([Reply::RefuseImages]);
    fixture.report(Some(false));
    let configured = |policy, image_input| {
        let mut config = config(policy);
        config.configured_facts.image_input = Some(image_input);
        fixture.service.create_session(config).unwrap().id
    };
    let session_id = configured(ModelImageInputPolicy::Auto, true);
    assert!(takes_images(&fixture.service, session_id));

    fixture.turn_with_image(session_id, "look").await.unwrap();
    assert!(fell_back(&fixture.service, session_id));
    assert!(takes_images(&fixture.service, session_id));
    fixture
        .turn_with_image(session_id, "look again")
        .await
        .unwrap();
    let requests = fixture.model.requests();
    assert_eq!(requests.len(), 3);
    assert!(request_contains_images(&requests[0]));
    assert!(!request_contains_images(&requests[1]), "the fallback");
    assert!(request_contains_images(&requests[2]));
    let plain = fixture.session(ModelImageInputPolicy::Auto);
    assert!(!takes_images(&fixture.service, plain));

    let disabled = configured(ModelImageInputPolicy::Disabled, true);
    assert!(!takes_images(&fixture.service, disabled));
    let other = Fixture::new([]);
    other.report(Some(true));
    let mut unsupported = config(ModelImageInputPolicy::Auto);
    unsupported.configured_facts.image_input = Some(false);
    let session_id = other.service.create_session(unsupported).unwrap().id;
    assert!(!takes_images(&other.service, session_id));
}

/// A steering message with an image, to a running turn on a model known
/// not to read images, is refused with the turn start's message.
#[tokio::test]
async fn steering_with_an_image_is_refused_for_a_model_reported_without_image_input() {
    let release = Arc::new(Notify::new());
    let fixture = Fixture::new([Reply::Held(release.clone())]);
    fixture.report(Some(false));
    let session_id = fixture.session(ModelImageInputPolicy::Auto);
    let service = fixture.service.clone();
    let running = tokio::spawn(async move { service.start_turn(session_id, "work").await });
    while fixture.model.requests().is_empty() {
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    let turn = fixture.service.snapshot(session_id).unwrap().turn.unwrap();

    let steered = fixture.service.steer_turn_with_sink(
        session_id,
        turn.id,
        turn.generation,
        AgentSteeringSubmission {
            text: "see this".to_string(),
            images: vec![image(&fixture.service, session_id)],
            host_context: None,
        },
        &NoopAgentEventSink,
    );

    assert_eq!(steered, Err(AgentError::InvalidInput(REFUSED.to_string())));
    release.notify_one();
    assert_eq!(running.await.unwrap().unwrap().final_answer, "Done.");
}

/// A model its listing says reads images is sent them, as an unknown one
/// is; when it refuses one anyway, the turn falls back once, and the
/// refusal beats the listing from then on, for every session on the route.
#[tokio::test]
async fn a_model_reported_with_image_input_is_sent_images_until_it_refuses_one() {
    let fixture = Fixture::new([Reply::Answer, Reply::RefuseImages]);
    fixture.report(Some(true));
    let route = endpoint().route(MODEL);
    let observed = || {
        fixture
            .service
            .inner
            .model_metadata
            .observed_image_input(&route, None, now_ms())
    };
    assert_eq!(observed(), ModelImageInputCapability::Supported);
    let session_id = fixture.session(ModelImageInputPolicy::Auto);
    assert!(takes_images(&fixture.service, session_id));
    fixture.turn_with_image(session_id, "look").await.unwrap();
    assert!(request_contains_images(&fixture.model.requests()[0]));

    fixture
        .turn_with_image(session_id, "look again")
        .await
        .unwrap();

    let requests = fixture.model.requests();
    assert_eq!(requests.len(), 3, "refused once, then sent without images");
    assert!(request_contains_images(&requests[1]));
    assert!(!request_contains_images(&requests[2]));
    assert!(fell_back(&fixture.service, session_id));
    assert_eq!(observed(), ModelImageInputCapability::Unsupported);
    let later = fixture.session(ModelImageInputPolicy::Auto);
    assert!(!takes_images(&fixture.service, later));
}

/// `imageInput = "disabled"` is the person's: no listing's "yes" sends an
/// image.
#[tokio::test]
async fn a_disabled_image_input_wins_over_a_reported_yes() {
    let fixture = Fixture::new([]);
    fixture.report(Some(true));
    let session_id = fixture.session(ModelImageInputPolicy::Disabled);

    assert!(!takes_images(&fixture.service, session_id));
    assert_eq!(
        fixture.turn_with_image(session_id, "look").await,
        Err(AgentError::InvalidInput(REFUSED.to_string()))
    );
    assert!(fixture.model.requests().is_empty());
}

/// Recovery gives a session what its route is known to do, before anything
/// takes it again (a compaction reads it as recovery left it).
#[test]
fn recovery_takes_what_is_known_of_the_route() {
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let created = AgentService::with_history_store(
        Vision::new([]),
        AgentKernelRegistries::default(),
        store.clone(),
    );
    let session_id = created
        .create_session(config(ModelImageInputPolicy::Auto))
        .unwrap()
        .id;
    drop(created);
    let recovered = AgentService::without_default_model_with_history_store(
        AgentKernelRegistries::default(),
        store,
    );
    recovered.inner.model_metadata.record_listing(
        &endpoint(),
        &[listed(MODEL, Some(false))],
        now_ms(),
    );

    let report = recovered
        .recover_sessions_with_test_model(Vision::new([]))
        .unwrap();

    assert_eq!((report.restored, report.quarantined.len()), (1, 0));
    let cell = recovered.session(session_id).unwrap();
    let recovered_with = cell.lock().unwrap().settings.effective_input_capabilities;
    assert!(!recovered_with.image_input);
}

/// A session created on a model, or given one, takes what is known of the
/// model's route: no images to a model listed without them, images to one
/// listed with them.
#[test]
fn creation_and_model_selection_take_what_is_known_of_the_route() {
    let service = AgentService::without_default_model_with_history_store(
        AgentKernelRegistries::default(),
        Arc::new(EphemeralAgentHistoryStore::default()),
    );
    let local = ModelEndpointKey::new(
        ModelApiProtocol::Responses,
        &ModelEndpoint::parse(LOCAL).unwrap(),
    );
    service.inner.model_metadata.record_listing(
        &local,
        &[
            listed("text-only", Some(false)),
            listed("vision", Some(true)),
        ],
        now_ms(),
    );
    let create = |model: &str| {
        let mut config = config(ModelImageInputPolicy::Auto);
        config.model = model.to_string();
        service
            .create_model_session(
                config,
                ModelApiProtocol::Responses,
                LOCAL,
                None,
                Vec::new(),
                crate::ModelRetryPolicy::default(),
                ModelCompactionCapability::LocalSummary,
                crate::AgentSessionWorkspace::default(),
            )
            .unwrap()
    };
    let select = |session: &crate::AgentSessionRuntimeSnapshot, model: &str| {
        let update = crate::AgentSessionModelConfigurationUpdate {
            provider_id: "provider-1".to_string(),
            provider_protocol: ModelApiProtocol::Responses,
            model: model.to_string(),
            instructions: "test".to_string(),
            reasoning_effort: None,
            configured_facts: Default::default(),
            input_capabilities: Default::default(),
            auto_compaction_enabled: true,
            auto_compact_threshold_percent: 80,
            compaction_capability: ModelCompactionCapability::LocalSummary,
            turn_budget: Default::default(),
        };
        service
            .set_session_model_configuration(
                session.id,
                session.generation,
                session.revision,
                ModelApiProtocol::Responses,
                LOCAL,
                None,
                Vec::new(),
                crate::ModelRetryPolicy::default(),
                update,
            )
            .unwrap()
    };

    let text_only = create("text-only");
    let vision = create("vision");
    assert!(!text_only.input_capabilities.image_input);
    assert!(vision.input_capabilities.image_input);

    let selected = select(&vision, "text-only");
    assert!(!selected.input_capabilities.image_input);
    assert!(select(&selected, "vision").input_capabilities.image_input);
}
