use std::fs::{self, File};
use std::io::{Read, Seek};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hanafubuki_agent_approval::{ApprovalActionKind, AuthorizedAction};
use hanafubuki_agent_foundation::cancellation::CancellationToken;

use crate::atomic_write::{
    atomic_write, AtomicPublishHook, AtomicPublishMode, NoopAtomicPublishHook,
};
use hanafubuki_agent_foundation::paths::ResolvedPath;

use crate::model::*;
use crate::policy::WorkspaceFsPolicy;
use crate::resolver::{
    identity_from_metadata, prepare_operation, sha256_hex, target_resolved, ExistingPathPlan,
    MissingPathPlan, PatchFilePlan, TargetPlan, WorkspaceOperationPlan,
};
use crate::PreparedWorkspaceOperation;

const SEARCH_DEADLINE: Duration = Duration::from_secs(2);
const IO_CHUNK_BYTES: usize = 64 * 1024;
const MAX_PREVIEW_BYTES: usize = 512;

#[derive(Clone)]
pub struct WorkspaceFsService {
    policy: WorkspaceFsPolicy,
    publish_hook: Arc<dyn AtomicPublishHook>,
}

impl WorkspaceFsService {
    pub fn new(policy: WorkspaceFsPolicy) -> Self {
        Self {
            policy,
            publish_hook: Arc::new(NoopAtomicPublishHook),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn with_publish_hook(
        policy: WorkspaceFsPolicy,
        publish_hook: Arc<dyn AtomicPublishHook>,
    ) -> Self {
        Self {
            policy,
            publish_hook,
        }
    }

    pub fn prepare(
        &self,
        request: &WorkspaceFsRequest,
    ) -> Result<PreparedWorkspaceOperation, WorkspaceFsError> {
        prepare_operation(&self.policy, request)
    }

    /// Read one file explicitly selected by the user without creating a
    /// reusable filesystem capability.

    pub fn execute(
        &self,
        request: &WorkspaceFsRequest,
        approval: Option<&AuthorizedAction>,
        cancellation: &CancellationToken,
    ) -> Result<WorkspaceFsResult, WorkspaceFsError> {
        let prepared = self.prepare(request)?;
        if prepared.action().kind != ApprovalActionKind::FileRead {
            let approved = approval.ok_or(WorkspaceFsError::ApprovalRequired)?;
            if approved.action() != prepared.action() {
                return Err(WorkspaceFsError::ApprovalMismatch);
            }
        } else if approval.is_some() {
            return Err(WorkspaceFsError::ApprovalMismatch);
        }
        if cancellation.is_cancelled() {
            return Err(WorkspaceFsError::Cancelled);
        }
        match prepared.plan {
            WorkspaceOperationPlan::ListDirectory {
                root,
                max_depth,
                max_entries,
            } => self.list_directory(root, max_depth, max_entries, cancellation),
            WorkspaceOperationPlan::SearchText {
                root,
                matcher,
                max_depth,
                max_results,
                max_bytes,
            } => self.search_text(
                root,
                &matcher,
                max_depth,
                max_results,
                max_bytes,
                cancellation,
            ),
            WorkspaceOperationPlan::ReadFile {
                file,
                offset,
                max_bytes,
            } => self.read_file(file, offset, max_bytes, cancellation),
            WorkspaceOperationPlan::WriteFile {
                target,
                content,
                content_sha256,
                overwrite,
            } => self.write_file(
                target,
                content,
                content_sha256,
                overwrite,
                false,
                cancellation,
            ),
            WorkspaceOperationPlan::ApplyPatch { changes } => {
                self.apply_patch(changes, cancellation)
            }
            WorkspaceOperationPlan::MovePath {
                source,
                destination,
            } => self.move_path(source, destination, cancellation),
            WorkspaceOperationPlan::DeletePath { target, recursive } => {
                self.delete_path(target, recursive, cancellation)
            }
        }
    }
}

mod binary;
mod listing;
mod mutations;
mod patch_execute;
mod platform_move;
mod read;
mod revalidate;
mod search;
mod write;

use binary::{bounded_preview, bounded_utf8};
use platform_move::move_no_replace;
use revalidate::*;
