use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hanafubuki_agent_contracts::{
    ContextRenderer, ContextRole, ContextSectionKind, ContextSlot, SelectableSkill,
    SelectableSkills, TurnExtensions,
};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_kernel::test_support::{FakeRegisteredExchange, FakeTools};
use hanafubuki_agent_kernel::{
    AgentCompactionStatus, AgentCompactionTrigger, AgentError, AgentEventKind, AgentRevision,
    AgentService, AgentSessionConfig, AgentSessionRuntimeSnapshot, AgentSkillRef,
    AgentTurnBudgetPolicy, AgentTurnInput, ConversationItem, ModelCompactionCapability, SkillId,
    SkillScope,
};
use hanafubuki_agent_model_api::{
    ModelCompactionResult, ModelError, ModelEvent, ModelEventStream, ModelFuture, ModelInputItem,
    ModelMessagePhase, ModelOpaqueCompactionPayload, ModelOutputItem, ModelPort, ModelRequest,
    ModelResponseMetadata, ModelResult, ModelStream, ModelToolCall, ModelUsage,
};
use serde_json::json;

/// A renderer that shows a section's content as it is.
fn plain_renderer(kind: ContextSectionKind, slot: ContextSlot) -> ContextRenderer {
    ContextRenderer {
        kind,
        slot,
        order: 0,
        role: ContextRole::Developer,
        preamble: "",
        epilogue: "",
        versioned: false,
    }
}

#[derive(Clone, Debug, PartialEq)]
struct TestSessionView {
    runtime: AgentSessionRuntimeSnapshot,
    model_context: Vec<ConversationItem>,
}

impl std::ops::Deref for TestSessionView {
    type Target = AgentSessionRuntimeSnapshot;

    fn deref(&self) -> &Self::Target {
        &self.runtime
    }
}

fn test_session_view(
    service: &AgentService,
    session_id: hanafubuki_agent_kernel::AgentSessionId,
) -> Result<TestSessionView, AgentError> {
    Ok(TestSessionView {
        runtime: service.snapshot(session_id)?,
        model_context: service.test_model_context(session_id)?,
    })
}

enum ScriptedResponse {
    Events(Vec<ModelResult<ModelEvent>>),
    WaitForCancellation,
}

#[derive(Clone, Default)]
struct ScriptedModel {
    responses: Arc<Mutex<VecDeque<ScriptedResponse>>>,
    compact_responses: Arc<Mutex<VecDeque<ModelResult<ModelCompactionResult>>>>,
    requests: Arc<Mutex<Vec<ModelRequest>>>,
    compact_requests: Arc<Mutex<Vec<ModelRequest>>>,
    compaction_capability: ModelCompactionCapability,
}

impl ScriptedModel {
    fn new(responses: impl IntoIterator<Item = ScriptedResponse>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(responses.into_iter().collect())),
            compact_responses: Arc::new(Mutex::new(VecDeque::new())),
            requests: Arc::new(Mutex::new(Vec::new())),
            compact_requests: Arc::new(Mutex::new(Vec::new())),
            compaction_capability: ModelCompactionCapability::LocalSummary,
        }
    }

    fn remote(
        capability: ModelCompactionCapability,
        responses: impl IntoIterator<Item = ScriptedResponse>,
        compact_responses: impl IntoIterator<Item = ModelResult<ModelCompactionResult>>,
    ) -> Self {
        Self {
            responses: Arc::new(Mutex::new(responses.into_iter().collect())),
            compact_responses: Arc::new(Mutex::new(compact_responses.into_iter().collect())),
            requests: Arc::new(Mutex::new(Vec::new())),
            compact_requests: Arc::new(Mutex::new(Vec::new())),
            compaction_capability: capability,
        }
    }

    fn requests(&self) -> Vec<ModelRequest> {
        self.requests.lock().unwrap().clone()
    }

    fn compact_requests(&self) -> Vec<ModelRequest> {
        self.compact_requests.lock().unwrap().clone()
    }
}

impl ModelPort for ScriptedModel {
    fn compaction_capability(&self) -> ModelCompactionCapability {
        self.compaction_capability
    }

