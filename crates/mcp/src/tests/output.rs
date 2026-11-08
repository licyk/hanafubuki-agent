use hanafubuki_agent_tool_contracts::{
    ToolOutputContent, MAX_TOOL_OUTPUT_BYTES, MAX_TOOL_OUTPUT_ITEMS,
};
use rmcp::model::ReadResourceResult;

use super::*;

#[test]
fn tool_output_preserves_text_image_audio_resource_and_error_state() {
    let mut result = CallToolResult::success(vec![
        Content::text("hello"),
        Content::image("AA==", "image/png"),
        Content::resource(ResourceContents::text("body", "mcp://local/doc")),
    ]);
    result.structured_content = Some(json!({"ok": true}));
    result.is_error = Some(true);
    let output = convert_tool_output(result).unwrap();
    assert!(output.is_error);
    assert_eq!(output.content.len(), 3);
    assert_eq!(output.structured_content, Some(json!({"ok": true})));
}

/// Credential values are stored verbatim in settings, but a resolved
/// connection still refuses to print them, and a changed value must invalidate
/// the connection that was built from the previous one.

#[test]
fn malformed_binary_or_mime_output_is_rejected() {
    assert_eq!(
        convert_tool_output(CallToolResult::success(vec![Content::image(
            "not-base64",
            "image/png"
        )]))
        .unwrap_err(),
        McpError::LimitExceeded
    );
    assert_eq!(
        convert_tool_output(CallToolResult::success(vec![Content::image(
            "AA==",
            "text/plain"
        )]))
        .unwrap_err(),
        McpError::LimitExceeded
    );
}

