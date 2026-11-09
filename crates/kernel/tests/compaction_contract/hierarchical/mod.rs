//! The `hierarchical` strategy (billion-context): the projection the
//! requests are built from, the compression tools and the state they
//! commit, persistence across a restart, the standard compaction as its
//! floor, switching to and from it, and a panic in the algorithm contained.
//! The fake model answers from one queue; every request it saw is kept.

use hanafubuki_agent_kernel::{
    AgentCompactionStrategy, AgentCompressionState, AgentHierarchicalCompression, AgentSessionId,
    EphemeralAgentHistoryStore,
};

use super::*;

mod fallbacks;
mod forks;
mod projection;
mod switching;

/// A first answer long enough to be worth folding.
static FIRST_ANSWER: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| "a long first answer ".repeat(200));
static SECOND_ANSWER: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| "a long second answer ".repeat(200));

const WINDOW: u64 = 100_000;

fn hierarchical_config() -> AgentSessionConfig {
    AgentSessionConfig {
        context_window_tokens: Some(WINDOW),
        compaction_strategy: AgentCompactionStrategy::Hierarchical,
        ..config()
    }
}

/// The tuning the turns run with: only the newest message is kept out of a
/// fold, and a fold spans at least 1 000 characters.
fn tuning() -> AgentHierarchicalCompression {
    AgentHierarchicalCompression {
        preserve_recent_messages: 1,
        preserve_recent_tokens: 0,
        min_compress_range_chars: 1_000,
        ..AgentHierarchicalCompression::default()
    }
}

/// A turn as the facade admits one: with `[compaction.hierarchical]`.
fn turn(text: &str) -> AgentTurnInput {
    let mut input = AgentTurnInput::from(text);
    input.turn = TurnExtensions::default().with(Arc::new(tuning()));
    input
}

/// A response calling `name` with `arguments`.
fn call(name: &str, call_id: &str, arguments: serde_json::Value) -> ScriptedResponse {
    let call = ModelToolCall {
        item_id: format!("{call_id}-item"),
        call_id: call_id.to_string(),
        namespace: None,
        name: name.to_string(),
        arguments,
        argument_error: None,
    };
    ScriptedResponse::Events(vec![
        Ok(ModelEvent::ResponseStarted {
            response_id: call_id.to_string(),
            metadata: ModelResponseMetadata::default(),
        }),
        Ok(ModelEvent::OutputItemAdded {
            item: ModelOutputItem::ToolCall { call: call.clone() },
        }),
        Ok(ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::ToolCall { call },
        }),
        Ok(ModelEvent::ResponseCompleted {
            response_id: call_id.to_string(),
            usage: Some(ModelUsage {
                input_tokens: 2_000,
                output_tokens: 1,
            }),
            end_turn: Some(false),
        }),
    ])
}

/// A `compress` call folding `range` under `topic`.
fn compress(call_id: &str, range: &str, topic: &str) -> ScriptedResponse {
    call(
        "compress",
        call_id,
        json!({
            "content": format!(
                "{range} {topic}\nThe user and the assistant went through {topic}; nothing in it is still open."
            )
        }),
    )
}

/// The two long exchanges every scenario starts from: refs m00001 to
/// m00004.
fn two_exchanges() -> Vec<ScriptedResponse> {
    vec![
        response("one", &FIRST_ANSWER, 1_000),
        response("two", &SECOND_ANSWER, 2_000),
    ]
}

async fn seed(service: &AgentService, session: AgentSessionId) {
    service.start_turn(session, turn("first")).await.unwrap();
    service.start_turn(session, turn("second")).await.unwrap();
}

fn state(service: &AgentService, session: AgentSessionId) -> AgentCompressionState {
    service
        .test_compression_state(session)
        .unwrap()
        .expect("the session has a compression state")
}

/// The request's input as one string.
fn text(request: &ModelRequest) -> String {
    serde_json::to_string(&request.input).unwrap()
}

fn tool_names(request: &ModelRequest) -> Vec<String> {
    request
        .tools
        .iter()
        .filter_map(|tool| tool.name())
        .map(|name| name.qualified())
        .collect()
}

const COMPRESSION_TOOLS: [&str; 5] = [
    "compress",
    "decompress",
    "search_context",
    "acp_status",
    "acp_cache",
];

/// The text of the tool output committed for `call_id`.
fn tool_output(service: &AgentService, session: AgentSessionId, call_id: &str) -> String {
    service
        .test_model_context(session)
        .unwrap()
        .into_iter()
        .find_map(|item| match item {
            ConversationItem::ToolOutput {
                call_id: id,
                output,
                ..
            } if id == call_id => Some(serde_json::to_string(&output.content).unwrap()),
            _ => None,
        })
        .expect("the call's output is committed")
}
