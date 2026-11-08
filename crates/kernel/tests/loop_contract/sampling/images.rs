use super::*;

#[tokio::test]
async fn large_view_image_output_does_not_trigger_compaction_from_base64_size() {
    use base64::Engine as _;

    let first_request = image_request(vec![ModelInputItem::user_text("Inspect the image")]);
    let raw_image = vec![7u8; 3 * 1024 * 1024];
    let image_output = hanafubuki_agent_tool_contracts::ToolOutput {
        content: vec![hanafubuki_agent_tool_contracts::ToolOutputContent::Image {
            mime_type: "image/png".to_string(),
            data: base64::engine::general_purpose::STANDARD.encode(&raw_image),
            detail: Some(hanafubuki_agent_tool_contracts::ToolOutputImageDetail::High),
        }],
        structured_content: None,
        is_error: false,
    };
    let second_request = image_request(vec![
        ModelInputItem::user_text("Inspect the image"),
        ModelInputItem::ToolCall {
            call_id: "call-large-image".to_string(),
            namespace: None,
            name: "view_image".to_string(),
            arguments: json!({"path": "large-fixture.png"}),
        },
        ModelInputItem::ToolOutput {
            call_id: "call-large-image".to_string(),
            output: image_output,
        },
    ]);
    let tool_events = vec![
        Ok(ModelEvent::ResponseStarted {
            response_id: "response-large-image".to_string(),
            metadata: ModelResponseMetadata::default(),
        }),
        Ok(ModelEvent::OutputItemAdded {
            item: ModelOutputItem::ToolCall {
                call: ModelToolCall {
                    item_id: "item-large-image".to_string(),
                    call_id: "call-large-image".to_string(),
                    namespace: None,
                    name: "view_image".to_string(),
                    arguments: json!({"path": "large-fixture.png"}),
                    argument_error: None,
                },
            },
        }),
        Ok(ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::ToolCall {
                call: ModelToolCall {
                    item_id: "item-large-image".to_string(),
                    call_id: "call-large-image".to_string(),
                    namespace: None,
                    name: "view_image".to_string(),
                    arguments: json!({"path": "large-fixture.png"}),
                    argument_error: None,
                },
            },
        }),
        Ok(ModelEvent::ResponseCompleted {
            response_id: "response-large-image".to_string(),
            usage: Some(ModelUsage {
                input_tokens: 5_607,
                output_tokens: 109,
            }),
            end_turn: Some(false),
        }),
    ];
    let model = FakeModelTransport::new([
        FakeModelExchange::new(first_request.clone(), tool_events),
        FakeModelExchange::new(
            second_request.clone(),
            assistant_events_with_usage(
                "The image is visible.",
                Some(ModelUsage {
                    input_tokens: 6_000,
                    output_tokens: 20,
                }),
            ),
        ),
    ]);
    let prepared = hanafubuki_agent_kernel::PreparedAgentImage {
        bytes: raw_image,
        mime_type: "image/png".to_string(),
        width: 2_048,
        height: 2_048,
        display_name: "large-fixture.png".to_string(),
        detail: hanafubuki_agent_kernel::AgentImageDetail::High,
    };
    let tools = image_tool_port(prepared).await;
    let service = AgentService::new(
        Arc::new(ImageCapableModel {
            inner: model.clone(),
        }),
        tools.registries(),
    );
    let mut session_config = config();
    session_config.context_window_tokens = Some(1_250_000);
    session_config.auto_compact_threshold_percent = 80;
    let session = service.create_session(session_config).unwrap();

    let outcome = service
        .start_turn(session.id, "Inspect the image")
        .await
        .unwrap();

    assert_eq!(outcome.final_answer, "The image is visible.");
    assert_eq!(model.requests(), vec![first_request, second_request]);
    assert!(service.snapshot(session.id).unwrap().compaction.is_none());
}

