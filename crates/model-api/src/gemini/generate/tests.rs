use super::*;
use crate::{
    ModelApiProtocol, ModelEndpoint, ModelEvent, ModelInputItem, ModelOutputItem, ModelRequest,
    ModelResponseMetadata, ModelRouteKey, ModelToolChoice, ModelUsage,
};
use hanafubuki_agent_tool_contracts::{ToolOutput, ToolOutputContent};
use serde_json::{json, Value};

fn route(model: &str) -> ModelRouteKey {
    ModelRouteKey::new(
        ModelApiProtocol::GeminiGenerateContent,
        &ModelEndpoint::parse("https://gemini.test/prefix").unwrap(),
        model,
    )
}

fn request(model: &str, input: Vec<ModelInputItem>) -> ModelRequest {
    ModelRequest {
        model: model.into(),
        instructions: "Global instructions".into(),
        input,
        tools: Vec::new(),
        tool_choice: ModelToolChoice::Auto,
        parallel_tool_calls: true,
        reasoning: None,
        output_format: None,
    }
}

fn body(request: &ModelRequest) -> Value {
    serde_json::from_slice(&encode(request, &route(&request.model)).unwrap()).unwrap()
}

#[test]
fn encodes_context_and_images_in_order_and_native_output_contracts() {
    let mut req = request(
        "gemini-3-flash",
        vec![
            ModelInputItem::ContextFragment {
                role: crate::ModelContextRole::Developer,
                kind: "fragment".into(),
                content: "First dynamic instruction".into(),
            },
            ModelInputItem::UserMessage {
                content: vec![
                    crate::ModelUserContent::InputText {
                        text: "Identify".into(),
                    },
                    crate::ModelUserContent::InputImage {
                        mime_type: "image/png".into(),
                        data: vec![1, 2, 3],
                        detail: Default::default(),
                    },
                ],
            },
            ModelInputItem::AssistantMessage {
                phase: None,
                text: "A picture".into(),
            },
            ModelInputItem::user_text("More"),
        ],
    );
    req.output_format = Some(crate::ModelOutputFormat::JsonSchema(
        crate::ModelJsonSchemaOutput {
            name: "answer".into(),
            description: None,
            schema: json!({"type":"object"}),
            strict: true,
        },
    ));
    req.reasoning = Some(crate::ModelReasoningConfig {
        effort: crate::ModelReasoningEffort::High,
        max_output_tokens: None,
    });
    req.tools.push(
        crate::ModelToolSpec {
            name: "list_files".into(),
            description: "List".into(),
            input_schema: json!({"type":"object","properties":{},"additionalProperties":false}),
            strict: false,
        }
        .try_into()
        .unwrap(),
    );
    req.tool_choice = ModelToolChoice::Required {
        name: "list_files".into(),
    };
    let json = body(&req);
    assert_eq!(
        json["systemInstruction"]["parts"][0]["text"],
        "Global instructions"
    );
    assert_eq!(
        json["contents"][0]["parts"],
        json!([{"text":"First dynamic instruction"},{"text":"Identify"},{"inlineData":{"mimeType":"image/png","data":"AQID"}}])
    );
    assert_eq!(json["contents"][1]["role"], "model");
    assert_eq!(json["contents"][2]["parts"][0]["text"], "More");
    assert_eq!(
        json["tools"][0]["functionDeclarations"][0]["parametersJsonSchema"]["additionalProperties"],
        false
    );
    assert_eq!(
        json["toolConfig"]["functionCallingConfig"],
        json!({"mode":"ANY","allowedFunctionNames":["list_files"]})
    );
    assert_eq!(
        json["generationConfig"]["responseJsonSchema"],
        json!({"type":"object"})
    );
    assert_eq!(
        json["generationConfig"]["thinkingConfig"]["thinkingLevel"],
        "HIGH"
    );
}

