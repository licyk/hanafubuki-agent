use std::collections::VecDeque;
use std::sync::Mutex;

use serde_json::json;

use super::*;
use crate::{
    ModelEventStream, ModelFuture, ModelResult, ModelStream, ModelToolCall,
    ToolProviderCapabilities,
};

/// A session that answers each request with the next scripted outcome and
/// keeps what it was asked and taught.
struct ScriptedSession {
    start: ModelStructuredOutputCapability,
    forced_tool: bool,
    outcomes: Mutex<VecDeque<ModelResult<Vec<ModelEvent>>>>,
    requests: Mutex<Vec<ModelRequest>>,
    learned: Mutex<Vec<ModelStructuredOutputCapability>>,
}

impl ScriptedSession {
    fn new(
        start: ModelStructuredOutputCapability,
        outcomes: Vec<ModelResult<Vec<ModelEvent>>>,
    ) -> Self {
        Self {
            start,
            forced_tool: true,
            outcomes: Mutex::new(outcomes.into()),
            requests: Mutex::new(Vec::new()),
            learned: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<ModelRequest> {
        self.requests.lock().unwrap().clone()
    }

    fn learned(&self) -> Vec<ModelStructuredOutputCapability> {
        self.learned.lock().unwrap().clone()
    }
}

impl ModelSession for ScriptedSession {
    fn tool_capabilities(&self) -> ToolProviderCapabilities {
        ToolProviderCapabilities {
            required_named_tool_choice: self.forced_tool,
            ..crate::ModelApiProtocol::Responses.tool_capabilities()
        }
    }

    fn structured_output_capability_for(&self, _model: &str) -> ModelStructuredOutputCapability {
        self.start
    }

    fn learn_structured_output(&self, _model: &str, rung: ModelStructuredOutputCapability) {
        self.learned.lock().unwrap().push(rung);
    }

    fn stream(
        &self,
        request: ModelRequest,
        _cancellation: CancellationToken,
    ) -> ModelFuture<'_, ModelResult<ModelStream>> {
        self.requests.lock().unwrap().push(request);
        let outcome = self.outcomes.lock().unwrap().pop_front().unwrap();
        Box::pin(async move {
            outcome.map(|events| {
                Box::new(ScriptedStream {
                    events: events.into(),
                }) as ModelStream
            })
        })
    }
}

struct ScriptedStream {
    events: VecDeque<ModelEvent>,
}

impl ModelEventStream for ScriptedStream {
    fn next(&mut self) -> ModelFuture<'_, Option<ModelResult<ModelEvent>>> {
        let next = self.events.pop_front().map(Ok);
        Box::pin(async move { next })
    }
}

fn request(tool: bool) -> ModelJsonObjectRequest {
    ModelJsonObjectRequest {
        subject: "title",
        model: "gateway-model".into(),
        instructions: "Title the conversation.".into(),
        input: "Explain the task center".into(),
        schema: json!({
            "type": "object",
            "properties": {"title": {"type": "string"}},
            "required": ["title"],
            "additionalProperties": false
        }),
        output_name: "agent_conversation_title",
        output_description: "A concise conversation title",
        tool: tool.then_some(ModelJsonObjectTool {
            name: "submit_agent_conversation_title",
            description: "Submit the title",
        }),
        schema_in_text: true,
        timeout: Duration::from_secs(5),
    }
}

fn refusal_of_structured_output() -> ModelError {
    ModelError::UnsupportedStructuredOutput {
        status: Some(400),
        code: Some("invalid_input".into()),
        message: "Structured output (`text.format`) is not supported by this stateless gateway."
            .into(),
        request_id: None,
    }
}

fn refusal_of_tool_choice() -> ModelError {
    ModelError::UnsupportedToolChoice {
        status: Some(400),
        code: None,
        message: "`tool_choice` is not supported".into(),
        request_id: None,
    }
}

fn completed() -> ModelEvent {
    ModelEvent::ResponseCompleted {
        response_id: "response-1".into(),
        usage: Some(ModelUsage {
            input_tokens: 10,
            output_tokens: 2,
        }),
        end_turn: Some(true),
    }
}

