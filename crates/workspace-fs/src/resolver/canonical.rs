use super::paths::target_path_text;
use super::{PatchFilePlan, PathIdentity, TargetPlan, WorkspaceOperationPlan};
use crate::model::{ExternalPathReport, WorkspaceFsError};
use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Serialize)]
#[serde(
    tag = "operation",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub(super) enum CanonicalFileAction<'a> {
    ListDirectory {
        root: &'a PathIdentity,
        max_depth: usize,
        max_entries: usize,
    },
    SearchText {
        root: &'a PathIdentity,
        query: &'a str,
        max_depth: usize,
        max_results: usize,
        max_bytes: usize,
    },
    ReadFile {
        file: &'a PathIdentity,
        offset: usize,
        max_bytes: usize,
    },
    WriteFile {
        target: CanonicalTarget<'a>,
        content_sha256: &'a str,
        content_bytes: usize,
        overwrite: bool,
    },
    ApplyPatch {
        patch_sha256: &'a str,
        changes: Vec<CanonicalPatchChange>,
        /// Resolved out-of-workspace paths, so an approval cannot be replayed
        /// against a patch that reaches somewhere else.
        external_paths: &'a [ExternalPathReport],
    },
    MovePath {
        source: &'a PathIdentity,
        destination: CanonicalMissingTarget<'a>,
    },
    DeletePath {
        target: &'a PathIdentity,
        recursive: bool,
    },
}

#[derive(Serialize)]
#[serde(
    tag = "change",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub(super) enum CanonicalPatchChange {
    Add {
        target: CanonicalTargetOwned,
        content_sha256: String,
        content_bytes: usize,
    },
    Delete {
        target: PathIdentity,
    },
    Update {
        source: PathIdentity,
        source_sha256: String,
        destination: Option<CanonicalTargetOwned>,
        content_sha256: String,
        content_bytes: usize,
    },
}

#[derive(Serialize)]
#[serde(
    tag = "state",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub(super) enum CanonicalTargetOwned {
    Existing(PathIdentity),
    Missing {
        /// A location that does not exist yet has no identity to record, so the
        /// digest of its resolved key stands in. Never the rendered path: the
        /// approval must not be pinned to a spelling.
        path_digest: String,
        parent: PathIdentity,
    },
}

#[derive(Serialize)]
#[serde(
    tag = "state",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub(super) enum CanonicalTarget<'a> {
    Existing(&'a PathIdentity),
    Missing(CanonicalMissingTarget<'a>),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CanonicalMissingTarget<'a> {
    pub(super) path_digest: String,
    pub(super) parent: &'a PathIdentity,
}

/// Paths shown on the approval card. Always the interchange form: whoever
/// approves this has to recognise the location.
pub(super) fn operation_policy_paths(plan: &WorkspaceOperationPlan) -> Vec<String> {
    match plan {
        WorkspaceOperationPlan::ListDirectory { root, .. }
        | WorkspaceOperationPlan::SearchText { root, .. } => {
            vec![root.identity.path.clone()]
        }
        WorkspaceOperationPlan::ReadFile { file, .. } => vec![file.identity.path.clone()],
        WorkspaceOperationPlan::ApplyPatch { changes } => {
            changes.iter().flat_map(patch_policy_paths).collect()
        }
        WorkspaceOperationPlan::WriteFile { target, .. } => {
            vec![target_path_text(target).to_string()]
        }
        WorkspaceOperationPlan::MovePath {
            source,
            destination,
        } => vec![
            source.identity.path.clone(),
            destination.target.native_text().to_string(),
        ],
        WorkspaceOperationPlan::DeletePath { target, .. } => {
            vec![target.identity.path.clone()]
        }
    }
}

pub(super) fn canonical_patch_change(change: &PatchFilePlan) -> CanonicalPatchChange {
    match change {
        PatchFilePlan::Add {
            target,
            content,
            content_sha256,
        } => CanonicalPatchChange::Add {
            target: canonical_target_owned(target),
            content_sha256: content_sha256.clone(),
            content_bytes: content.len(),
        },
        PatchFilePlan::Delete { target } => CanonicalPatchChange::Delete {
            target: target.identity.clone(),
        },
        PatchFilePlan::Update {
            source,
            source_sha256,
            destination,
            content,
            content_sha256,
        } => CanonicalPatchChange::Update {
            source: source.identity.clone(),
            source_sha256: source_sha256.clone(),
            destination: destination.as_deref().map(canonical_target_owned),
            content_sha256: content_sha256.clone(),
            content_bytes: content.len(),
        },
    }
}

fn canonical_target_owned(target: &TargetPlan) -> CanonicalTargetOwned {
    match target {
        TargetPlan::Existing(path) => CanonicalTargetOwned::Existing(path.identity.clone()),
        TargetPlan::Missing(path) => CanonicalTargetOwned::Missing {
            path_digest: sha256_hex(&path.target.key().digest_bytes()),
            parent: path.parent.identity.clone(),
        },
    }
}

fn patch_policy_paths(change: &PatchFilePlan) -> Vec<String> {
    match change {
        PatchFilePlan::Add { target, .. } => vec![target_path_text(target).to_string()],
        PatchFilePlan::Delete { target } => vec![target.identity.path.clone()],
        PatchFilePlan::Update {
            source,
            destination,
            ..
        } => {
            let mut paths = vec![source.identity.path.clone()];
            if let Some(destination) = destination {
                paths.push(target_path_text(destination).to_string());
            }
            paths
        }
    }
}

pub(super) fn canonical_target(target: &TargetPlan) -> CanonicalTarget<'_> {
    match target {
        TargetPlan::Existing(path) => CanonicalTarget::Existing(&path.identity),
        TargetPlan::Missing(path) => CanonicalTarget::Missing(CanonicalMissingTarget {
            path_digest: sha256_hex(&path.target.key().digest_bytes()),
            parent: &path.parent.identity,
        }),
    }
}

pub(super) fn serialize_action(
    action: &CanonicalFileAction<'_>,
) -> Result<Vec<u8>, WorkspaceFsError> {
    serde_json::to_vec(action).map_err(|error| WorkspaceFsError::Resolution(error.to_string()))
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        write!(&mut output, "{byte:02x}").expect("write to string");
    }
    output
}
