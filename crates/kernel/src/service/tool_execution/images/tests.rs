use super::*;
use crate::conversation::AgentHistoryStore;

#[test]
fn inline_tool_images_replay_in_place_and_same_sized_images_remain_distinct() {
    let store = Arc::new(crate::EphemeralAgentHistoryStore::default());
    let service = AgentService::without_default_model_with_history_store(
        crate::AgentKernelRegistries::default(),
        store.clone(),
    );
    // The image store needs an existing owner, but no turn or provider request.
    let session = service
        .create_model_session(
            crate::AgentSessionConfig {
                provider_id: "test".into(),
                provider_protocol: hanafubuki_agent_model_api::ModelApiProtocol::Responses,
                model: "test".into(),
                instructions: "test".into(),
                mode: crate::AgentSessionMode::Default,
                reasoning_effort: None,
                context_window_tokens: None,
                input_capabilities: Default::default(),
                auto_compaction_enabled: true,
                auto_compact_threshold_percent: 80,
                compaction_capability:
                    hanafubuki_agent_model_api::ModelCompactionCapability::LocalSummary,
                sampling_retry_policy: Default::default(),
                context_window_source: None,
                compaction_strategy: Default::default(),
                scope: Default::default(),
                configured_facts: Default::default(),
                role_instructions: None,
            },
            hanafubuki_agent_model_api::ModelApiProtocol::Responses,
            "http://127.0.0.1:1/v1",
            None,
            Vec::new(),
            Default::default(),
            hanafubuki_agent_model_api::ModelCompactionCapability::LocalSummary,
            Default::default(),
        )
        .unwrap();
    let first = vec![17; 512];
    let second = vec![23; 512];
    let original: ToolOutput = serde_json::from_value(serde_json::json!({
        "content":[
            {"type":"text","text":"before"},
            {"type":"image","mimeType":"image/webp","data":base64::engine::general_purpose::STANDARD.encode(&first),"detail":"original"},
            {"type":"embedded_resource","uri":"mcp://test/image","mimeType":"image/png","text":"caption","data":base64::engine::general_purpose::STANDARD.encode(&second)},
            {"type":"audio","mimeType":"audio/wav","data":"AA=="},
            {"type":"text","text":"after"}
        ],"structuredContent":null,"isError":false
    })).unwrap();
    original.validate().unwrap();
    let mut stored = original.clone();
    let images = service
        .stage_tool_output_images(session.id, &mut stored)
        .unwrap();
    assert_ne!(images[0].attachment_id, images[1].attachment_id);
    assert_eq!(store.load_image_attachment(&images[0]).unwrap(), first);
    assert_eq!(store.load_image_attachment(&images[1]).unwrap(), second);
    assert!(serde_json::to_vec(&stored).unwrap().len() < 512);
    let repeated = service
        .stage_tool_output_images(session.id, &mut original.clone())
        .unwrap();
    assert_eq!(repeated, images);
    let display = service
        .present_tool_call(
            None,
            "image_fixture",
            &serde_json::json!({}),
            Some((&stored, &Default::default())),
        )
        .with_image_attachments(&images);
    let Some(hanafubuki_agent_contracts::AgentToolDisplay::Content { blocks }) = display.display
    else {
        panic!("expected content display")
    };
    assert!(matches!(
        blocks[1],
        hanafubuki_agent_contracts::AgentOutputBlock::Image { bytes: 512, .. }
    ));
    assert!(matches!(
        blocks[2],
        hanafubuki_agent_contracts::AgentOutputBlock::Resource {
            bytes: Some(512),
            ..
        }
    ));
    for origin in [
        hanafubuki_agent_model_api::ModelToolCallOrigin::Function,
        hanafubuki_agent_model_api::ModelToolCallOrigin::Freeform,
    ] {
        let item = ConversationItem::ToolOutput {
            call_id: "call".into(),
            origin,
            output: stored.clone(),
            images: images.clone(),
            truncation: None,
            tool_search_results: Vec::new(),
            display_metadata: Default::default(),
        };
        let rebuilt = item
            .as_model_input_resolved(|image| {
                store
                    .load_image_attachment(image)
                    .map_err(history_store_error)
            })
            .unwrap()
            .unwrap();
        match rebuilt {
            hanafubuki_agent_model_api::ModelInputItem::ToolOutput { output, .. }
            | hanafubuki_agent_model_api::ModelInputItem::CustomToolOutput { output, .. } => {
                assert_eq!(output, original)
            }
            _ => panic!("expected tool output"),
        }
    }
}
