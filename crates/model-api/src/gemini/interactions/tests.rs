use hanafubuki_agent_tool_contracts::{ToolOutput, ToolOutputContent};
use serde_json::{json, Value};

use super::{encode, Decoder};
use crate::test_support::deterministic_user_request;
use crate::{
    ModelApiProtocol, ModelEndpoint, ModelError, ModelEvent, ModelImageDetail, ModelInputItem,
    ModelJsonSchemaOutput, ModelOutputFormat, ModelOutputItem, ModelReasoningConfig,
    ModelReasoningEffort, ModelRequest, ModelResponseMetadata, ModelRouteKey, ModelToolChoice,
    ModelToolSpec, ModelUsage, ModelUserContent,
};

fn route(host: &str, model: &str) -> ModelRouteKey {
    ModelRouteKey::new(
        ModelApiProtocol::GeminiInteractions,
        &ModelEndpoint::parse(host).unwrap(),
        model,
    )
}

fn next(decoder: &mut Decoder, frame: Value) -> Vec<ModelEvent> {
    decoder
        .decode(&frame, &ModelResponseMetadata::default())
        .unwrap()
}

fn start(route: &ModelRouteKey) -> Decoder {
    let mut decoder = Decoder::new(route.clone());
    let event = next(
        &mut decoder,
        json!({"event_type":"interaction.created","interaction":{"id":"turn","status":"in_progress"}}),
    );
    assert!(
        matches!(&event[..], [ModelEvent::ResponseStarted{response_id,..}] if response_id=="turn")
    );
    decoder
}

fn body(request: &ModelRequest, route: &ModelRouteKey) -> Value {
    serde_json::from_slice(&encode(request, route).unwrap()).unwrap()
}

#[test]
fn stateless_followup_preserves_thought_blocks_parallel_ids_and_multimodal_results() {
    let origin = route("https://one.test/v1", "gemini-4");
    let mut decoder = start(&origin);
    let thought_text = json!({"type":"text","text":"looking","annotations":[]});
    let thought_image = json!({"type":"image","mime_type":"image/png","data":"Cg==","extra": 1});
    let frames = [
        json!({"event_type":"step.start","index":0,"step":{"type":"thought","signature":"","summary":[thought_text]}}),
        json!({"event_type":"step.delta","index":0,"delta":{"type":"thought_summary","content":thought_image}}),
        json!({"event_type":"step.delta","index":0,"delta":{"type":"thought_signature","signature":"opaque"}}),
        json!({"event_type":"step.stop","index":0}),
        json!({"event_type":"step.start","index":1,"step":{"type":"function_call","id":"a","name":"inspect","arguments":{}}}),
        json!({"event_type":"step.start","index":2,"step":{"type":"function_call","id":"b","name":"inspect","arguments":{}}}),
        json!({"event_type":"step.delta","index":1,"delta":{"type":"arguments_delta","arguments":"{\"x\":"}}),
        json!({"event_type":"step.delta","index":2,"delta":{"type":"arguments_delta","arguments":"{\"x\":2}"}}),
        json!({"event_type":"step.stop","index":2}),
        json!({"event_type":"step.delta","index":1,"delta":{"type":"arguments_delta","arguments":"1}"}}),
        json!({"event_type":"step.stop","index":1}),
        json!({"event_type":"interaction.completed","interaction":{"id":"turn","status":"requires_action","usage":{"total_input_tokens":11,"total_output_tokens":90,"total_thought_tokens":245,"total_cached_tokens":2,"total_tokens":346}}}),
    ];
    let events: Vec<_> = frames
        .into_iter()
        .flat_map(|frame| next(&mut decoder, frame))
        .collect();
    assert!(matches!(
        events.last(),
        Some(ModelEvent::ResponseCompleted {
            end_turn: Some(false),
            usage: Some(ModelUsage {
                input_tokens: 11,
                output_tokens: 335,
                cache_read_tokens: Some(2),
                ..
            }),
            ..
        })
    ));
    let mut request = deterministic_user_request("go");
    for event in events {
        match event {
            ModelEvent::OutputItemCompleted {
                item:
                    ModelOutputItem::Reasoning {
                        replay: Some(replay),
                        summary,
                        ..
                    },
            } => {
                assert_eq!(summary, ["looking"]);
                request.input.push(ModelInputItem::Reasoning { replay });
            }
            ModelEvent::OutputItemCompleted {
                item: ModelOutputItem::ToolCall { call },
            } => {
                assert!(call.argument_error.is_none());
                request.input.push(ModelInputItem::ToolCall {
                    call_id: call.call_id,
                    namespace: call.namespace,
                    name: call.name,
                    arguments: call.arguments,
                });
            }
            _ => {}
        }
    }
    request.input.push(ModelInputItem::ToolOutput {
        call_id: "b".into(),
        output: ToolOutput {
            content: vec![
                ToolOutputContent::Text {
                    text: "failed".into(),
                },
                ToolOutputContent::Image {
                    mime_type: "image/png".into(),
                    data: "Cg==".into(),
                    detail: None,
                },
            ],
            structured_content: Some(json!({"status":"retry"})),
            is_error: true,
        },
    });
    request.input.push(ModelInputItem::ToolOutput {
        call_id: "a".into(),
        output: ToolOutput::structured(json!({"answer":1}), false),
    });
    let payload = body(&request, &origin);
    assert_eq!(payload["stream"], true);
    assert_eq!(payload["store"], false);
    assert!(payload.get("previous_interaction_id").is_none());
    assert_eq!(
        payload["input"][1],
        json!({"type":"thought","signature":"opaque","summary":[thought_text,thought_image]})
    );
    for (i, (id, x)) in [("a", 1), ("b", 2)].into_iter().enumerate() {
        assert_eq!(
            payload["input"][i + 2],
            json!({"type":"function_call","id":id,"name":"inspect","arguments":{"x":x}})
        );
    }
    assert_eq!(
        payload["input"][4],
        json!({"type":"function_result","call_id":"b","name":"inspect","is_error":true,"result":[{"type":"text","text":"failed"},{"type":"image","mime_type":"image/png","data":"Cg=="},{"type":"text","text":"{\"status\":\"retry\"}"}]})
    );
    assert_eq!(
        payload["input"][5]["result"],
        json!([{"type":"text","text":"{\"answer\":1}"}])
    );

    let same_endpoint = body(&request, &route("https://one.test/v1", "another"));
    assert_eq!(same_endpoint["input"][1], payload["input"][1]);
    let other_endpoint = body(&request, &route("https://two.test/v1", "gemini-4"));
    assert!(!other_endpoint.to_string().contains("opaque"));
    assert_eq!(other_endpoint["input"][1]["id"], "a");
}