    fn stream(
        &self,
        request: ModelRequest,
        cancellation: CancellationToken,
    ) -> ModelFuture<'_, ModelResult<ModelStream>> {
        self.requests.lock().unwrap().push(request);
        let response = self.responses.lock().unwrap().pop_front();
        Box::pin(async move {
            match response.ok_or_else(|| ModelError::Protocol("fixture exhausted".to_string()))? {
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
        _cancellation: CancellationToken,
    ) -> ModelFuture<'_, ModelResult<ModelCompactionResult>> {
        self.compact_requests.lock().unwrap().push(request);
        let result = self.compact_responses.lock().unwrap().pop_front();
        Box::pin(async move {
            result.unwrap_or_else(|| {
                Err(ModelError::Protocol(
                    "fixture remote compaction exhausted".to_string(),
                ))
            })
        })
    }
}

struct ScriptedStream {
    events: VecDeque<ModelResult<ModelEvent>>,
}

impl ModelEventStream for ScriptedStream {
    fn next(&mut self) -> ModelFuture<'_, Option<ModelResult<ModelEvent>>> {
        let event = self.events.pop_front();
        Box::pin(async move { event })
    }
}

struct CancellationStream {
    cancellation: CancellationToken,
}

impl ModelEventStream for CancellationStream {
    fn next(&mut self) -> ModelFuture<'_, Option<ModelResult<ModelEvent>>> {
        let cancellation = self.cancellation.clone();
        Box::pin(async move {
            while !cancellation.is_cancelled() {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            Some(Err(ModelError::Cancelled))
        })
    }
}

fn response(id: &str, text: &str, input_tokens: u64) -> ScriptedResponse {
    ScriptedResponse::Events(vec![
        Ok(ModelEvent::ResponseStarted {
            response_id: id.to_string(),
            metadata: ModelResponseMetadata::default(),
        }),
        Ok(ModelEvent::OutputItemAdded {
            item: ModelOutputItem::AssistantMessage {
                item_id: format!("item-{id}"),
                phase: Some(ModelMessagePhase::FinalAnswer),
                text: String::new(),
            },
        }),
        Ok(ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::AssistantMessage {
                item_id: format!("item-{id}"),
                phase: Some(ModelMessagePhase::FinalAnswer),
                text: text.to_string(),
            },
        }),
        Ok(ModelEvent::ResponseCompleted {
            response_id: id.to_string(),
            usage: Some(ModelUsage {
                input_tokens,
                output_tokens: 2,
            }),
            end_turn: Some(true),
        }),
    ])
}

fn tool_response() -> ScriptedResponse {
    tool_response_with_usage(10)
}

fn tool_response_with_usage(input_tokens: u64) -> ScriptedResponse {
    tool_response_with_usage_and_commentary(input_tokens, None)
}

fn tool_response_with_usage_and_commentary(
    input_tokens: u64,
    commentary: Option<String>,
) -> ScriptedResponse {
    let mut events = vec![Ok(ModelEvent::ResponseStarted {
        response_id: "tool".to_string(),
        metadata: ModelResponseMetadata::default(),
    })];
    if let Some(text) = commentary {
        events.push(Ok(ModelEvent::OutputItemAdded {
            item: ModelOutputItem::AssistantMessage {
                item_id: "tool-commentary".to_string(),
                phase: Some(ModelMessagePhase::Commentary),
                text: String::new(),
            },
        }));
        events.push(Ok(ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::AssistantMessage {
                item_id: "tool-commentary".to_string(),
                phase: Some(ModelMessagePhase::Commentary),
                text,
            },
        }));
    }
    events.extend([
        Ok(ModelEvent::OutputItemAdded {
            item: ModelOutputItem::ToolCall {
                call: ModelToolCall {
                    item_id: "tool-item".to_string(),
                    call_id: "tool-call".to_string(),
                    namespace: None,
                    name: "list_instances".to_string(),
                    arguments: json!({}),
                    argument_error: None,
                },
            },
        }),
        Ok(ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::ToolCall {
                call: ModelToolCall {
                    item_id: "tool-item".to_string(),
                    call_id: "tool-call".to_string(),
                    namespace: None,
                    name: "list_instances".to_string(),
                    arguments: json!({}),
                    argument_error: None,
                },
            },
        }),
        Ok(ModelEvent::ResponseCompleted {
            response_id: "tool".to_string(),
            usage: Some(ModelUsage {
                input_tokens,
                output_tokens: 1,
            }),
            end_turn: Some(false),
        }),
    ]);
    ScriptedResponse::Events(events)
}

fn new_context_tool_response() -> ScriptedResponse {
    ScriptedResponse::Events(vec![
        Ok(ModelEvent::ResponseStarted {
            response_id: "new-context".to_string(),
            metadata: ModelResponseMetadata::default(),
        }),
        Ok(ModelEvent::OutputItemAdded {
            item: ModelOutputItem::ToolCall {
                call: ModelToolCall {
                    item_id: "new-context-item".to_string(),
                    call_id: "new-context-call".to_string(),
                    namespace: None,
                    name: "new_context".to_string(),
                    arguments: json!({}),
                    argument_error: None,
                },
            },
        }),
        Ok(ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::ToolCall {
                call: ModelToolCall {
                    item_id: "new-context-item".to_string(),
                    call_id: "new-context-call".to_string(),
                    namespace: None,
                    name: "new_context".to_string(),
                    arguments: json!({}),
                    argument_error: None,
                },
            },
        }),
        Ok(ModelEvent::ResponseCompleted {
            response_id: "new-context".to_string(),
            usage: Some(ModelUsage {
                input_tokens: 10,
                output_tokens: 1,
            }),
            end_turn: Some(false),
        }),
    ])
}