#[tokio::test]
async fn view_image_output_is_preserved_but_retried_as_text_when_images_are_rejected() {
    use base64::Engine as _;

    let first_request = image_request(vec![ModelInputItem::user_text("Inspect the image")]);
    let image_output = hanafubuki_agent_tool_contracts::ToolOutput {
        content: vec![hanafubuki_agent_tool_contracts::ToolOutputContent::Image {
            mime_type: "image/png".to_string(),
            data: base64::engine::general_purpose::STANDARD.encode([1, 2, 3]),
            detail: Some(hanafubuki_agent_tool_contracts::ToolOutputImageDetail::High),
        }],
        structured_content: None,
        is_error: false,
    };
    let second_request = image_request(vec![
        ModelInputItem::user_text("Inspect the image"),
        ModelInputItem::ToolCall {
            call_id: "call-view-image".to_string(),
            namespace: None,
            name: "view_image".to_string(),
            arguments: json!({"path": "fixture.png"}),
        },
        ModelInputItem::ToolOutput {
            call_id: "call-view-image".to_string(),
            output: image_output,
        },
    ]);
    let sanitized_second_request =
        hanafubuki_agent_model_api::sanitize_images_for_unsupported_input(&second_request).0;
    let later_request = request(vec![ModelInputItem::user_text("follow up")]);
    let tool_events = vec![
        Ok(ModelEvent::ResponseStarted {
            response_id: "response-view-image".to_string(),
            metadata: ModelResponseMetadata::default(),
        }),
        Ok(ModelEvent::OutputItemAdded {
            item: ModelOutputItem::ToolCall {
                call: ModelToolCall {
                    item_id: "item-view-image".to_string(),
                    call_id: "call-view-image".to_string(),
                    namespace: None,
                    name: "view_image".to_string(),
                    arguments: json!({"path": "fixture.png"}),
                    argument_error: None,
                },
            },
        }),
        Ok(ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::ToolCall {
                call: ModelToolCall {
                    item_id: "item-view-image".to_string(),
                    call_id: "call-view-image".to_string(),
                    namespace: None,
                    name: "view_image".to_string(),
                    arguments: json!({"path": "fixture.png"}),
                    argument_error: None,
                },
            },
        }),
        Ok(ModelEvent::ResponseCompleted {
            response_id: "response-view-image".to_string(),
            usage: None,
            end_turn: Some(false),
        }),
    ];
    let model = FakeModelTransport::new([
        FakeModelExchange::new(first_request.clone(), tool_events),
        FakeModelExchange::new(
            second_request.clone(),
            vec![Err(ModelError::UnsupportedImageInput {
                status: Some(400),
                code: Some("unsupported_image".into()),
                message: "model only supports text input".into(),
                request_id: Some("request-image".into()),
            })],
        ),
        FakeModelExchange::new(
            sanitized_second_request.clone(),
            vec![Err(ModelError::StreamInterrupted(
                "fixture fallback stream interruption".into(),
            ))],
        ),
        FakeModelExchange::new(
            sanitized_second_request.clone(),
            deterministic_assistant_events("The image is visible."),
        ),
        FakeModelExchange::new(
            later_request.clone(),
            deterministic_assistant_events("Text-only follow up."),
        ),
    ]);
    let prepared = hanafubuki_agent_kernel::PreparedAgentImage {
        bytes: vec![1, 2, 3],
        mime_type: "image/png".to_string(),
        width: 1,
        height: 1,
        display_name: "fixture.png".to_string(),
        detail: hanafubuki_agent_kernel::AgentImageDetail::High,
    };
    let tools = image_tool_port(prepared).await;
    let service = AgentService::new(
        Arc::new(ImageCapableModel {
            inner: model.clone(),
        }),
        tools.registries(),
    );
    let session = service.create_session(config()).unwrap();

    let outcome = service
        .start_turn(session.id, "Inspect the image".to_string())
        .await
        .unwrap();

    assert_eq!(outcome.final_answer, "The image is visible.");
    assert_eq!(
        model.requests(),
        vec![
            first_request,
            second_request,
            sanitized_second_request.clone(),
            sanitized_second_request,
        ]
    );
    let snapshot = test_session_view(&service, session.id).unwrap();
    assert!(!snapshot.input_capabilities.image_input);
    assert!(service
        .events_since(session.id, None)
        .unwrap()
        .events
        .iter()
        .any(|event| matches!(
            event.kind,
            AgentEventKind::ModelRetryScheduled {
                stage: hanafubuki_agent_kernel::AgentModelRetryStage::ImageInputFallback,
                current_retry: 1,
                maximum_retries: 1,
                delay_millis: 0,
                ..
            }
        )));
    assert!(service
        .events_since(session.id, None)
        .unwrap()
        .events
        .iter()
        .any(|event| matches!(
            event.kind,
            AgentEventKind::ModelRetryScheduled {
                stage: hanafubuki_agent_kernel::AgentModelRetryStage::ResponseStream,
                ..
            }
        )));
    assert_eq!(
        service
            .events_since(session.id, None)
            .unwrap()
            .events
            .iter()
            .filter(|event| matches!(event.kind, AgentEventKind::SamplingStarted { .. }))
            .count(),
        2,
        "content fallback and stream retry must not emit duplicate sampling starts"
    );
    assert!(snapshot.model_context.iter().any(|item| matches!(
        item,
        ConversationItem::ToolOutput { output, images, .. }
            if output.content.is_empty()
                && images.len() == 1
                && images[0].display_name == "fixture.png"
    )));
    let later_session = service.create_session(config()).unwrap();
    assert!(
        !later_session.input_capabilities.image_input,
        "a new session on the same exact route should reuse the observed rejection"
    );
    let later_outcome = service
        .start_turn(later_session.id, "follow up")
        .await
        .unwrap();
    assert_eq!(later_outcome.final_answer, "Text-only follow up.");
    assert_eq!(model.requests().last(), Some(&later_request));
}

