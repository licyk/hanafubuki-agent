use super::*;
use base64::Engine as _;

#[test]
fn large_tool_result_image_is_estimated_as_media_instead_of_base64_text() {
    let raw = vec![7u8; 3 * 1024 * 1024];
    let encoded = base64::engine::general_purpose::STANDARD.encode(&raw);
    assert!(encoded.len() > 4_000_000);
    let request = ModelRequest {
        model: "fixture".to_string(),
        instructions: "inspect the image".to_string(),
        input: vec![ModelInputItem::ToolOutput {
            call_id: "call-image".to_string(),
            output: hanafubuki_agent_tool_contracts::ToolOutput {
                content: vec![hanafubuki_agent_tool_contracts::ToolOutputContent::Image {
                    mime_type: "image/png".to_string(),
                    data: encoded,
                    detail: Some(hanafubuki_agent_tool_contracts::ToolOutputImageDetail::High),
                }],
                structured_content: None,
                is_error: false,
            },
        }],
        tools: Vec::new(),
        tool_choice: hanafubuki_agent_model_api::ModelToolChoice::None,
        parallel_tool_calls: false,
        reasoning: None,
        output_format: None,
    };

    let estimated = estimate_model_request_tokens(&request).unwrap();

    assert!(estimated < 10_000, "media estimate was {estimated}");
}

#[test]
fn embedded_tool_result_image_uses_the_same_bounded_media_estimate() {
    let encoded = base64::engine::general_purpose::STANDARD.encode(vec![3u8; 1024 * 1024]);
    let request = ModelRequest {
        model: "fixture".to_string(),
        instructions: String::new(),
        input: vec![ModelInputItem::CustomToolOutput {
            call_id: "call-image".to_string(),
            output: hanafubuki_agent_tool_contracts::ToolOutput {
                content: vec![
                    hanafubuki_agent_tool_contracts::ToolOutputContent::EmbeddedResource {
                        uri: "https://example.test/image.png".to_string(),
                        mime_type: Some("image/png".to_string()),
                        text: None,
                        data: Some(encoded),
                    },
                ],
                structured_content: None,
                is_error: false,
            },
        }],
        tools: Vec::new(),
        tool_choice: hanafubuki_agent_model_api::ModelToolChoice::None,
        parallel_tool_calls: false,
        reasoning: None,
        output_format: None,
    };

    let estimated = estimate_model_request_tokens(&request).unwrap();

    assert!(estimated < 10_000, "media estimate was {estimated}");
}

/// A command's styles are kept in the history's bytes but never sent, so
/// they move neither the context estimate nor when compaction starts.
#[test]
fn display_metadata_is_not_in_the_context_estimate() {
    let styles = hanafubuki_agent_contracts::AgentStyledText {
        text_bytes: 4,
        spans: vec![hanafubuki_agent_contracts::AgentStyleSpan {
            start: 0,
            end: 3,
            style: hanafubuki_agent_contracts::AgentTextStyle {
                bold: true,
                ..Default::default()
            },
        }],
    };
    let output = |display_metadata| ConversationItem::ToolOutput {
        call_id: "call-1".to_string(),
        origin: Default::default(),
        output: hanafubuki_agent_tool_contracts::ToolOutput::structured(
            serde_json::json!({ "output": "red\n" }),
            false,
        ),
        truncation: None,
        images: Vec::new(),
        tool_search_results: Vec::new(),
        display_metadata,
    };
    let shell = |styled| ConversationItem::UserShellCommand {
        call_id: "user-shell-1".to_string(),
        command: "ls".to_string(),
        cwd: None,
        exit_code: Some(0),
        duration_millis: 1,
        output: "red\n".repeat(400),
        truncated: false,
        styled,
    };
    let styled = [
        output(hanafubuki_agent_contracts::AgentToolDisplayMetadata {
            styled_output: Some(styles.clone()),
        }),
        shell(Some(styles)),
    ];
    let plain = [output(Default::default()), shell(None)];
    for (styled, plain) in styled.iter().zip(&plain) {
        assert!(styled.retained_bytes() > plain.retained_bytes());
        assert_eq!(
            estimate::estimate_conversation_items_tokens([styled]).unwrap(),
            estimate::estimate_conversation_items_tokens([plain]).unwrap()
        );
    }
}

