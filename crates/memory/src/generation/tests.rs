use std::collections::VecDeque;
use std::sync::Mutex;

use hanafubuki_agent_model_api::{
    ModelError, ModelEvent, ModelEventStream, ModelFuture, ModelOutputFormat, ModelOutputItem,
    ModelRequest, ModelResponseMetadata, ModelResult, ModelStream, ModelStructuredOutputCapability,
    ModelToolCall, ModelToolChoice, ModelUsage, ToolProviderCapabilities,
};

use super::*;

struct FixtureSession {
    events: Mutex<VecDeque<ModelResult<ModelEvent>>>,
    requests: Mutex<Vec<ModelRequest>>,
    supported: bool,
    native: bool,
    /// Refuses native structured output, then a forced tool too when set,
    /// as a gateway without them does.
    refuses: Option<bool>,
    learned: Mutex<Vec<ModelStructuredOutputCapability>>,
}

impl FixtureSession {
    fn new(events: Vec<ModelResult<ModelEvent>>) -> Arc<Self> {
        Arc::new(Self {
            events: Mutex::new(events.into()),
            requests: Mutex::new(Vec::new()),
            supported: true,
            native: false,
            refuses: None,
            learned: Mutex::new(Vec::new()),
        })
    }

    fn native(events: Vec<ModelResult<ModelEvent>>) -> Arc<Self> {
        Arc::new(Self {
            native: true,
            ..Arc::into_inner(Self::new(events)).unwrap()
        })
    }

    /// A gateway that refuses native output, and a forced tool when
    /// `tool_choice` is true.
    fn refusing(events: Vec<ModelResult<ModelEvent>>, tool_choice: bool) -> Arc<Self> {
        Arc::new(Self {
            native: true,
            refuses: Some(tool_choice),
            ..Arc::into_inner(Self::new(events)).unwrap()
        })
    }

    fn refusal(&self, request: &ModelRequest) -> Option<ModelError> {
        let tool_choice = self.refuses?;
        if request.output_format.is_some() {
            return Some(ModelError::UnsupportedStructuredOutput {
                status: Some(400),
                code: Some("invalid_input".into()),
                message: "Structured output (`text.format`) is not supported".into(),
                request_id: None,
            });
        }
        (tool_choice && matches!(request.tool_choice, ModelToolChoice::Required { .. })).then(
            || ModelError::UnsupportedToolChoice {
                status: Some(400),
                code: Some("invalid_input".into()),
                message: "Forced tool calls (`tool_choice`) are not supported".into(),
                request_id: None,
            },
        )
    }
}

impl ModelSession for FixtureSession {
    fn structured_output_capability(&self) -> ModelStructuredOutputCapability {
        if self.native {
            ModelStructuredOutputCapability::NativeJsonSchema
        } else {
            ModelStructuredOutputCapability::RequiredTool
        }
    }

    fn learn_structured_output(&self, _model: &str, rung: ModelStructuredOutputCapability) {
        self.learned.lock().unwrap().push(rung);
    }

    fn tool_capabilities(&self) -> ToolProviderCapabilities {
        ToolProviderCapabilities {
            function_tools: self.supported,
            strict_json_schema: true,
            maximum_tool_name_bytes: 64,
            namespace_tools: false,
            native_tool_search: false,
            freeform_tools: false,
            hosted_web_search: false,
            parallel_tool_calls: false,
            multimodal_tool_output_transport:
                hanafubuki_agent_tool_contracts::MultimodalToolOutputTransport::Unsupported,
            required_named_tool_choice: self.supported,
        }
    }

    fn stream(
        &self,
        request: ModelRequest,
        _cancellation: CancellationToken,
    ) -> ModelFuture<'_, ModelResult<ModelStream>> {
        let refusal = self.refusal(&request);
        self.requests.lock().unwrap().push(request);
        if let Some(refusal) = refusal {
            return Box::pin(async move { Err(refusal) });
        }
        let events = std::mem::take(&mut *self.events.lock().unwrap());
        Box::pin(async move { Ok(Box::new(FixtureStream { events }) as ModelStream) })
    }
}

struct FixtureStream {
    events: VecDeque<ModelResult<ModelEvent>>,
}

impl ModelEventStream for FixtureStream {
    fn next(&mut self) -> ModelFuture<'_, Option<ModelResult<ModelEvent>>> {
        let next = self.events.pop_front();
        Box::pin(async move { next })
    }
}

fn private_tool_events(name: &str, arguments: serde_json::Value) -> Vec<ModelResult<ModelEvent>> {
    vec![
        Ok(ModelEvent::ResponseStarted {
            response_id: "response-1".into(),
            metadata: ModelResponseMetadata::default(),
        }),
        Ok(ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::ToolCall {
                call: ModelToolCall {
                    item_id: "item-1".into(),
                    call_id: "call-1".into(),
                    namespace: None,
                    name: name.into(),
                    arguments,
                    argument_error: None,
                },
            },
        }),
        Ok(ModelEvent::ResponseCompleted {
            response_id: "response-1".into(),
            usage: Some(ModelUsage {
                input_tokens: 12,
                output_tokens: 5,
            }),
            end_turn: Some(false),
        }),
    ]
}

