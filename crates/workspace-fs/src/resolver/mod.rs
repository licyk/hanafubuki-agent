use std::collections::HashSet;
use std::fs;
use std::path::Path;

use hanafubuki_agent_approval::{
    ApprovalAction, ApprovalActionKind, ApprovalOrigin, ApprovalRiskClass, PermissionActionDetails,
};
use hanafubuki_agent_foundation::paths::{PathKey, ResolvedPath};

use crate::model::*;
use crate::patch::{derive_updated_content, parse_patch, PatchHunk};
use crate::policy::{resolution_error, WorkspaceFsPolicy};

mod canonical;
mod identity;
mod patch_plan;
mod paths;
mod plan;
mod platform_identity;
mod prepare;

pub(crate) use canonical::sha256_hex;
pub(crate) use identity::{identity, identity_from_metadata};
pub(crate) use paths::{resolve_existing, target_resolved};
pub use plan::PreparedWorkspaceOperation;
pub(crate) use plan::{
    ExistingPathPlan, MissingPathPlan, PatchFilePlan, PathIdentity, TargetPlan,
    WorkspaceOperationPlan,
};
pub(crate) use prepare::prepare_operation;

use canonical::*;
use patch_plan::{prepare_patch_changes, PreparedPatch};
use paths::{resolve_missing, resolve_target, target_path_text};
use platform_identity::platform_identity;
use prepare::validate_content;
