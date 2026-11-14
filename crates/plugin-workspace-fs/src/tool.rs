use std::sync::Arc;

use hanafubuki_agent_approval::{
    ApprovalAction, ApprovalActionKind, AuthorizedAction, PermissionRequirement,
};
use hanafubuki_agent_contracts::{
    AgentActivityAction, AgentMemoryContextEffect, AgentPatchHunkStart, AgentToolDisplayMetadata,
    AgentToolError, AgentToolHandler, BlockingService, ToolAvailability, ToolDescriptor,
    ToolEffect, ToolFuture, ToolInvocation, ToolPlanContext, ToolResult,
};
use hanafubuki_agent_tool_contracts::{
    FreeformToolSpec, ToolContractError, ToolExecutionClass, ToolExposure, ToolName,
};
use hanafubuki_agent_workspace_fs::{WorkspaceFsResult, WorkspaceFsService};

use crate::arguments::parse;
use crate::specs::{apply_patch_freeform, spec};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FsToolKind {
    ListDirectory,
    SearchText,
    ReadFile,
    ApplyPatch,
    MovePath,
    DeletePath,
}

impl FsToolKind {
    pub(crate) const ALL: [Self; 6] = [
        Self::ListDirectory,
        Self::SearchText,
        Self::ReadFile,
        Self::ApplyPatch,
        Self::MovePath,
        Self::DeletePath,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::ListDirectory => "list_directory",
            Self::SearchText => "search_text",
            Self::ReadFile => "read_file",
            Self::ApplyPatch => "apply_patch",
            Self::MovePath => "move_path",
            Self::DeletePath => "delete_path",
        }
    }

    fn effect(self) -> ToolEffect {
        match self {
            Self::ListDirectory | Self::SearchText | Self::ReadFile => ToolEffect::ReadOnly,
            Self::ApplyPatch | Self::MovePath | Self::DeletePath => ToolEffect::Mutating,
        }
    }
}

/// One workspace filesystem tool. Every call prepares an immutable plan from
/// the workspace policy; reads need no approval, writes need approval of the
/// plan's exact action, and `WorkspaceFsService::execute` re-prepares and
/// re-checks that action before any I/O.
pub(crate) struct FsTool {
    kind: FsToolKind,
    files: WorkspaceFsService,
    blocking: Arc<BlockingService>,
    descriptor: ToolDescriptor,
    freeform: Option<FreeformToolSpec>,
}

impl FsTool {
    pub(crate) fn new(
        kind: FsToolKind,
        files: WorkspaceFsService,
        blocking: Arc<BlockingService>,
    ) -> Result<Self, ToolContractError> {
        let spec = spec(kind);
        Ok(Self {
            kind,
            files,
            blocking,
            descriptor: ToolDescriptor {
                name: ToolName::parse(kind.name())?,
                description: spec.description.to_string(),
                input_schema: spec.input_schema,
                exposure: ToolExposure::Direct,
                // Filesystem reads are serialized with the session's writes.
                execution_class: ToolExecutionClass::Exclusive,
                effect: kind.effect(),
                action: AgentActivityAction::fixed(kind.name()),
                memory_effect: AgentMemoryContextEffect::None,
                timeout: None,
            },
            freeform: match kind {
                FsToolKind::ApplyPatch => Some(apply_patch_freeform()?),
                _ => None,
            },
        })
    }
}

impl AgentToolHandler for FsTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn availability(&self, plan: &ToolPlanContext) -> ToolAvailability {
        match &self.freeform {
            Some(freeform) if plan.freeform_tools => ToolAvailability::Freeform(freeform.clone()),
            _ => ToolAvailability::Function,
        }
    }

    fn permission<'a>(
        &'a self,
        invocation: &'a ToolInvocation,
    ) -> ToolFuture<'a, Result<PermissionRequirement<ApprovalAction>, AgentToolError>> {
        Box::pin(async move {
            let (request, _) = parse(self.kind, &invocation.arguments)?;
            let files = self.files.clone();
            let prepared = self
                .blocking
                .run("agent workspace-fs prepare", move || {
                    files
                        .prepare(&request)
                        .map(|prepared| prepared.action().clone())
                })
                .await?;
            Ok(match prepared {
                Ok(action) if action.kind == ApprovalActionKind::FileRead => {
                    PermissionRequirement::NotRequired
                }
                Ok(action) => PermissionRequirement::ReviewRequired(action),
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
            let (request, notes) = parse(self.kind, &invocation.arguments)?;
            let files = self.files.clone();
            let cancellation = invocation.cancellation.clone();
            let result = self
                .blocking
                .run("agent workspace-fs execute", move || {
                    files.execute(&request, approval.as_ref(), &cancellation)
                })
                .await?;
            if invocation.cancellation.is_cancelled() {
                return Err(AgentToolError::Cancelled);
            }
            let result = result.map_err(|error| AgentToolError::Failed(error.to_string()))?;
            let metadata = display_metadata(&result);
            serde_json::to_value(result)
                .map(|value| {
                    ToolResult::success(value)
                        .with_argument_notes(&notes)
                        .with_display_metadata(metadata)
                })
                .map_err(|error| AgentToolError::Failed(error.to_string()))
        })
    }
}

/// Where an applied patch's chunks landed, for its diff's line numbers;
/// the model's result never carries them.
fn display_metadata(result: &WorkspaceFsResult) -> AgentToolDisplayMetadata {
    let WorkspaceFsResult::PatchApplied(applied) = result else {
        return AgentToolDisplayMetadata::default();
    };
    AgentToolDisplayMetadata {
        patch_hunk_starts: Some(
            applied
                .hunk_starts
                .iter()
                .map(|file| {
                    file.iter()
                        .map(|start| AgentPatchHunkStart {
                            old_line: start.old_line,
                            new_line: start.new_line,
                        })
                        .collect()
                })
                .collect(),
        ),
        ..Default::default()
    }
}