#[tokio::test]
async fn phase_one_uses_one_required_private_tool_and_redacts_both_directions() {
    let session = FixtureSession::new(private_tool_events(
        PHASE_ONE_TOOL_NAME,
        json!({
            "hasMemory": true,
            "rawMemory": "keep secret-value out",
            "rolloutSummary": "summary secret-value",
            "rolloutSlug": "useful-task"
        }),
    ));
    let result = generate_stage_one(
        session.clone(),
        "fixture".into(),
        AgentMemoryEvidence {
            physical_session_id: "session-1".into(),
            source_revision: 4,
            updated_at_millis: 100,
            task_ids: vec![1],
            bounded_text: "evidence secret-value".into(),
            truncated: false,
        },
        &["secret-value".into()],
        CancellationToken::default(),
    )
    .await
    .unwrap();
    let result = result.value.unwrap();
    assert!(!result.raw_memory.contains("secret-value"));
    assert!(!result.rollout_summary.contains("secret-value"));
    assert_eq!(result.raw_memory, "keep [REDACTED] out");
    assert_eq!(result.rollout_summary, "summary [REDACTED]");
    let requests = session.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].tool_choice,
        ModelToolChoice::Required {
            name: PHASE_ONE_TOOL_NAME.into()
        }
    );
    assert_eq!(requests[0].tools.len(), 1);
    assert!(!serde_json::to_string(&requests[0].input)
        .unwrap()
        .contains("secret-value"));
}

/// A forced tool answered with words, not an object, is no extraction.
#[tokio::test]
async fn a_forced_tool_answered_with_free_text_fails() {
    let text_session = FixtureSession::new(vec![
        Ok(ModelEvent::AssistantTextDelta {
            item_id: "item-1".into(),
            delta: "not structured".into(),
        }),
        Ok(ModelEvent::ResponseCompleted {
            response_id: "response-1".into(),
            usage: None,
            end_turn: Some(true),
        }),
    ]);
    assert!(matches!(
        generate_stage_one(
            text_session,
            "fixture".into(),
            AgentMemoryEvidence {
                physical_session_id: "session-1".into(),
                source_revision: 1,
                updated_at_millis: 100,
                task_ids: vec![1],
                bounded_text: "evidence".into(),
                truncated: false,
            },
            &[],
            CancellationToken::default(),
        )
        .await,
        Err(AgentMemoryGenerationError::Failed(_))
    ));
}

#[tokio::test]
async fn responses_and_chat_use_native_json_schema_without_tools() {
    let session = FixtureSession::native(vec![
        Ok(ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::AssistantMessage {
                item_id: "item-1".into(),
                phase: None,
                text: serde_json::to_string(&json!({
                    "hasMemory": true,
                    "rawMemory": "raw",
                    "rolloutSummary": "summary",
                    "rolloutSlug": "task"
                }))
                .unwrap(),
            },
        }),
        Ok(ModelEvent::ResponseCompleted {
            response_id: "response-1".into(),
            usage: None,
            end_turn: Some(true),
        }),
    ]);
    let result = generate_stage_one(
        session.clone(),
        "fixture".into(),
        AgentMemoryEvidence {
            physical_session_id: "session-1".into(),
            source_revision: 1,
            updated_at_millis: 100,
            task_ids: vec![1],
            bounded_text: "evidence".into(),
            truncated: false,
        },
        &[],
        CancellationToken::default(),
    )
    .await
    .unwrap();
    assert_eq!(result.value.unwrap().rollout_slug, "task");
    let requests = session.requests.lock().unwrap();
    assert!(requests[0].tools.is_empty());
    assert!(matches!(
        requests[0].output_format,
        Some(ModelOutputFormat::JsonSchema(_))
    ));
}

#[tokio::test]
async fn phase_one_ignores_fields_the_schema_does_not_name() {
    let evidence = || AgentMemoryEvidence {
        physical_session_id: "session-1".into(),
        source_revision: 1,
        updated_at_millis: 100,
        task_ids: vec![1],
        bounded_text: "evidence".into(),
        truncated: false,
    };
    let session = FixtureSession::new(private_tool_events(
        PHASE_ONE_TOOL_NAME,
        json!({
            "hasMemory": true,
            "rawMemory": "raw",
            "rolloutSummary": "summary",
            "rolloutSlug": "task",
            "confidence": 0.9,
            "notes": {"extra": true}
        }),
    ));
    let result = generate_stage_one(
        session,
        "fixture".into(),
        evidence(),
        &[],
        CancellationToken::default(),
    )
    .await
    .unwrap();
    assert_eq!(result.value.unwrap().raw_memory, "raw");
}

