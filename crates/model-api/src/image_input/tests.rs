use hanafubuki_agent_tool_contracts::{ToolOutput, ToolOutputContent};
use serde_json::json;

use super::*;
use crate::{ModelImageDetail, ModelToolChoice};

fn request(input: Vec<ModelInputItem>) -> ModelRequest {
    ModelRequest {
        model: "model".into(),
        instructions: "instructions".into(),
        input,
        tools: Vec::new(),
        tool_choice: ModelToolChoice::Auto,
        parallel_tool_calls: false,
        reasoning: None,
        output_format: None,
    }
}

#[test]
fn sanitizes_user_tool_and_embedded_images_without_mutating_source() {
    let original = request(vec![
        ModelInputItem::UserMessage {
            content: vec![
                ModelUserContent::InputText {
                    text: "before".into(),
                },
                ModelUserContent::InputImage {
                    mime_type: "image/png".into(),
                    data: vec![1, 2, 3],
                    detail: ModelImageDetail::High,
                },
                ModelUserContent::InputText {
                    text: "after".into(),
                },
            ],
        },
        ModelInputItem::ToolOutput {
            call_id: "call-1".into(),
            output: ToolOutput {
                content: vec![
                    ToolOutputContent::Text {
                        text: "tool".into(),
                    },
                    ToolOutputContent::Image {
                        mime_type: "image/png".into(),
                        data: "AQID".into(),
                        detail: None,
                    },
                    ToolOutputContent::EmbeddedResource {
                        uri: "https://example.test/image".into(),
                        mime_type: Some("IMAGE/jpeg".into()),
                        text: None,
                        data: Some("AQID".into()),
                    },
                    ToolOutputContent::EmbeddedResource {
                        uri: "https://example.test/document".into(),
                        mime_type: Some("application/octet-stream".into()),
                        text: None,
                        data: Some("AQID".into()),
                    },
                ],
                structured_content: Some(json!({"kept": true})),
                is_error: true,
            },
        },
    ]);
    let (sanitized, report) = sanitize_images_for_unsupported_input(&original);
    assert_eq!(report.replaced_user_images, 1);
    assert_eq!(report.replaced_tool_images, 1);
    assert_eq!(report.replaced_embedded_images, 1);
    assert_eq!(report.total_replaced(), 3);
    assert!(request_contains_images(&original));
    assert!(!request_contains_images(&sanitized));
    assert!(matches!(
        &original.input[0],
        ModelInputItem::UserMessage { content }
            if matches!(content[1], ModelUserContent::InputImage { .. })
    ));
    assert!(matches!(
        &sanitized.input[1],
        ModelInputItem::ToolOutput { call_id, output }
            if call_id == "call-1"
                && output.is_error
                && output.structured_content == Some(json!({"kept": true}))
                && matches!(&output.content[0], ToolOutputContent::Text { text } if text == "tool")
                && matches!(&output.content[1], ToolOutputContent::Text { text } if text == IMAGE_INPUT_OMITTED_PLACEHOLDER)
                && matches!(&output.content[2], ToolOutputContent::Text { text } if text == IMAGE_INPUT_OMITTED_PLACEHOLDER)
                && matches!(&output.content[3], ToolOutputContent::EmbeddedResource { mime_type: Some(mime_type), data: Some(data), .. }
                    if mime_type == "application/octet-stream" && data == "AQID")
    ));
}

#[test]
fn no_images_produces_zero_report_and_equal_request() {
    let original = request(vec![ModelInputItem::user_text("hello")]);
    let (sanitized, report) = sanitize_images_for_unsupported_input(&original);
    assert_eq!(sanitized, original);
    assert_eq!(report.total_replaced(), 0);
}

#[test]
fn classifier_accepts_explicit_rejections_and_rejects_unrelated_failures() {
    for (status, message) in [
        (Some(400), "Model only supports text input"),
        (Some(415), "unsupported image content"),
        (Some(422), "vision input cannot be processed"),
        (Some(501), "multimodal attachment is not supported"),
        (
            None,
            "this endpoint is text-only and cannot handle image input",
        ),
    ] {
        assert!(matches!(
            classify_remote_model_error(status, None, message.into(), None),
            ModelError::UnsupportedImageInput { .. }
        ));
    }
    assert!(matches!(
        classify_remote_model_error(
            Some(400),
            Some("unsupported_image".into()),
            String::new(),
            None
        ),
        ModelError::UnsupportedImageInput { .. }
    ));
    assert!(matches!(
        classify_remote_model_error(Some(400), None, String::new(), None),
        ModelError::Remote { .. }
    ));
    for (status, message) in [
        (Some(400), "The image data is invalid"),
        (Some(400), "image is too large"),
        (Some(401), "image input is unsupported"),
        (Some(429), "image service is rate limited"),
        (Some(500), "image input is unsupported"),
    ] {
        assert!(matches!(
            classify_remote_model_error(status, None, message.into(), None),
            ModelError::Remote { .. }
        ));
    }
    for (status, code, message) in [
        (Some(400), Some("context_length_exceeded"), "too large"),
        (Some(413), None, "maximum context length was exceeded"),
        (None, Some("prompt_too_long"), "request failed"),
    ] {
        assert!(matches!(
            classify_remote_model_error(status, code.map(str::to_string), message.into(), None),
            ModelError::ContextWindowExceeded { .. }
        ));
    }
}
