use std::sync::Arc;

use hanafubuki_agent_approval::{ApprovalAction, AuthorizedAction, PermissionRequirement};
use hanafubuki_agent_contracts::tools::arguments::required_bounded_string;
use hanafubuki_agent_contracts::{
    object_arguments, AgentActivityAction, AgentMemoryContextEffect, AgentToolError,
    AgentToolHandler, BlockingService, Register, ToolAvailability, ToolCallValidationError,
    ToolDescriptor, ToolEffect, ToolFuture, ToolInvocation, ToolPlanContext, ToolRegistry,
    ToolResult,
};
use hanafubuki_agent_tool_contracts::{
    ToolContractError, ToolExecutionClass, ToolExposure, ToolName,
};
use hanafubuki_agent_workspace_fs::WorkspaceFsService;
use kardia_core::{Context, Inject, PluginDef, Service};
use serde_json::{json, Value};

use crate::{prepare_view_image, MAXIMUM_SOURCE_IMAGE_BYTES};

/// Registers `view_image`, which reads a workspace image and stages it into
/// history with the application's image normalization.
pub(crate) fn view_image_plugin(files: WorkspaceFsService) -> PluginDef {
    PluginDef::new(move |ctx: Context, _| {
        let files = files.clone();
        async move {
            let tools = ctx.get::<ToolRegistry>()?;
            let blocking = ctx.get::<BlockingService>()?.arc().clone();
            let tool = ViewImageTool::new(files, blocking).map_err(kardia_core::Error::new)?;
            tools.register(Arc::new(tool))?;
            Ok(())
        }
    })
    .name("view-image")
    .inject(
        Inject::new()
            .with(ToolRegistry::NAME)
            .with(BlockingService::NAME),
    )
}

struct ViewImageTool {
    files: WorkspaceFsService,
    blocking: Arc<BlockingService>,
    descriptor: ToolDescriptor,
}

impl ViewImageTool {
    fn new(
        files: WorkspaceFsService,
        blocking: Arc<BlockingService>,
    ) -> Result<Self, ToolContractError> {
        Ok(Self {
            files,
            blocking,
            descriptor: ToolDescriptor {
                name: ToolName::parse("view_image")?,
                description: "View a local image file from the filesystem when visual inspection is needed. Use this for images already available on disk.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "minLength": 1, "maxLength": 4096 }
                    },
                    "required": ["path"],
                    "additionalProperties": false
                }),
                exposure: ToolExposure::Direct,
                execution_class: ToolExecutionClass::ConcurrentRead,
                effect: ToolEffect::ReadOnly,
                action: AgentActivityAction::fixed("view_image"),
                memory_effect: AgentMemoryContextEffect::None,
                timeout: None,
            },
        })
    }
}

fn parse(arguments: &Value) -> Result<String, ToolCallValidationError> {
    let arguments = object_arguments(arguments)?;
    required_bounded_string(arguments, "path", 4096)
}

impl AgentToolHandler for ViewImageTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    /// Offered only when the model takes images and the provider can return
    /// an image as tool output.
    fn availability(&self, plan: &ToolPlanContext) -> ToolAvailability {
        if plan.image_input && plan.multimodal_tool_output {
            ToolAvailability::Function
        } else {
            ToolAvailability::Omitted
        }
    }

    fn permission<'a>(
        &'a self,
        invocation: &'a ToolInvocation,
    ) -> ToolFuture<'a, Result<PermissionRequirement<ApprovalAction>, AgentToolError>> {
        Box::pin(async move {
            let path = parse(&invocation.arguments)?;
            if !invocation.context.image_input {
                return Ok(PermissionRequirement::Forbidden {
                    reason:
                        "view_image is not allowed because the model does not support image input"
                            .to_string(),
                });
            }
            let files = self.files.clone();
            let validated = self
                .blocking
                .run("agent view_image validate", move || {
                    files.validate_bounded_binary_read(&path, MAXIMUM_SOURCE_IMAGE_BYTES)
                })
                .await?;
            Ok(match validated {
                Ok(_) => PermissionRequirement::NotRequired,
                Err(error) => PermissionRequirement::Forbidden {
                    reason: error.to_string(),
                },
            })
        })
    }

    fn invoke(
        &self,
        invocation: ToolInvocation,
        approval: Option<AuthorizedAction>,
    ) -> ToolFuture<'_, Result<ToolResult, AgentToolError>> {
        Box::pin(async move {
            if approval.is_some() {
                return Err(AgentToolError::Failed(
                    "read-only Agent tool received an unrelated approval".to_string(),
                ));
            }
            let path = parse(&invocation.arguments)?;
            let files = self.files.clone();
            let cancellation = invocation.cancellation.clone();
            let image = self
                .blocking
                .run("agent view_image read", move || {
                    prepare_view_image(&files, &path, &cancellation)
                })
                .await?;
            if invocation.cancellation.is_cancelled() {
                return Err(AgentToolError::Cancelled);
            }
            image.map(ToolResult::image).map_err(AgentToolError::Failed)
        })
    }
}