#[test]
fn context_window_projects_tool_outputs_without_mutating_canonical_values() {
    let first = hanafubuki_agent_tool_contracts::ToolOutput::structured(
        serde_json::json!({
            "text": format!("FIRST-{}-LAST", "花火🙂".repeat(2_000))
        }),
        true,
    );
    let second = hanafubuki_agent_tool_contracts::ToolOutput::structured(
        serde_json::json!({"text": format!("SECOND-{}-END", "y".repeat(8_000))}),
        false,
    );
    let canonical = vec![first.clone(), second.clone()];
    let mut input = vec![
        ModelInputItem::ToolOutput {
            call_id: "first".to_string(),
            output: first,
        },
        ModelInputItem::CustomToolOutput {
            call_id: "second".to_string(),
            output: second,
        },
    ];

    project_model_tool_outputs(&mut input, Some(2_048), 256 * 1024).unwrap();

    for (item, canonical) in input.iter().zip(canonical) {
        let projected = match item {
            ModelInputItem::ToolOutput { output, .. }
            | ModelInputItem::CustomToolOutput { output, .. } => output,
            _ => unreachable!(),
        };
        assert!(serde_json::to_vec(projected).unwrap().len() <= 1_024);
        assert_eq!(projected.is_error, canonical.is_error);
        let text = projected.legacy_text_projection().unwrap();
        assert!(text.contains("<hanafubuki-model-output-projected>"));
        assert!(!text.contains("<hanafubuki-tool-output-truncation>"));
        assert_ne!(projected, &canonical);
        assert!(serde_json::to_vec(&canonical).unwrap().len() > 1_024);
    }
}

#[test]
fn tool_omission_world_state_changes_only_for_replace_or_remove() {
    let mut state = BTreeMap::new();
    let omissions = vec![hanafubuki_agent_contracts::AgentToolOmission {
        contributor: "optional-mcp".into(),
        tool_name: Some("invalid_remote".into()),
        reason: "invalid schema".into(),
    }];
    let first_revision = crate::AgentToolCatalogRevision([1; 32]);
    let first = tool_omission_world_state_update(&mut state, first_revision, &omissions)
        .unwrap()
        .unwrap();
    assert!(matches!(first, ConversationItem::WorldStateUpdate { .. }));
    assert!(
        tool_omission_world_state_update(&mut state, first_revision, &omissions)
            .unwrap()
            .is_none()
    );

    let changed = tool_omission_world_state_update(
        &mut state,
        crate::AgentToolCatalogRevision([2; 32]),
        &omissions,
    )
    .unwrap()
    .unwrap();
    assert!(matches!(
        changed,
        ConversationItem::WorldStateUpdate { updates }
            if matches!(updates.as_slice(), [AgentWorldStateUpdate::Replace { .. }])
    ));
    let removed =
        tool_omission_world_state_update(&mut state, crate::AgentToolCatalogRevision([3; 32]), &[])
            .unwrap()
            .unwrap();
    assert!(matches!(
        removed,
        ConversationItem::WorldStateUpdate { updates }
            if matches!(updates.as_slice(), [AgentWorldStateUpdate::Remove { .. }])
    ));
    assert!(tool_omission_world_state_update(
        &mut state,
        crate::AgentToolCatalogRevision([4; 32]),
        &[],
    )
    .unwrap()
    .is_none());
}