#[test]
fn streamed_parallel_calls_round_trip_signatures_results_and_image_to_their_original_parts() {
    let model = "gemini-3-flash";
    let mut decoder = Decoder::new(route(model));
    let meta = ModelResponseMetadata::default();
    let parts = json!([
        {"thought":true,"text":"Considering the files"},
        {"text":"I will look. "},
        {"text":"","thoughtSignature":"text-signature"},
        {"functionCall":{"name":"read","args":{"n":1}},"thoughtSignature":"call-signature"},
        {"functionCall":{"name":"read","id":"native-2","args":{"n":2}}}
    ]);
    let mut events = decoder.decode(&json!({
        "responseId": "abc",
        "candidates": [
            {"index": 1, "content": {"parts": [{"text": "another candidate"}]}},
            {"index": 0, "content": {"parts": parts}, "finishReason": "STOP"}
        ],
        "usageMetadata": {"promptTokenCount": 100, "cachedContentTokenCount": 80, "candidatesTokenCount": 15}
    }), &meta).unwrap();
    events.extend(decoder.decode(&json!({
        "usageMetadata": {"toolUsePromptTokenCount": 2, "candidatesTokenCount": 20, "thoughtsTokenCount": 10, "totalTokenCount": 132}
    }), &meta).unwrap());
    assert!(!events
        .iter()
        .any(|event| matches!(event, ModelEvent::ResponseCompleted { .. })));
    events.extend(decoder.finish().unwrap());
    assert!(decoder.finish().unwrap().is_empty());
    assert!(matches!(
        events.last(),
        Some(ModelEvent::ResponseCompleted {
            end_turn: Some(false),
            usage: Some(ModelUsage {
                input_tokens: 102,
                output_tokens: 30,
                cache_read_tokens: Some(80),
                ..
            }),
            ..
        })
    ));
    let mut input = vec![ModelInputItem::user_text("Look")];
    let mut ids = Vec::new();
    for item in events.into_iter().filter_map(|e| match e {
        ModelEvent::OutputItemCompleted { item } => Some(item),
        _ => None,
    }) {
        match item {
            ModelOutputItem::AssistantMessage { text, phase, .. } => {
                input.push(ModelInputItem::AssistantMessage { phase, text })
            }
            ModelOutputItem::ToolCall { call } => {
                ids.push(call.call_id.clone());
                input.push(ModelInputItem::ToolCall {
                    call_id: call.call_id,
                    namespace: None,
                    name: call.name,
                    arguments: call.arguments,
                });
            }
            ModelOutputItem::Reasoning {
                replay: Some(replay),
                ..
            } => {
                let stored = serde_json::to_string(&replay).unwrap();
                assert!(!format!("{replay:?}").contains("call-signature"));
                input.push(ModelInputItem::Reasoning {
                    replay: serde_json::from_str(&stored).unwrap(),
                });
            }
            _ => {}
        }
    }
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1]);
    // The host finished the second call first. The wire results still match
    // their original calls, including an omitted provider ID.
    input.push(ModelInputItem::ToolOutput {
        call_id: ids[1].clone(),
        output: ToolOutput::structured(json!("not found"), true),
    });
    input.push(ModelInputItem::ToolOutput {
        call_id: ids[0].clone(),
        output: ToolOutput {
            content: vec![ToolOutputContent::Image {
                mime_type: "image/jpeg".into(),
                data: "AQID".into(),
                detail: None,
            }],
            structured_content: None,
            is_error: false,
        },
    });
    let mut req = request(model, input);
    let json = body(&req);
    assert_eq!(json["contents"][1]["parts"], parts);
    let outputs = &json["contents"][2]["parts"];
    let first = &outputs[0]["functionResponse"];
    assert!(first.get("id").is_none());
    assert_eq!(first["name"], "read");
    assert_eq!(first["parts"][0]["inlineData"]["data"], "AQID");
    assert_eq!(
        first["response"]["output"]["$ref"],
        first["parts"][0]["inlineData"]["displayName"]
    );
    assert_eq!(
        outputs[1]["functionResponse"],
        json!({"id":"native-2","name":"read","response":{"error":"not found"}})
    );
    req.model = "gemini-3.1-pro".into();
    assert!(encode(&req, &route(&req.model))
        .unwrap_err()
        .to_string()
        .contains("thought signature"));
    req.input.push(ModelInputItem::user_text("New turn"));
    let json = body(&req);
    assert!(!json.to_string().contains("signature"));
    assert_eq!(json["contents"][1]["parts"][0]["text"], "I will look. ");
}

#[test]
fn a_limit_preserves_partial_text_without_completing_tools_and_early_eof_and_safety_are_distinct() {
    let mut decoder = Decoder::new(route("fixture"));
    let meta = ModelResponseMetadata::default();
    let partial = json!({"candidates":[{"content":{"parts":[{"text":"Partial"},{"functionCall":{"name":"do_work","args":{}}}]}}]});
    let mut events = decoder.decode(&partial, &meta).unwrap();
    events.extend(
        decoder
            .decode(
                &json!({"candidates":[{"finishReason":"MAX_TOKENS"}]}),
                &meta,
            )
            .unwrap(),
    );
    events.extend(decoder.finish().unwrap());
    assert!(
        matches!(events.last(),Some(ModelEvent::ResponseIncomplete{reason,..}) if reason=="MAX_TOKENS")
    );
    assert!(!events.iter().any(|e| matches!(
        e,
        ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::ToolCall { .. }
        }
    )));
    let mut decoder = Decoder::new(route("fixture"));
    decoder.decode(&partial, &meta).unwrap();
    assert!(matches!(
        decoder.finish(),
        Err(crate::ModelError::StreamInterrupted(_))
    ));
    let mut decoder = Decoder::new(route("fixture"));
    assert!(
        matches!(decoder.decode(&json!({"promptFeedback":{"blockReason":"SAFETY"}}),&meta),Err(crate::ModelError::Remote{code:Some(reason),..}) if reason=="SAFETY")
    );
}