#[test]
fn stable_v1_request_uses_content_tool_choice_thinking_and_schema_fields() {
    let origin = route("https://one.test/v1", "gemini-4");
    let mut request = deterministic_user_request("ignored");
    request.model = "models/gemini-4".into();
    request.instructions = "system".into();
    request.input = vec![ModelInputItem::UserMessage {
        content: vec![
            ModelUserContent::InputText {
                text: "inspect".into(),
            },
            ModelUserContent::InputImage {
                mime_type: "image/png".into(),
                data: vec![1, 2, 3],
                detail: ModelImageDetail::High,
            },
        ],
    }];
    request.reasoning = Some(ModelReasoningConfig {
        effort: ModelReasoningEffort::High,
        max_output_tokens: None,
    });
    let schema = json!({"type":"object","properties":{"ok":{"type":"boolean"}},"required":["ok"]});
    request.tools = vec![ModelToolSpec {
        name: "review".into(),
        description: "Check".into(),
        input_schema: schema.clone(),
        strict: false,
    }
    .try_into()
    .unwrap()];
    request.tool_choice = ModelToolChoice::Required {
        name: "review".into(),
    };
    request.output_format = Some(ModelOutputFormat::JsonSchema(ModelJsonSchemaOutput {
        name: "out".into(),
        description: None,
        schema: schema.clone(),
        strict: true,
    }));
    let payload = body(&request, &origin);
    assert_eq!(payload["model"], "gemini-4");
    assert_eq!(payload["system_instruction"], "system");
    assert_eq!(
        payload["input"][0],
        json!({"type":"user_input","content":[{"type":"text","text":"inspect"},{"type":"image","mime_type":"image/png","data":"AQID","resolution":"high"}]})
    );
    assert_eq!(
        payload["tools"],
        json!([{"type":"function","name":"review","description":"Check","parameters":schema}])
    );
    assert_eq!(
        payload["generation_config"],
        json!({"tool_choice":{"allowed_tools":{"mode":"any","tools":["review"]}},"thinking_summaries":"auto","thinking_level":"high"})
    );
    assert_eq!(
        payload["response_format"],
        json!({"type":"text","mime_type":"application/json","schema":schema})
    );
    for (choice, expected) in [
        (ModelToolChoice::Auto, "auto"),
        (ModelToolChoice::None, "none"),
    ] {
        request.tool_choice = choice;
        assert_eq!(
            body(&request, &origin)["generation_config"]["tool_choice"],
            expected
        );
    }
}