/// Each run of adjacent sections is ordered by slot, then renderer order,
/// then kind; an item between two runs keeps them apart.
#[test]
fn every_run_of_sections_is_ordered_by_slot_order_and_kind_and_items_keep_their_place() {
    use hanafubuki_agent_contracts::ContextSlot;

    let section = |slot, order, kind: &str| PlacedInput::Section {
        position: (slot, order, kind.to_string()),
        item: ModelInputItem::ContextFragment {
            role: ModelContextRole::Developer,
            kind: kind.to_string(),
            content: String::new(),
        },
    };
    let placed = vec![
        section(ContextSlot::Memory, 100, "memory"),
        section(ContextSlot::Capabilities, 900, "zeta_board"),
        section(ContextSlot::Capabilities, 900, "alpha_board"),
        section(ContextSlot::Capabilities, 110, "catalog"),
        section(ContextSlot::WorldState, 710, "permission"),
        section(ContextSlot::WorldState, 700, "roots"),
        section(ContextSlot::Instructions, 700, "instructions"),
        PlacedInput::Item(ModelInputItem::user_text("hello")),
        section(ContextSlot::WorldState, 710, "later_permission"),
        section(ContextSlot::Instructions, 700, "later_instructions"),
    ];

    let order = ordered_input(placed)
        .into_iter()
        .map(|item| match item {
            ModelInputItem::ContextFragment { kind, .. } => kind,
            _ => "user".to_string(),
        })
        .collect::<Vec<_>>();
    // Equal slot and order fall back to the kind name, whatever order the
    // sections came in.
    assert_eq!(
        order,
        [
            "instructions",
            "roots",
            "permission",
            "catalog",
            "alpha_board",
            "zeta_board",
            "memory",
            "user",
            "later_instructions",
            "later_permission",
        ]
    );
}