#[test]
fn unrelated_bad_parts_leave_text_and_the_other_call_usable() {
    let mut decoder = Decoder::new(route("fixture"));
    let mut events = decoder
        .decode(
            &json!({"candidates":[{"content":{"parts":[
        {"functionCall":{"args":{}}},
        {"unknownField":true},
        {"text":"Still here"},
        {"functionCall":{"name":"list","args":"broken"}}
    ]},"finishReason":"STOP"}]}),
            &Default::default(),
        )
        .unwrap();
    events.extend(decoder.finish().unwrap());
    assert!(events
        .iter()
        .any(|e| matches!(e, ModelEvent::AssistantTextDelta {delta,..} if delta=="Still here")));
    let calls: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            ModelEvent::OutputItemCompleted {
                item: ModelOutputItem::ToolCall { call },
            } => Some(call),
            _ => None,
        })
        .collect();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].arguments, json!({}));
    assert!(calls[0].argument_error.is_some());
}

#[test]
fn gemini_25_uses_a_budget_and_places_tool_images_after_all_parallel_results() {
    let mut req = request(
        "models/gemini-2.5-flash",
        vec![ModelInputItem::user_text("inspect both")],
    );
    for id in ["a", "b"] {
        req.input.push(ModelInputItem::ToolCall {
            call_id: id.into(),
            namespace: Some("local".into()),
            name: "read".into(),
            arguments: json!({}),
        });
    }
    req.input.push(ModelInputItem::ToolOutput {
        call_id: "a".into(),
        output: ToolOutput {
            content: vec![ToolOutputContent::Image {
                mime_type: "image/png".into(),
                data: "AQID".into(),
                detail: None,
            }],
            structured_content: None,
            is_error: false,
        },
    });
    req.input.push(ModelInputItem::ToolOutput {
        call_id: "b".into(),
        output: ToolOutput::structured(json!("second"), false),
    });
    req.reasoning = Some(crate::ModelReasoningConfig {
        effort: crate::ModelReasoningEffort::Low,
        max_output_tokens: None,
    });
    let value = body(&req);
    assert_eq!(
        value["generationConfig"]["thinkingConfig"]["thinkingBudget"],
        1024
    );
    let parts = &value["contents"][2]["parts"];
    assert_eq!(parts[0]["functionResponse"]["name"], "local__read");
    assert!(parts[0]["functionResponse"].get("parts").is_none());
    assert_eq!(parts[1]["functionResponse"]["response"]["output"], "second");
    assert_eq!(parts[2]["inlineData"]["data"], "AQID");
}

#[test]
fn compressed_text_and_remaining_parallel_calls_keep_their_projected_order() {
    let model = "gemini-3-flash";
    let mut input = vec![
        ModelInputItem::user_text("inspect"),
        ModelInputItem::AssistantMessage {
            text: "[m1] Visible text".into(),
            phase: None,
        },
    ];
    for (id, signature) in [("first", Some("required")), ("second", None)] {
        input.push(ModelInputItem::ToolCall {
            call_id: id.into(),
            namespace: None,
            name: "read".into(),
            arguments: json!({}),
        });
        input.push(ModelInputItem::Reasoning {
            replay: crate::ModelReasoningReplay::GeminiGenerate {
                route: route(model),
                parts: vec![crate::GeminiReplayPart::Call {
                    call_id: id.into(),
                    provider_id: Some(id.into()),
                    signature: signature.map(str::to_owned),
                }],
            },
        });
    }
    let mut req = request(model, input);
    let json = body(&req);
    assert_eq!(
        json["contents"][1]["parts"],
        json!([
            {"text":"[m1] Visible text"},
            {"functionCall":{"name":"read","args":{},"id":"first"},"thoughtSignature":"required"},
            {"functionCall":{"name":"read","args":{},"id":"second"}}
        ])
    );
    req.input.drain(2..4);
    assert!(encode(&req, &route(model))
        .unwrap_err()
        .to_string()
        .contains("thought signature"));
}
