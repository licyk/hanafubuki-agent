use super::*;

#[derive(Clone)]
struct ImageCapableModel {
    inner: FakeModelTransport,
}

impl ModelPort for ImageCapableModel {
    fn tool_capabilities(&self) -> hanafubuki_agent_tool_contracts::ToolProviderCapabilities {
        hanafubuki_agent_tool_contracts::ToolProviderCapabilities {
            function_tools: true,
            strict_json_schema: false,
            maximum_tool_name_bytes: 64,
            namespace_tools: false,
            native_tool_search: false,
            freeform_tools: false,
            hosted_web_search: false,
            parallel_tool_calls: false,
            multimodal_tool_output_transport:
                hanafubuki_agent_tool_contracts::MultimodalToolOutputTransport::SyntheticUserMessage,
            required_named_tool_choice: false,
        }
    }

    fn route_key(&self, model: &str) -> Option<hanafubuki_agent_model_api::ModelRouteKey> {
        Some(hanafubuki_agent_model_api::ModelRouteKey::new(
            hanafubuki_agent_model_api::ModelApiProtocol::ChatCompletions,
            &hanafubuki_agent_model_api::ModelEndpoint::parse("https://images.test/v1").unwrap(),
            model,
        ))
    }

    fn stream(
        &self,
        request: ModelRequest,
        cancellation: CancellationToken,
    ) -> ModelFuture<'_, ModelResult<ModelStream>> {
        self.inner.stream(request, cancellation)
    }
}

mod images;

struct CancellingImageRejectionModel {
    attempts: AtomicUsize,
}

impl ModelPort for CancellingImageRejectionModel {
    fn tool_capabilities(&self) -> hanafubuki_agent_tool_contracts::ToolProviderCapabilities {
        hanafubuki_agent_tool_contracts::ToolProviderCapabilities {
            function_tools: true,
            strict_json_schema: false,
            maximum_tool_name_bytes: 64,
            namespace_tools: false,
            native_tool_search: false,
            freeform_tools: false,
            hosted_web_search: false,
            parallel_tool_calls: false,
            multimodal_tool_output_transport:
                hanafubuki_agent_tool_contracts::MultimodalToolOutputTransport::SyntheticUserMessage,
            required_named_tool_choice: false,
        }
    }

    fn route_key(&self, model: &str) -> Option<hanafubuki_agent_model_api::ModelRouteKey> {
        Some(hanafubuki_agent_model_api::ModelRouteKey::new(
            hanafubuki_agent_model_api::ModelApiProtocol::ChatCompletions,
            &hanafubuki_agent_model_api::ModelEndpoint::parse("https://cancel-images.test/v1")
                .unwrap(),
            model,
        ))
    }

    fn stream(
        &self,
        _request: ModelRequest,
        cancellation: CancellationToken,
    ) -> ModelFuture<'_, ModelResult<ModelStream>> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        cancellation.cancel();
        Box::pin(async {
            Err(ModelError::UnsupportedImageInput {
                status: Some(400),
                code: Some("unsupported_image".into()),
                message: "image input is not supported".into(),
                request_id: None,
            })
        })
    }
}

fn direct_image_request(text: &str) -> ModelRequest {
    image_request(vec![ModelInputItem::UserMessage {
        content: vec![
            hanafubuki_agent_model_api::ModelUserContent::InputText {
                text: text.to_string(),
            },
            hanafubuki_agent_model_api::ModelUserContent::InputImage {
                mime_type: "image/png".to_string(),
                data: vec![1, 2, 3],
                detail: hanafubuki_agent_model_api::ModelImageDetail::High,
            },
        ],
    }])
}

fn direct_image_turn(
    service: &AgentService,
    session_id: AgentSessionId,
    text: &str,
) -> AgentTurnInput {
    let image = service
        .stage_image_attachment(
            session_id,
            hanafubuki_agent_kernel::PreparedAgentImage {
                bytes: vec![1, 2, 3],
                mime_type: "image/png".to_string(),
                width: 1,
                height: 1,
                display_name: "fixture.png".to_string(),
                detail: hanafubuki_agent_kernel::AgentImageDetail::High,
            },
        )
        .unwrap();
    AgentTurnInput {
        text: text.to_string(),
        images: vec![image],
        skills: Vec::new(),
        turn: Default::default(),
        world_state: Default::default(),
        permission_mode: hanafubuki_agent_kernel::AgentPermissionMode::Manual,
        origin: Default::default(),
        provisional_title: None,
    }
}

mod reasoning;
mod retry;
mod terminal;
mod tolerance;
mod tools;
mod usage;