/// A request's composition: each group is the estimate of its own
/// serialized part (images by their own rule), the tools are split by the
/// provider that planned them, the sections by kind in the order the
/// request shows them, and the groups add up to the request's estimate.
#[test]
fn a_request_is_composed_of_its_groups_and_they_add_up_to_its_estimate() {
    use hanafubuki_agent_contracts::estimate_tokens;
    use hanafubuki_agent_model_api::{ModelImageDetail, ModelUserContent};

    let tool = |provider: &str, core: crate::AgentCoreTool| {
        crate::tool_catalog::AgentPlannedTool::fixed(
            crate::AgentToolProviderId::parse(provider.to_string()).unwrap(),
            core.model_spec(),
            crate::AgentToolExecutionClass::Exclusive,
        )
        .unwrap()
    };
    let plan = AgentToolPlan::build(vec![
        tool("builtin", crate::AgentCoreTool::UpdatePlan),
        tool("builtin", crate::AgentCoreTool::NewContext),
        tool("mcp.fixture", crate::AgentCoreTool::RequestUserInput),
    ])
    .unwrap();
    let section = |kind: &str, content: &str| ModelInputItem::ContextFragment {
        role: ModelContextRole::Developer,
        kind: kind.to_string(),
        content: content.to_string(),
    };
    let user = ModelInputItem::UserMessage {
        content: vec![
            ModelUserContent::InputText {
                text: "Look at this.".to_string(),
            },
            ModelUserContent::InputImage {
                mime_type: "image/png".to_string(),
                data: vec![1u8; 300 * 1024],
                detail: ModelImageDetail::High,
            },
        ],
    };
    let input = vec![
        section("environment", &"cwd ".repeat(40)),
        section("workspace_instructions", &"Be brief. ".repeat(30)),
        user,
        ModelInputItem::Reasoning {
            replay: hanafubuki_agent_model_api::ModelReasoningReplay::ReasoningContent {
                text: "Thinking it over.".to_string(),
            },
        },
        ModelInputItem::AssistantMessage {
            phase: None,
            text: "Here it is.".repeat(20),
        },
        ModelInputItem::ToolCall {
            call_id: "call-1".to_string(),
            namespace: None,
            name: "update_plan".to_string(),
            arguments: serde_json::json!({"plan": []}),
        },
        ModelInputItem::ToolOutput {
            call_id: "call-1".to_string(),
            output: hanafubuki_agent_tool_contracts::ToolOutput::json(
                serde_json::json!({"updated": true}),
                false,
            )
            .unwrap(),
        },
        ModelInputItem::ContinuationContext {
            provider_id: "goals".to_string(),
            hook_id: "continue".to_string(),
            generation: [0; 32],
            content: "Keep going.".to_string(),
        },
        ModelInputItem::CompactionSummary {
            summary: "Earlier, the user asked for a plan.".repeat(5),
            source_revision: 3,
            compacted_item_count: 7,
        },
        section("environment", "a later change"),
        ModelInputItem::user_text("Next?"),
    ];
    let request = ModelRequest {
        model: "fixture".to_string(),
        instructions: "You are the fixture. ".repeat(50),
        input,
        tools: plan.model_specs(),
        tool_choice: hanafubuki_agent_model_api::ModelToolChoice::Auto,
        parallel_tool_calls: true,
        reasoning: None,
        output_format: None,
    };

    let composition = compose_request(&request, &plan).unwrap();

    let (stripped, _) = estimate::without_media(&request).unwrap();
    let items = |indexes: &[usize]| {
        estimate_tokens(
            indexes
                .iter()
                .map(|index| estimate::serialized_len(&stripped.input[*index]).unwrap())
                .sum(),
        )
    };
    // The plan lists its tools by name: new_context, request_user_input,
    // update_plan.
    let tools = |names: &[&str]| {
        estimate_tokens(
            request
                .tools
                .iter()
                .filter(|spec| names.contains(&spec.name().unwrap().flattened_alias().as_str()))
                .map(|spec| estimate::serialized_len(spec).unwrap())
                .sum(),
        )
    };
    assert_eq!(
        composition.instructions_tokens,
        estimate_tokens(estimate::serialized_len(&request.instructions).unwrap())
    );
    assert_eq!(
        composition.tools,
        vec![
            crate::AgentContextTools {
                provider_id: Some("builtin".to_string()),
                count: 2,
                tokens: tools(&["new_context", "update_plan"]),
            },
            crate::AgentContextTools {
                provider_id: Some("mcp.fixture".to_string()),
                count: 1,
                tokens: tools(&["request_user_input"]),
            },
        ]
    );
    assert_eq!(
        composition.sections,
        vec![
            crate::AgentContextSection {
                kind: "environment".to_string(),
                tokens: items(&[0, 9]),
            },
            crate::AgentContextSection {
                kind: "workspace_instructions".to_string(),
                tokens: items(&[1]),
            },
        ]
    );
    assert_eq!(composition.user_messages_tokens, items(&[2, 10]));
    assert_eq!(composition.reasoning_tokens, items(&[3]));
    assert_eq!(composition.assistant_messages_tokens, items(&[4]));
    assert_eq!(composition.tool_calls_tokens, items(&[5, 6]));
    assert_eq!(composition.continuation_tokens, items(&[7]));
    assert_eq!(composition.compaction_summaries_tokens, items(&[8]));
    // A 300 KiB image is two tiles at high detail.
    assert_eq!(
        (composition.images, composition.image_tokens),
        (1, 85 + 2 * 170)
    );
    assert!(composition.other_tokens > 0);

    let estimate = estimate_model_request_tokens(&request).unwrap();
    assert_eq!(composition.total_tokens, estimate);
    let sum = composition.instructions_tokens
        + composition
            .tools
            .iter()
            .map(|tools| tools.tokens)
            .sum::<u64>()
        + composition
            .sections
            .iter()
            .map(|section| section.tokens)
            .sum::<u64>()
        + composition.user_messages_tokens
        + composition.assistant_messages_tokens
        + composition.reasoning_tokens
        + composition.tool_calls_tokens
        + composition.continuation_tokens
        + composition.compaction_summaries_tokens
        + composition.image_tokens
        + composition.other_tokens;
    assert_eq!(sum, estimate);
}
