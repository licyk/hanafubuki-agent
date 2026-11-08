//! Deterministic fake transport shared by public contract and Agent tests.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use hanafubuki_agent_foundation::cancellation::CancellationToken;
use serde_json::json;

use crate::model::{
    ModelEvent, ModelInputItem, ModelMessagePhase, ModelOutputItem, ModelRequest,
    ModelResponseMetadata, ModelToolCall, ModelUsage,
};
use crate::port::{ModelError, ModelEventStream, ModelFuture, ModelPort, ModelResult, ModelStream};

pub const FIXTURE_MODEL: &str = "fixture-model";
pub const FIXTURE_RESPONSE_ID: &str = "response-1";
pub const FIXTURE_ITEM_ID: &str = "item-1";
pub const FIXTURE_CALL_ID: &str = "call-1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FakeModelExchange {
    pub expected_request: ModelRequest,
    pub events: Vec<ModelResult<ModelEvent>>,
}

impl FakeModelExchange {
    pub fn new(expected_request: ModelRequest, events: Vec<ModelResult<ModelEvent>>) -> Self {
        Self {
            expected_request,
            events,
        }
    }
}

#[derive(Default)]
struct FakeModelState {
    exchanges: VecDeque<FakeModelExchange>,
    requests: Vec<ModelRequest>,
}

#[derive(Clone, Default)]
pub struct FakeModelTransport {
    state: Arc<Mutex<FakeModelState>>,
}

impl FakeModelTransport {
    pub fn new(exchanges: impl IntoIterator<Item = FakeModelExchange>) -> Self {
        Self {
            state: Arc::new(Mutex::new(FakeModelState {
                exchanges: exchanges.into_iter().collect(),
                requests: Vec::new(),
            })),
        }
    }

    pub fn requests(&self) -> Vec<ModelRequest> {
        self.state.lock().unwrap().requests.clone()
    }

    pub fn remaining_exchanges(&self) -> usize {
        self.state.lock().unwrap().exchanges.len()
    }
}

impl ModelPort for FakeModelTransport {
    fn stream(
        &self,
        request: ModelRequest,
        cancellation: CancellationToken,
    ) -> ModelFuture<'_, ModelResult<ModelStream>> {
        let result = if cancellation.is_cancelled() {
            Err(ModelError::Cancelled)
        } else {
            let mut state = self.state.lock().unwrap();
            state.requests.push(request.clone());
            match state.exchanges.pop_front() {
                Some(exchange) if exchange.expected_request == request => {
                    Ok(Box::new(FakeModelEventStream {
                        events: exchange.events.into(),
                        cancellation,
                    }) as ModelStream)
                }
                Some(exchange) => {
                    state.exchanges.push_front(exchange);
                    Err(ModelError::Protocol(
                        "fake model request did not match the next deterministic fixture"
                            .to_string(),
                    ))
                }
                None => Err(ModelError::Protocol(
                    "fake model transport has no remaining exchange".to_string(),
                )),
            }
        };
        Box::pin(async move { result })
    }
}

struct FakeModelEventStream {
    events: VecDeque<ModelResult<ModelEvent>>,
    cancellation: CancellationToken,
}

impl ModelEventStream for FakeModelEventStream {
    fn next(&mut self) -> ModelFuture<'_, Option<ModelResult<ModelEvent>>> {
        let event = if self.cancellation.is_cancelled() {
            Some(Err(ModelError::Cancelled))
        } else {
            self.events.pop_front()
        };
        Box::pin(async move { event })
    }
}

pub fn deterministic_user_request(text: impl Into<String>) -> ModelRequest {
    ModelRequest {
        model: FIXTURE_MODEL.to_string(),
        instructions: "fixture instructions".to_string(),
        input: vec![ModelInputItem::user_text(text)],
        tools: Vec::new(),
        tool_choice: crate::ModelToolChoice::Auto,
        parallel_tool_calls: false,
        reasoning: None,
        output_format: None,
    }
}

pub fn deterministic_assistant_events(text: impl Into<String>) -> Vec<ModelResult<ModelEvent>> {
    let text = text.into();
    vec![
        Ok(ModelEvent::ResponseStarted {
            response_id: FIXTURE_RESPONSE_ID.to_string(),
            metadata: ModelResponseMetadata::default(),
        }),
        Ok(ModelEvent::OutputItemAdded {
            item: ModelOutputItem::AssistantMessage {
                item_id: FIXTURE_ITEM_ID.to_string(),
                phase: Some(ModelMessagePhase::FinalAnswer),
                text: String::new(),
            },
        }),
        Ok(ModelEvent::AssistantTextDelta {
            item_id: FIXTURE_ITEM_ID.to_string(),
            delta: text.clone(),
        }),
        Ok(ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::AssistantMessage {
                item_id: FIXTURE_ITEM_ID.to_string(),
                phase: Some(ModelMessagePhase::FinalAnswer),
                text,
            },
        }),
        Ok(ModelEvent::ResponseCompleted {
            response_id: FIXTURE_RESPONSE_ID.to_string(),
            usage: Some(ModelUsage {
                input_tokens: 3,
                output_tokens: 2,
            }),
            end_turn: Some(true),
        }),
    ]
}

pub fn deterministic_tool_call_events() -> Vec<ModelResult<ModelEvent>> {
    vec![
        Ok(ModelEvent::ResponseStarted {
            response_id: FIXTURE_RESPONSE_ID.to_string(),
            metadata: ModelResponseMetadata::default(),
        }),
        Ok(ModelEvent::OutputItemAdded {
            item: ModelOutputItem::ToolCall {
                call: ModelToolCall {
                    item_id: FIXTURE_ITEM_ID.to_string(),
                    call_id: FIXTURE_CALL_ID.to_string(),
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
                    item_id: FIXTURE_ITEM_ID.to_string(),
                    call_id: FIXTURE_CALL_ID.to_string(),
                    namespace: None,
                    name: "list_instances".to_string(),
                    arguments: json!({}),
                    argument_error: None,
                },
            },
        }),
        Ok(ModelEvent::ResponseCompleted {
            response_id: FIXTURE_RESPONSE_ID.to_string(),
            usage: Some(ModelUsage {
                input_tokens: 4,
                output_tokens: 1,
            }),
            end_turn: Some(false),
        }),
    ]
}