#[test]
fn an_undecodable_content_block_becomes_a_placeholder_and_the_rest_is_kept() {
    let result = crate::output::decode_call_tool_result_leniently(json!({
        "content": [
            {"type": "text", "text": "kept"},
            {"type": "video", "data": "AAAA", "secret": "never shown"},
            {"type": "image", "mimeType": "image/png"},
            {"type": format!("x{}", "y".repeat(200))},
            {"no": "type"}
        ],
        "isError": false,
        "unknownField": true
    }))
    .unwrap();
    let output = convert_tool_output(result).unwrap();
    let texts = output
        .content
        .iter()
        .map(|item| match item {
            ToolOutputContent::Text { text } => text.as_str(),
            other => panic!("unexpected item {other:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(texts[0], "kept");
    assert_eq!(texts[1], "[unsupported MCP content: video]");
    assert_eq!(texts[2], "[unsupported MCP content: image]");
    assert_eq!(
        texts[3],
        format!("[unsupported MCP content: x{}]", "y".repeat(63))
    );
    assert_eq!(texts[4], "[unsupported MCP content: unknown]");

    // Whatever is still not a tools/call result stays an error.
    assert!(crate::output::decode_call_tool_result_leniently(json!({"tools": []})).is_none());
    assert!(crate::output::decode_call_tool_result_leniently(json!({
        "resultType": "incomplete",
        "content": [{"type": "video"}]
    }))
    .is_none());
}

#[test]
fn an_undecodable_resource_entry_becomes_a_placeholder_and_the_rest_is_kept() {
    let read = |result: serde_json::Value| {
        crate::output::convert_read_resource_output(serde_json::from_value(result).unwrap())
    };
    let output = read(json!({
        "contents": [
            {"uri": "mcp://local/doc", "text": "kept", "mimeType": "text/plain"},
            {"uri": "mcp://local/neither", "secret": "never shown"},
            {"uri": "mcp://local/typed", "type": "video", "data": "AAAA"},
            {"uri": "mcp://local/blob", "blob": "AA"}
        ],
        "vendorExtension": true
    }))
    .unwrap();
    assert_eq!(
        output.content,
        [
            ToolOutputContent::EmbeddedResource {
                uri: "mcp://local/doc".into(),
                mime_type: Some("text/plain".into()),
                text: Some("kept".into()),
                data: None,
            },
            ToolOutputContent::Text {
                text: "[unsupported MCP content: resource]".into()
            },
            ToolOutputContent::Text {
                text: "[unsupported MCP content: video]".into()
            },
            ToolOutputContent::EmbeddedResource {
                uri: "mcp://local/blob".into(),
                mime_type: None,
                text: None,
                data: Some("AA==".into()),
            },
        ]
    );

    // Whatever is still not a resources/read result stays an error.
    let unexpected = McpError::Protocol("MCP server returned an unexpected response type".into());
    for result in [
        json!({"contents": {"uri": "mcp://local/doc"}, "extra": 1}),
        json!({"contents": [{"uri": "x"}], "ttlMs": "soon"}),
        json!({"tools": [{"name": "bad", "inputSchema": 1}]}),
    ] {
        assert_eq!(read(result).unwrap_err(), unexpected);
    }
}

#[test]
fn more_items_than_the_bound_keep_a_prefix_and_say_how_many_were_omitted() {
    let total = MAX_TOOL_OUTPUT_ITEMS + 6;
    let output = convert_tool_output(CallToolResult::success(
        (0..total)
            .map(|index| Content::text(format!("item {index}")))
            .collect(),
    ))
    .unwrap();
    assert_eq!(output.content.len(), MAX_TOOL_OUTPUT_ITEMS);
    assert_eq!(
        output.content[MAX_TOOL_OUTPUT_ITEMS - 2],
        ToolOutputContent::Text {
            text: format!("item {}", MAX_TOOL_OUTPUT_ITEMS - 2)
        }
    );
    assert_eq!(
        output.content[MAX_TOOL_OUTPUT_ITEMS - 1],
        ToolOutputContent::Text {
            text: format!(
                "[{} more MCP content items omitted; at most {MAX_TOOL_OUTPUT_ITEMS} are kept]",
                total - (MAX_TOOL_OUTPUT_ITEMS - 1)
            )
        }
    );
    let exact = convert_tool_output(CallToolResult::success(
        (0..MAX_TOOL_OUTPUT_ITEMS)
            .map(|index| Content::text(format!("item {index}")))
            .collect(),
    ))
    .unwrap();
    assert_eq!(exact.content.len(), MAX_TOOL_OUTPUT_ITEMS);

    let resources = crate::output::convert_read_resource_output(
        ReadResourceResult::new(
            (0..total)
                .map(|index| ResourceContents::text("body", format!("mcp://local/{index}")))
                .collect(),
        )
        .into(),
    )
    .unwrap();
    assert_eq!(resources.content.len(), MAX_TOOL_OUTPUT_ITEMS);
}

#[test]
fn media_type_parameters_and_missing_base64_padding_are_accepted_in_canonical_form() {
    let output = convert_tool_output(CallToolResult::success(vec![
        Content::image("AAA", " Image/PNG; charset=binary"),
        Content::audio("AA", "audio/wav;rate=44100"),
        Content::resource(
            ResourceContents::blob("AA", "mcp://local/blob").with_mime_type("image/png; q=1"),
        ),
    ]))
    .unwrap();
    assert_eq!(
        output.content,
        [
            ToolOutputContent::Image {
                mime_type: "image/png".into(),
                data: "AAA=".into(),
                detail: None,
            },
            ToolOutputContent::Audio {
                mime_type: "audio/wav".into(),
                data: "AA==".into(),
            },
            ToolOutputContent::EmbeddedResource {
                uri: "mcp://local/blob".into(),
                mime_type: Some("image/png".into()),
                text: None,
                data: Some("AA==".into()),
            },
        ]
    );
}

#[test]
fn byte_bound_alphabet_and_media_kind_stay_strict() {
    for (data, mime_type) in [
        ("AA_-", "image/png"),
        ("A", "image/png"),
        ("AA==", "text/plain; charset=utf-8"),
        ("AA==", "; image/png"),
    ] {
        assert_eq!(
            convert_tool_output(CallToolResult::success(vec![Content::image(
                data, mime_type
            )]))
            .unwrap_err(),
            McpError::LimitExceeded,
            "{data} {mime_type}"
        );
    }
    assert_eq!(
        convert_tool_output(CallToolResult::success(vec![Content::text(
            "x".repeat(MAX_TOOL_OUTPUT_BYTES)
        )]))
        .unwrap_err(),
        McpError::LimitExceeded
    );
}