#[tokio::test]
async fn phase_one_keeps_what_a_partial_extraction_can_give() {
    let extract = |arguments: serde_json::Value| async move {
        generate_stage_one(
            FixtureSession::new(private_tool_events(PHASE_ONE_TOOL_NAME, arguments)),
            "fixture".into(),
            AgentMemoryEvidence {
                physical_session_id: "session-1".into(),
                source_revision: 1,
                updated_at_millis: 100,
                task_ids: vec![1],
                bounded_text: "evidence".into(),
                truncated: false,
            },
            &[],
            CancellationToken::default(),
        )
        .await
        .map(|result| result.value)
    };

    // No hasMemory, no summary, no slug: the raw memory is kept.
    let partial = extract(json!({"rawMemory": " raw\0 ", "extra": 1}))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(partial.raw_memory, "raw");
    assert_eq!(partial.rollout_summary, "");
    assert_eq!(partial.rollout_slug, "memory");

    // A bad slug is rewritten and an over-long text cut on a character
    // boundary, rather than the extraction refused.
    let repaired = extract(json!({
        "hasMemory": true,
        "rawMemory": null,
        "rolloutSummary": format!("a{}", "é".repeat(MAXIMUM_GENERATION_OUTPUT_BYTES / 2)),
        "rolloutSlug": "  Fix: Windows/WSL path_handling!! "
    }))
    .await
    .unwrap()
    .unwrap();
    assert_eq!(repaired.rollout_slug, "fix-windows-wsl-path-handling");
    assert_eq!(
        repaired.rollout_summary.len(),
        MAXIMUM_GENERATION_OUTPUT_BYTES - 1
    );
    assert_eq!(repaired.raw_memory, "");
    let long_slug = extract(json!({"rawMemory": "raw", "rolloutSlug": "a-".repeat(100)}))
        .await
        .unwrap()
        .unwrap()
        .rollout_slug;
    assert!(
        long_slug.len() <= 80 && !long_slug.ends_with('-'),
        "{long_slug}"
    );

    // The no-op gate wins over stray fields; nothing usable is no memory.
    assert_eq!(
        extract(json!({"hasMemory": false, "rawMemory": "stray"}))
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        extract(json!({"hasMemory": true, "rawMemory": " ", "rolloutSummary": null}))
            .await
            .unwrap(),
        None
    );

    // Only output that is not an extraction object at all fails.
    assert!(matches!(
        extract(json!({"rawMemory": 7})).await,
        Err(AgentMemoryGenerationError::Failed(_))
    ));
}

fn evidence() -> AgentMemoryEvidence {
    AgentMemoryEvidence {
        physical_session_id: "session-1".into(),
        source_revision: 1,
        updated_at_millis: 100,
        task_ids: vec![1],
        bounded_text: "evidence".into(),
        truncated: false,
    }
}

/// On a gateway that refuses native output the extraction comes through
/// the forced tool, and the route learns it.
#[tokio::test]
async fn a_refused_native_output_extracts_through_the_forced_tool() {
    let session = FixtureSession::refusing(
        private_tool_events(
            PHASE_ONE_TOOL_NAME,
            json!({"hasMemory": true, "rawMemory": "raw", "rolloutSummary": "s", "rolloutSlug": "t"}),
        ),
        false,
    );
    let result = generate_stage_one(
        session.clone(),
        "fixture".into(),
        evidence(),
        &[],
        CancellationToken::default(),
    )
    .await
    .unwrap();
    assert_eq!(result.value.unwrap().raw_memory, "raw");
    assert_eq!(result.usage.input_tokens, 12);
    assert_eq!(
        *session.learned.lock().unwrap(),
        [ModelStructuredOutputCapability::RequiredTool]
    );
    assert_eq!(session.requests.lock().unwrap().len(), 2);
}

/// On one that refuses both, it comes as text, read tolerantly.
#[tokio::test]
async fn a_refused_forced_tool_extracts_from_text() {
    let session = FixtureSession::refusing(
        vec![
            Ok(ModelEvent::OutputItemCompleted {
                item: ModelOutputItem::AssistantMessage {
                    item_id: "item-1".into(),
                    phase: None,
                    text: "```json\n{\"hasMemory\": true, \"rawMemory\": \"raw\"}\n```".into(),
                },
            }),
            Ok(ModelEvent::ResponseCompleted {
                response_id: "response-1".into(),
                usage: None,
                end_turn: Some(true),
            }),
        ],
        true,
    );
    let result = generate_stage_one(
        session.clone(),
        "fixture".into(),
        evidence(),
        &[],
        CancellationToken::default(),
    )
    .await
    .unwrap();
    assert_eq!(result.value.unwrap().raw_memory, "raw");
    assert_eq!(
        *session.learned.lock().unwrap(),
        [
            ModelStructuredOutputCapability::RequiredTool,
            ModelStructuredOutputCapability::Text
        ]
    );
    let requests = session.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(requests[2].tools.is_empty() && requests[2].output_format.is_none());
    assert!(matches!(
        requests[0].output_format,
        Some(ModelOutputFormat::JsonSchema(_))
    ));
}