#[test]
fn stream_keeps_initial_text_and_ignores_unrelated_extensions() {
    let origin = route("https://one.test/v1", "m");
    let mut decoder = start(&origin);
    let events: Vec<_> = [
        json!({"event_type":"interaction.status_update","status":"in_progress"}),
        json!({"event_type":"step.start","index":0,"step":{"type":"unknown_step"}}),
        json!({"event_type":"step.delta","index":0,"delta":{"type":"other"}}),
        json!({"event_type":"step.stop","index":0}),
        json!({"event_type":"step.start","index":1,"step":{"type":"model_output","content":[{"type":"text","text":"first "}]}}),
        json!({"event_type":"step.delta","index":1,"delta":{"type":"text","text":"second"}}),
        json!({"event_type":"step.stop","index":1,"usage":{"total_input_tokens":3},"step_usage":{"total_input_tokens":2}}),
        json!({"event_type":"interaction.completed","interaction":{"id":"turn","status":"completed","usage":{"total_output_tokens":"bad"}}}),
    ].into_iter().flat_map(|frame| next(&mut decoder,frame)).collect();
    assert!(matches!(&events[..],
        [ModelEvent::OutputItemAdded{..}, ModelEvent::AssistantTextDelta{delta:first,..},ModelEvent::AssistantTextDelta{delta:second,..}, ModelEvent::OutputItemCompleted{item:ModelOutputItem::AssistantMessage{text,..}}, ModelEvent::ResponseCompleted{end_turn:Some(true),usage:Some(ModelUsage{input_tokens:3,..}),..}]
        if first=="first " && second=="second" && text=="first second"));
}

#[test]
fn malformed_arguments_are_recoverable_and_a_partial_call_never_completes() {
    let origin = route("https://one.test/v1", "m");
    let mut decoder = start(&origin);
    let call = json!({"event_type":"step.start","index":1,"step":{"type":"function_call","id":"b","name":"f"}});
    next(&mut decoder, call.clone());
    next(
        &mut decoder,
        json!({"event_type":"step.delta","index":1,"delta":{"type":"arguments_delta","arguments":"broken"}}),
    );
    let events = next(&mut decoder, json!({"event_type":"step.stop","index":1}));
    assert!(
        matches!(&events[..],[ModelEvent::OutputItemCompleted{item:ModelOutputItem::ToolCall {call}}] if call.call_id=="b" && call.argument_error.is_some() && call.arguments==json!({}))
    );
    let terminal = |status| {
        let mut decoder = start(&origin);
        next(&mut decoder, call.clone());
        let frame = json!({"event_type":"interaction.completed", "interaction":{"id":"turn","status":status}});
        decoder.decode(&frame, &Default::default())
    };
    assert!(matches!(
        terminal("requires_action"),
        Err(ModelError::StreamInterrupted(_))
    ));
    assert!(matches!(terminal("failed"), Err(ModelError::Remote { .. })));
    assert_eq!(terminal("cancelled"), Err(ModelError::Cancelled));
    assert!(matches!(
        terminal("incomplete").as_deref(),
        Ok([ModelEvent::ResponseIncomplete { .. }])
    ));
}

#[test]
fn error_events_and_failed_resource_errors_reach_the_shared_remote_error_handler() {
    let error = json!({"code":"quota_exceeded", "message":"Model quota reached"});
    for frame in [
        json!({"event_type":"error","error":error}),
        json!({"event_type":"interaction.completed","interaction":{"status":"failed","error":error}}),
        json!({"event_type":"interaction.completed","interaction":{"status":"failed","errors":[null,{},error]}}),
    ] {
        assert_eq!(super::stream_error(&frame), Some(&error));
    }
    let empty_error = json!({"event_type":"error"});
    assert!(super::stream_error(&empty_error).is_some());
    let success = json!({"event_type":"interaction.completed","interaction":{"status":"completed","errors":[error]}});
    assert!(super::stream_error(&success).is_none());
}