fn message(text: &str) -> Vec<ModelEvent> {
    vec![
        ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::AssistantMessage {
                item_id: "item-1".into(),
                phase: None,
                text: text.into(),
            },
        },
        completed(),
    ]
}

fn call(name: &str, arguments: Value) -> Vec<ModelEvent> {
    vec![
        ModelEvent::OutputItemCompleted {
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
        },
        completed(),
    ]
}

async fn ask(
    session: &ScriptedSession,
    tool: bool,
) -> Result<ModelJsonObjectAnswer, ModelJsonObjectError> {
    request_json_object(session, request(tool), CancellationToken::default()).await
}

#[tokio::test]
async fn a_refused_native_output_falls_to_the_forced_tool_and_is_learned() {
    let session = ScriptedSession::new(
        ModelStructuredOutputCapability::NativeJsonSchema,
        vec![
            Err(refusal_of_structured_output()),
            Ok(call(
                "submit_agent_conversation_title",
                json!({"title": "Task center"}),
            )),
        ],
    );

    let answer = ask(&session, true).await.unwrap();

    assert_eq!(answer.text, r#"{"title":"Task center"}"#);
    assert_eq!(answer.rung, ModelStructuredOutputCapability::RequiredTool);
    assert_eq!(answer.usage.unwrap().input_tokens, 10);
    assert_eq!(
        session.learned(),
        [ModelStructuredOutputCapability::RequiredTool]
    );
    let requests = session.requests();
    assert!(requests[0].output_format.is_some());
    assert!(requests[1].output_format.is_none());
    assert_eq!(
        requests[1].tool_choice,
        ModelToolChoice::Required {
            name: "submit_agent_conversation_title".into()
        }
    );
}

#[tokio::test]
async fn a_refused_forced_tool_falls_to_text_with_the_schema_in_the_instructions() {
    let session = ScriptedSession::new(
        ModelStructuredOutputCapability::NativeJsonSchema,
        vec![
            Err(refusal_of_structured_output()),
            Err(refusal_of_tool_choice()),
            Ok(message("```json\n{\"title\": \"Task center\"}\n```")),
        ],
    );

    let answer = ask(&session, true).await.unwrap();

    assert_eq!(answer.rung, ModelStructuredOutputCapability::Text);
    assert_eq!(
        json_object_text(&answer.text),
        r#"{"title": "Task center"}"#
    );
    assert_eq!(
        session.learned(),
        [
            ModelStructuredOutputCapability::RequiredTool,
            ModelStructuredOutputCapability::Text
        ]
    );
    let text_request = &session.requests()[2];
    assert!(text_request.tools.is_empty() && text_request.output_format.is_none());
    assert!(text_request
        .instructions
        .starts_with("Title the conversation.\n\nAnswer with only a JSON object"));
    assert!(text_request
        .instructions
        .contains(r#""required":["title"]"#));
}

#[tokio::test]
async fn the_chain_starts_at_the_routes_rung() {
    let session = ScriptedSession::new(
        ModelStructuredOutputCapability::Text,
        vec![Ok(message(r#"{"title":"Task center"}"#))],
    );

    let answer = ask(&session, true).await.unwrap();

    assert_eq!(answer.rung, ModelStructuredOutputCapability::Text);
    assert_eq!(session.requests().len(), 1);
    assert!(session.learned().is_empty());
}

#[tokio::test]
async fn without_a_usable_forced_tool_the_chain_skips_that_rung() {
    // The caller offers no tool (the reviewer's two rungs).
    let session = ScriptedSession::new(
        ModelStructuredOutputCapability::NativeJsonSchema,
        vec![
            Err(refusal_of_structured_output()),
            Ok(message(r#"{"title":"Task center"}"#)),
        ],
    );
    let answer = ask(&session, false).await.unwrap();
    assert_eq!(answer.rung, ModelStructuredOutputCapability::Text);
    assert_eq!(session.learned(), [ModelStructuredOutputCapability::Text]);

    // The session cannot force a tool.
    let session = ScriptedSession {
        forced_tool: false,
        ..ScriptedSession::new(
            ModelStructuredOutputCapability::RequiredTool,
            vec![Ok(message(r#"{"title":"Task center"}"#))],
        )
    };
    let answer = ask(&session, true).await.unwrap();
    assert_eq!(answer.rung, ModelStructuredOutputCapability::Text);
    assert!(session.requests()[0].tools.is_empty());
}

#[tokio::test]
async fn a_forced_tool_answered_with_text_is_read_and_teaches_nothing() {
    let session = ScriptedSession::new(
        ModelStructuredOutputCapability::RequiredTool,
        vec![Ok(message(" {\"title\":\"Task center\"} "))],
    );

    let answer = ask(&session, true).await.unwrap();

    assert_eq!(answer.text, r#"{"title":"Task center"}"#);
    assert_eq!(answer.rung, ModelStructuredOutputCapability::RequiredTool);
    assert!(session.learned().is_empty());
}

#[tokio::test]
async fn other_failures_end_the_chain() {
    // A refusal at the last rung, an unrelated error, a call of another
    // tool: no retry, nothing learned.
    for (start, outcome) in [
        (
            ModelStructuredOutputCapability::Text,
            Err(refusal_of_structured_output()),
        ),
        (
            ModelStructuredOutputCapability::Text,
            Err(rejection(400, "Invalid request")),
        ),
        (
            ModelStructuredOutputCapability::NativeJsonSchema,
            Ok(call("submit_agent_conversation_title", json!({}))),
        ),
    ] {
        let session = ScriptedSession::new(start, vec![outcome]);
        assert!(matches!(
            ask(&session, true).await,
            Err(ModelJsonObjectError::Failed(_))
        ));
        assert_eq!(session.requests().len(), 1);
        assert!(session.learned().is_empty());
    }
}

fn rejection(status: u16, message: &str) -> ModelError {
    ModelError::Remote {
        status: Some(status),
        code: Some("invalid_request_error".into()),
        message: message.into(),
        request_id: None,
    }
}

#[tokio::test]
async fn a_native_output_rejected_in_other_words_falls_to_the_forced_tool_learned_on_its_answer() {
    let session = ScriptedSession::new(
        ModelStructuredOutputCapability::NativeJsonSchema,
        vec![
            Err(rejection(
                400,
                "Unrecognized request argument supplied: text",
            )),
            Ok(call(
                "submit_agent_conversation_title",
                json!({"title": "Task center"}),
            )),
        ],
    );

    let answer = ask(&session, true).await.unwrap();

    assert_eq!(answer.rung, ModelStructuredOutputCapability::RequiredTool);
    assert_eq!(
        session.learned(),
        [ModelStructuredOutputCapability::RequiredTool]
    );
    assert_eq!(session.requests()[1].tools.len(), 1);
}

#[tokio::test]
async fn a_native_output_and_a_forced_tool_rejected_in_other_words_fall_to_text() {
    let session = ScriptedSession::new(
        ModelStructuredOutputCapability::NativeJsonSchema,
        vec![
            Err(rejection(422, "Invalid parameter: text")),
            Err(rejection(400, "Function calling is disabled on this route")),
            Ok(message(r#"{"title":"Task center"}"#)),
        ],
    );

    let answer = ask(&session, true).await.unwrap();

    assert_eq!(answer.rung, ModelStructuredOutputCapability::Text);
    assert_eq!(session.learned(), [ModelStructuredOutputCapability::Text]);
    let requests = session.requests();
    assert!(requests[1].output_format.is_none() && requests[1].tools.len() == 1);
    assert!(requests[2].output_format.is_none() && requests[2].tools.is_empty());
}

#[tokio::test]
async fn a_forced_tool_rejected_in_other_words_falls_to_text_after_a_learned_refusal() {
    // Native refused in words the classifier knows (learned at once), the
    // forced tool rejected in others (learned once text answers).
    let session = ScriptedSession::new(
        ModelStructuredOutputCapability::NativeJsonSchema,
        vec![
            Err(refusal_of_structured_output()),
            Err(rejection(400, "Bad request")),
            Ok(message(r#"{"title":"Task center"}"#)),
        ],
    );

    let answer = ask(&session, true).await.unwrap();

    assert_eq!(answer.rung, ModelStructuredOutputCapability::Text);
    assert_eq!(
        session.learned(),
        [
            ModelStructuredOutputCapability::RequiredTool,
            ModelStructuredOutputCapability::Text
        ]
    );
}

#[tokio::test]
async fn rejections_no_rung_answers_teach_nothing() {
    let session = ScriptedSession::new(
        ModelStructuredOutputCapability::NativeJsonSchema,
        vec![
            Err(rejection(400, "Bad request")),
            Err(rejection(400, "Bad request")),
            Err(rejection(400, "Bad request")),
        ],
    );

    assert!(matches!(
        ask(&session, true).await,
        Err(ModelJsonObjectError::Failed(_))
    ));
    assert_eq!(session.requests().len(), 3);
    assert!(session.learned().is_empty());

    // A rejection at the forced tool, then a failure unrelated to the
    // request at text: nothing learned either.
    let session = ScriptedSession::new(
        ModelStructuredOutputCapability::RequiredTool,
        vec![
            Err(rejection(400, "Bad request")),
            Err(ModelError::IdleTimeout {
                timeout: Duration::from_secs(1),
            }),
        ],
    );
    assert!(ask(&session, true).await.is_err());
    assert_eq!(session.requests().len(), 2);
    assert!(session.learned().is_empty());
}

#[tokio::test]
async fn failures_that_are_not_about_the_request_do_not_step_down() {
    let failures = [
        rejection(401, "Invalid API key"),
        rejection(403, "Forbidden"),
        rejection(404, "No such model"),
        rejection(429, "Rate limited"),
        ModelError::RequestRetriesExhausted {
            attempts: 3,
            status: Some(503),
            code: None,
            message: "Service unavailable".into(),
            request_id: None,
        },
        rejection(500, "Internal error"),
        ModelError::ContextWindowExceeded {
            status: Some(400),
            code: Some("context_length_exceeded".into()),
            message: "This model's maximum context length is 8192 tokens".into(),
            request_id: None,
        },
        ModelError::IdleTimeout {
            timeout: Duration::from_secs(1),
        },
        ModelError::Remote {
            status: None,
            code: None,
            message: "stream error".into(),
            request_id: None,
        },
    ];
    for start in [
        ModelStructuredOutputCapability::NativeJsonSchema,
        ModelStructuredOutputCapability::RequiredTool,
    ] {
        for failure in failures.iter().cloned() {
            let message = failure.to_string();
            let session = ScriptedSession::new(start, vec![Err(failure)]);
            assert!(
                matches!(
                    ask(&session, true).await,
                    Err(ModelJsonObjectError::Failed(_))
                ),
                "{message}"
            );
            assert_eq!(session.requests().len(), 1, "{message}");
            assert!(session.learned().is_empty(), "{message}");
        }
    }
}

#[test]
fn json_object_text_reads_a_fence_a_balanced_object_or_the_text() {
    assert_eq!(
        json_object_text("Here it is:\n```json\n{\"a\": 1}\n```\nDone."),
        r#"{"a": 1}"#
    );
    assert_eq!(json_object_text("```\n{\"a\": 1}\n```"), r#"{"a": 1}"#);
    assert_eq!(
        json_object_text(r#"Sure! {"a": "}{", "b": {"c": "\"}"}} and more {"d": 2}"#),
        r#"{"a": "}{", "b": {"c": "\"}"}}"#
    );
    assert_eq!(json_object_text("  no object {  "), "no object {");
    assert_eq!(json_object_text("```json\n{\"a\": 1}"), r#"{"a": 1}"#);
    // An object first is read from there, a fence inside its strings kept.
    let fenced_strings = "{\n  \"a\": \"```sh\\nls\",\n  \"b\": \"```\"\n}";
    assert_eq!(json_object_text(fenced_strings), fenced_strings);
}