fn v2_compaction_response(payloads: &[&str], assistant_text: Option<&str>) -> ScriptedResponse {
    let mut events = vec![Ok(ModelEvent::ResponseStarted {
        response_id: "v2-compaction".to_string(),
        metadata: ModelResponseMetadata::default(),
    })];
    if let Some(text) = assistant_text {
        events.push(Ok(ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::AssistantMessage {
                item_id: "unexpected-summary".to_string(),
                phase: Some(ModelMessagePhase::FinalAnswer),
                text: text.to_string(),
            },
        }));
    }
    for (index, payload) in payloads.iter().enumerate() {
        events.push(Ok(ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::Compaction {
                item_id: format!("compaction-{index}"),
                payload: ModelOpaqueCompactionPayload::parse((*payload).to_string()).unwrap(),
            },
        }));
    }
    events.push(Ok(ModelEvent::ResponseCompleted {
        response_id: "v2-compaction".to_string(),
        usage: Some(ModelUsage {
            input_tokens: 40,
            output_tokens: 2,
        }),
        end_turn: Some(true),
    }));
    ScriptedResponse::Events(events)
}

fn v2_compaction_response_with_late_event() -> ScriptedResponse {
    let ScriptedResponse::Events(mut events) = v2_compaction_response(&["opaque"], None) else {
        unreachable!();
    };
    events.push(Ok(ModelEvent::OutputItemCompleted {
        item: ModelOutputItem::AssistantMessage {
            item_id: "late".to_string(),
            phase: Some(ModelMessagePhase::FinalAnswer),
            text: "late event".to_string(),
        },
    }));
    ScriptedResponse::Events(events)
}

fn v2_compaction_response_with_early_eof() -> ScriptedResponse {
    ScriptedResponse::Events(vec![Ok(ModelEvent::ResponseStarted {
        response_id: "v2-compaction".to_string(),
        metadata: ModelResponseMetadata::default(),
    })])
}

fn task_plan_response() -> ScriptedResponse {
    ScriptedResponse::Events(vec![
        Ok(ModelEvent::ResponseStarted {
            response_id: "task-plan".to_string(),
            metadata: ModelResponseMetadata::default(),
        }),
        Ok(ModelEvent::OutputItemAdded {
            item: ModelOutputItem::ToolCall {
                call: ModelToolCall {
                    item_id: "task-plan-item".to_string(),
                    call_id: "task-plan-call".to_string(),
                    namespace: None,
                    name: "update_plan".to_string(),
                    arguments: json!({}),
                    argument_error: None,
                },
            },
        }),
        Ok(ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::ToolCall {
                call: ModelToolCall {
                    item_id: "task-plan-item".to_string(),
                    call_id: "task-plan-call".to_string(),
                    namespace: None,
                    name: "update_plan".to_string(),
                    arguments: json!({
                        "explanation": "Compaction must retain this progress",
                        "plan": [{ "step": "Finish the compacted task", "status": "in_progress" }]
                    }),
                    argument_error: None,
                },
            },
        }),
        Ok(ModelEvent::ResponseCompleted {
            response_id: "task-plan".to_string(),
            usage: None,
            end_turn: Some(false),
        }),
    ])
}

fn config() -> AgentSessionConfig {
    AgentSessionConfig {
        provider_id: "provider-1".to_string(),
        provider_protocol: hanafubuki_agent_model_api::ModelApiProtocol::Responses,
        model: "fixture".to_string(),
        instructions: "fixture instructions".to_string(),
        mode: hanafubuki_agent_kernel::AgentSessionMode::Default,
        reasoning_effort: None,
        context_window_tokens: None,
        input_capabilities: Default::default(),
        auto_compaction_enabled: true,
        auto_compact_threshold_percent: 80,
        compaction_capability: ModelCompactionCapability::LocalSummary,
        sampling_retry_policy: hanafubuki_agent_kernel::AgentSamplingRetryPolicy::default(),
        compaction_strategy: Default::default(),
    }
}

mod automatic;
mod background;
mod hierarchical;
mod local;
mod recovery_fork;
mod remote_v1;
mod remote_v2;
