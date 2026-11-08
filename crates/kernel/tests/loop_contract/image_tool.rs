//! A registered `view_image` fixture: the kernel stages the images a
//! plugin tool returns, whatever plugin provides the tool.

use hanafubuki_agent_approval::PermissionRequirement;
use hanafubuki_agent_contracts::{
    AgentActivityAction, AgentMemoryContextEffect, AgentToolHandler, Register, ToolAvailability,
    ToolDescriptor, ToolEffect, ToolFuture, ToolInvocation, ToolPlanContext, ToolRegistry,
    ToolResult,
};
use hanafubuki_agent_kernel::PreparedAgentImage;
use hanafubuki_agent_tool_contracts::{ToolExecutionClass, ToolExposure, ToolName};
use kardia_core::{Context, Inject, PluginDef, Service};

use super::*;

pub(crate) fn view_image_spec() -> hanafubuki_agent_model_api::ModelToolSpec {
    hanafubuki_agent_model_api::ModelToolSpec {
        name: "view_image".to_string(),
        description: "View a local image file.".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": { "path": { "type": "string", "minLength": 1, "maxLength": 4096 } },
            "required": ["path"],
            "additionalProperties": false
        }),
        strict: false,
    }
}

/// For turns where the model is image-capable but never calls `view_image`.
pub(crate) fn unused_image() -> PreparedAgentImage {
    PreparedAgentImage {
        bytes: vec![0],
        mime_type: "image/png".to_string(),
        width: 1,
        height: 1,
        display_name: "unused.png".to_string(),
        detail: hanafubuki_agent_kernel::AgentImageDetail::High,
    }
}

struct ViewImageFixture {
    descriptor: ToolDescriptor,
    image: Mutex<Option<PreparedAgentImage>>,
}

impl AgentToolHandler for ViewImageFixture {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn availability(&self, plan: &ToolPlanContext) -> ToolAvailability {
        if plan.image_input && plan.multimodal_tool_output {
            ToolAvailability::Function
        } else {
            ToolAvailability::Omitted
        }
    }

    fn permission<'a>(
        &'a self,
        _invocation: &'a ToolInvocation,
    ) -> ToolFuture<'a, Result<PermissionRequirement<ApprovalAction>, AgentToolError>> {
        Box::pin(async { Ok(PermissionRequirement::NotRequired) })
    }

    fn invoke(
        &self,
        _invocation: ToolInvocation,
        _approval: Option<AuthorizedAction>,
    ) -> ToolFuture<'_, Result<ToolResult, AgentToolError>> {
        let image = self.image.lock().unwrap().take();
        Box::pin(async move {
            image
                .map(ToolResult::image)
                .ok_or_else(|| AgentToolError::Failed("image already viewed".into()))
        })
    }
}

/// The standard fixtures plus the registered `view_image` fixture.
pub(crate) async fn image_tool_port(image: PreparedAgentImage) -> FakeTools {
    let tools = FakeTools::new([]).await;
    let image = Arc::new(Mutex::new(Some(image)));
    let plugin = PluginDef::new(move |ctx: Context, _| {
        let image = image.lock().unwrap().take();
        async move {
            let spec = view_image_spec();
            ctx.get::<ToolRegistry>()?
                .register(Arc::new(ViewImageFixture {
                    descriptor: ToolDescriptor {
                        name: ToolName::parse(spec.name).unwrap(),
                        description: spec.description,
                        input_schema: spec.input_schema,
                        exposure: ToolExposure::Direct,
                        execution_class: ToolExecutionClass::ConcurrentRead,
                        effect: ToolEffect::ReadOnly,
                        action: AgentActivityAction::fixed("view_image"),
                        memory_effect: AgentMemoryContextEffect::None,
                        timeout: None,
                    },
                    image: Mutex::new(image),
                }))?;
            Ok(())
        }
    })
    .name("view-image-fixture")
    .inject(Inject::new().with(ToolRegistry::NAME));
    tools.mount(&plugin).await;
    tools
}
