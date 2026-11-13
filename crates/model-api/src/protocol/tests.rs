use super::*;

#[test]
fn protocol_capabilities_are_explicit_and_only_responses_has_extended_tools() {
    let responses = ModelApiProtocol::Responses.tool_capabilities();
    assert!(responses.namespace_tools);
    assert!(responses.native_tool_search);
    assert!(responses.freeform_tools);
    assert!(responses.hosted_web_search);
    assert_eq!(
        responses.multimodal_tool_output_transport,
        hanafubuki_agent_tool_contracts::MultimodalToolOutputTransport::NativeToolResult
    );

    assert_eq!(
        ModelApiProtocol::ChatCompletions
            .tool_capabilities()
            .multimodal_tool_output_transport,
        hanafubuki_agent_tool_contracts::MultimodalToolOutputTransport::SyntheticUserMessage
    );
    assert_eq!(
        ModelApiProtocol::AnthropicMessages
            .tool_capabilities()
            .multimodal_tool_output_transport,
        hanafubuki_agent_tool_contracts::MultimodalToolOutputTransport::NativeToolResult
    );

    for protocol in [
        ModelApiProtocol::ChatCompletions,
        ModelApiProtocol::AnthropicMessages,
    ] {
        let capabilities = protocol.tool_capabilities();
        assert!(capabilities.function_tools);
        assert!(!capabilities.namespace_tools);
        assert!(!capabilities.native_tool_search);
        assert!(!capabilities.freeform_tools);
        assert!(!capabilities.hosted_web_search);
    }

    assert_eq!(
        ModelApiProtocol::Responses.structured_output_capability(),
        crate::ModelStructuredOutputCapability::NativeJsonSchema
    );
    assert_eq!(
        ModelApiProtocol::ChatCompletions.structured_output_capability(),
        crate::ModelStructuredOutputCapability::NativeJsonSchema
    );
    assert_eq!(
        ModelApiProtocol::AnthropicMessages.structured_output_capability(),
        crate::ModelStructuredOutputCapability::RequiredTool
    );
}
