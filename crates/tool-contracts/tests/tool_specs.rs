use serde_json::json;

use hanafubuki_agent_tool_contracts::*;

fn function_spec() -> ToolSpec {
    ToolSpec::Function(FunctionToolSpec {
        name: ToolName::namespaced(vec!["server".into()], "read").unwrap(),
        description: "Read one value".into(),
        input_schema: json!({
            "type": "object",
            "properties": { "id": { "type": "string" } },
            "required": ["id"],
            "additionalProperties": false
        }),
        strict: false,
        deferred: true,
    })
}

#[test]
fn tool_spec_variants_round_trip_and_validate() {
    let specs = vec![
        function_spec(),
        ToolSpec::Namespace(NamespaceToolSpec {
            name: ToolName::parse("server").unwrap(),
            description: "Server tools".into(),
            tools: Vec::new(),
        }),
        ToolSpec::ToolSearch(ToolSearchSpec {
            execution: "client".into(),
            description: "Search deferred tools".into(),
            parameters: json!({
                "type": "object",
                "properties": {"query": {"type": "string"}},
                "required": ["query"],
                "additionalProperties": false
            }),
        }),
        ToolSpec::HostedWebSearch(HostedWebSearchSpec {
            external_web_access: Some(true),
            indexed_web_access: None,
            allowed_domains: None,
            search_context_size: Some("medium".into()),
        }),
        ToolSpec::Freeform(FreeformToolSpec {
            name: ToolName::parse("shell").unwrap(),
            description: "Run a shell grammar".into(),
            format: FreeformToolFormat {
                format_type: "grammar".into(),
                syntax: "lark".into(),
                definition: "start: /.+/".into(),
            },
        }),
    ];
    for spec in specs {
        spec.validate().unwrap();
        let encoded = serde_json::to_vec(&spec).unwrap();
        assert_eq!(serde_json::from_slice::<ToolSpec>(&encoded).unwrap(), spec);
    }
}

#[test]
fn canonical_digest_ignores_json_object_insertion_order() {
    let left = json!({"b": 2, "a": {"d": 4, "c": 3}});
    let right = json!({"a": {"c": 3, "d": 4}, "b": 2});
    assert_eq!(
        canonical_digest(&left).unwrap(),
        canonical_digest(&right).unwrap()
    );
}

#[test]
fn names_are_bounded_namespaced_and_have_stable_aliases() {
    let name = ToolName::namespaced(vec!["mcp".into(), "local".into()], "read").unwrap();
    assert_eq!(name.qualified(), "mcp::local::read");
    assert_eq!(name.flattened_alias(), "mcp__local__read");
    assert!(ToolName::parse("bad/name").is_err());
    assert!(ToolName::parse("bad.name").is_err());
    assert!(ToolName::namespaced(vec!["x".into(); 9], "read").is_err());
}

#[test]
fn output_rejects_wrong_mime_and_accepts_bounded_typed_content() {
    let output = ToolOutput {
        content: vec![
            ToolOutputContent::Text { text: "ok".into() },
            ToolOutputContent::Image {
                mime_type: "image/png".into(),
                data: "AA==".into(),
                detail: Some(ToolOutputImageDetail::High),
            },
            ToolOutputContent::ResourceLink {
                name: "remote".into(),
                uri: "mcp://server/resource".into(),
                mime_type: Some("text/plain".into()),
            },
        ],
        structured_content: Some(json!({"ok": true})),
        is_error: false,
    };
    output.validate().unwrap();
    let legacy = output.legacy_text_projection().unwrap();
    assert!(legacy.contains("unsupported_image"));
    assert!(!legacy.contains("AA=="));

    let invalid = ToolOutput {
        content: vec![ToolOutputContent::Image {
            mime_type: "text/html".into(),
            data: "AA==".into(),
            detail: None,
        }],
        structured_content: None,
        is_error: false,
    };
    assert_eq!(invalid.validate(), Err(ToolContractError::InvalidMimeType));
}

#[test]
fn tool_ids_reject_empty_unbounded_and_unsafe_values() {
    assert_eq!(
        ToolProviderId::parse("management.reads-v1")
            .unwrap()
            .as_str(),
        "management.reads-v1"
    );
    for value in [String::new(), "x".repeat(129), "unsafe/path".to_string()] {
        assert!(ToolId::parse(value).is_err());
    }
}

#[test]
fn tools_default_to_exclusive_execution() {
    assert_eq!(ToolExecutionClass::default(), ToolExecutionClass::Exclusive);
}

#[test]
fn canonical_media_type_and_base64_repair_only_parameters_case_and_padding() {
    assert_eq!(
        canonical_media_type(" Image/PNG ; charset=binary"),
        "image/png"
    );
    assert_eq!(canonical_media_type("audio/wav"), "audio/wav");
    assert_eq!(canonical_base64("AA".into()), "AA==");
    assert_eq!(canonical_base64("AAA".into()), "AAA=");
    assert_eq!(canonical_base64("AAAA".into()), "AAAA");
    assert_eq!(canonical_base64("A".into()), "A");
    assert_eq!(canonical_base64("AA=".into()), "AA=");

    let image = |data: &str, mime_type: &str| ToolOutput {
        content: vec![ToolOutputContent::Image {
            mime_type: mime_type.into(),
            data: data.into(),
            detail: None,
        }],
        structured_content: None,
        is_error: false,
    };
    assert_eq!(image("AA", "image/png").validate(), Ok(()));
    assert_eq!(image("AA==", "image/png").validate(), Ok(()));
    assert_eq!(
        image("AA_-", "image/png").validate(),
        Err(ToolContractError::OutputTooLarge)
    );
    assert_eq!(
        image("AA==", "image/png; charset=binary").validate(),
        Err(ToolContractError::InvalidMimeType)
    );
}
