use base64::Engine as _;
use serde_json::json;

use super::*;

fn bytes(output: &ToolOutput) -> usize {
    serde_json::to_vec(output).unwrap().len()
}

#[test]
fn bounded_output_is_unchanged_when_it_fits() {
    let output = ToolOutput::structured(json!({"status": "ok"}), false);
    let (bounded, truncation) = bound_tool_output(output.clone(), 1024).unwrap();
    assert_eq!(bounded, output);
    assert!(truncation.is_none());
}

#[test]
fn utf8_text_uses_head_and_tail_and_preserves_a_fitting_image() {
    let text = format!("HEAD-{}-TAIL", "花火🙂".repeat(2_000));
    let output = ToolOutput {
        content: vec![
            ToolOutputContent::Text { text },
            ToolOutputContent::Image {
                mime_type: "image/png".to_string(),
                data: base64::engine::general_purpose::STANDARD.encode([0_u8; 32]),
                detail: None,
            },
        ],
        structured_content: None,
        is_error: false,
    };
    let original_bytes = bytes(&output);
    let (bounded, truncation) = bound_tool_output(output, 1024).unwrap();
    let truncation = truncation.unwrap();

    assert!(bytes(&bounded) <= 1024);
    assert_eq!(truncation.original_bytes, original_bytes as u64);
    assert_eq!(truncation.retained_bytes, bytes(&bounded) as u64);
    assert!(truncation
        .omitted_content_types
        .iter()
        .any(|kind| kind == "text"));
    assert!(bounded
        .content
        .iter()
        .any(|item| matches!(item, ToolOutputContent::Image { .. })));
    let preview = bounded
        .content
        .iter()
        .find_map(|item| match item {
            ToolOutputContent::Text { text }
                if !text.starts_with("<hanafubuki-tool-output-truncation>") =>
            {
                Some(text)
            }
            _ => None,
        })
        .unwrap();
    assert!(preview.starts_with("HEAD-"));
    assert!(preview.ends_with("-TAIL"));
    assert!(preview.contains("bytes omitted"));
    assert!(std::str::from_utf8(preview.as_bytes()).is_ok());
}

#[test]
fn structured_output_becomes_an_explicit_preview_without_changing_error_state() {
    let output = ToolOutput::structured(
        json!({
            "status": "failed",
            "values": (0..4_000).collect::<Vec<_>>(),
            "tail": "LAST-VALUE"
        }),
        true,
    );
    let (bounded, truncation) = bound_tool_output(output, 768).unwrap();
    let truncation = truncation.unwrap();

    assert!(bounded.is_error);
    assert!(bounded.structured_content.is_none());
    assert!(bytes(&bounded) <= 768);
    assert!(truncation
        .omitted_content_types
        .iter()
        .any(|kind| kind == "structured"));
    assert!(truncation
        .omitted_content_types
        .iter()
        .any(|kind| kind == "structured_content"));
    let combined = bounded
        .content
        .iter()
        .filter_map(|item| match item {
            ToolOutputContent::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(combined.contains("structured JSON preview"));
    assert!(combined.contains("LAST-VALUE"));
    assert!(combined.contains("<hanafubuki-tool-output-truncation>"));
}

#[test]
fn item_limit_reserves_space_for_the_omission_marker() {
    let output = ToolOutput {
        content: (0..MAX_TOOL_OUTPUT_ITEMS)
            .map(|index| ToolOutputContent::Text {
                text: format!("{index}:{}", "x".repeat(128)),
            })
            .collect(),
        structured_content: None,
        is_error: false,
    };
    let (bounded, truncation) = bound_tool_output(output, 1024).unwrap();
    assert!(truncation.is_some());
    assert!(bounded.content.len() <= MAX_TOOL_OUTPUT_ITEMS);
    assert!(bytes(&bounded) <= 1024);
    bounded.validate().unwrap();
}

#[test]
fn invalid_runtime_output_becomes_an_outcome_unknown_error() {
    let invalid = ToolOutput {
        content: vec![ToolOutputContent::Image {
            mime_type: "text/plain".to_string(),
            data: base64::engine::general_purpose::STANDARD.encode(b"already executed"),
            detail: None,
        }],
        structured_content: None,
        is_error: false,
    };
    let (canonical, truncation) = canonicalize_tool_output(invalid, 512);
    assert!(canonical.is_error);
    assert!(truncation.is_none());
    assert!(bytes(&canonical) <= 512);
    assert!(canonical
        .legacy_text_projection()
        .unwrap()
        .contains("outcome_unknown"));
    assert!(canonical
        .legacy_text_projection()
        .unwrap()
        .contains("Do not automatically replay"));
}

#[test]
fn envelope_identity_and_deferred_selection_survive_canonical_truncation() {
    let selection = crate::AgentToolSelection {
        provider_id: crate::AgentToolProviderId::parse("mcp.fixture".to_string()).unwrap(),
        tool_id: crate::AgentToolId::parse("large_output".to_string()).unwrap(),
        generation: crate::AgentToolGeneration([7; 32]),
    };
    let skill_context = crate::ActivatedSkillContext {
        skill: hanafubuki_agent_skills::SkillId::new(
            hanafubuki_agent_skills::SkillScope::User,
            "large-output-skill",
        )
        .unwrap(),
        name: "large-output-skill".to_string(),
        content: "Use the exact output contract.".to_string(),
    };
    let output = crate::AgentToolOutput {
        output: ToolOutput::structured(
            json!({"value": format!("HEAD-{}-TAIL", "x".repeat(8_000))}),
            true,
        ),
        prepared_images: Vec::new(),
        selected_tools: vec![selection.clone()],
        tool_search_results: Vec::new(),
        skill_context: Some(skill_context.clone()),
        display_metadata: Default::default(),
    };

    let (canonical, truncation) = canonicalize_agent_tool_output(output, 512);

    assert!(canonical.output.is_error);
    assert!(truncation.is_some());
    assert_eq!(canonical.selected_tools, vec![selection]);
    assert_eq!(canonical.skill_context, Some(skill_context));
    assert!(canonical.tool_search_results.is_empty());
}