#[tokio::test]
async fn image_rejection_without_an_image_is_not_retried() {
    let expected = image_request(vec![ModelInputItem::user_text("plain text")]);
    let model = FakeModelTransport::new([FakeModelExchange::new(
        expected.clone(),
        vec![Err(ModelError::UnsupportedImageInput {
            status: Some(400),
            code: Some("unsupported_image".into()),
            message: "image input is not supported".into(),
            request_id: None,
        })],
    )]);
    let service = AgentService::new(
        Arc::new(ImageCapableModel {
            inner: model.clone(),
        }),
        image_tool_port(unused_image()).await.registries(),
    );
    let session = service.create_session(config()).unwrap();

    assert!(matches!(
        service.start_turn(session.id, "plain text").await,
        Err(AgentError::UnsupportedImageInput { .. })
    ));
    assert_eq!(model.requests(), vec![expected]);
    assert!(!service
        .events_since(session.id, None)
        .unwrap()
        .events
        .iter()
        .any(|event| matches!(
            event.kind,
            AgentEventKind::ModelRetryScheduled {
                stage: hanafubuki_agent_kernel::AgentModelRetryStage::ImageInputFallback,
                ..
            }
        )));
}

#[tokio::test]
async fn image_rejection_after_response_start_is_not_replayed() {
    let expected = direct_image_request("inspect");
    let model = FakeModelTransport::new([FakeModelExchange::new(
        expected.clone(),
        vec![
            Ok(ModelEvent::ResponseStarted {
                response_id: "response-started".to_string(),
                metadata: ModelResponseMetadata::default(),
            }),
            Err(ModelError::UnsupportedImageInput {
                status: Some(400),
                code: Some("unsupported_image".into()),
                message: "image input is not supported".into(),
                request_id: None,
            }),
        ],
    )]);
    let service = AgentService::new(
        Arc::new(ImageCapableModel {
            inner: model.clone(),
        }),
        image_tool_port(unused_image()).await.registries(),
    );
    let session = service.create_session(config()).unwrap();
    let input = direct_image_turn(&service, session.id, "inspect");

    assert!(matches!(
        service.start_turn(session.id, input).await,
        Err(AgentError::Model(_))
    ));
    assert_eq!(model.requests(), vec![expected]);
}

#[tokio::test]
async fn a_second_image_rejection_is_not_retried_again() {
    let expected = direct_image_request("inspect");
    let sanitized = hanafubuki_agent_model_api::sanitize_images_for_unsupported_input(&expected).0;
    let rejection = || {
        Err(ModelError::UnsupportedImageInput {
            status: Some(400),
            code: Some("unsupported_image".into()),
            message: "image input is not supported".into(),
            request_id: None,
        })
    };
    let model = FakeModelTransport::new([
        FakeModelExchange::new(expected.clone(), vec![rejection()]),
        FakeModelExchange::new(sanitized.clone(), vec![rejection()]),
    ]);
    let service = AgentService::new(
        Arc::new(ImageCapableModel {
            inner: model.clone(),
        }),
        image_tool_port(unused_image()).await.registries(),
    );
    let session = service.create_session(config()).unwrap();
    let input = direct_image_turn(&service, session.id, "inspect");

    assert!(matches!(
        service.start_turn(session.id, input).await,
        Err(AgentError::UnsupportedImageInput { .. })
    ));
    assert_eq!(model.requests(), vec![expected, sanitized]);
}
