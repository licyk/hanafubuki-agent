use super::sha256_hex;
use crate::model::WorkspacePathKind;
use grep_regex::RegexMatcher;
use hanafubuki_agent_approval::ApprovalAction;
use hanafubuki_agent_foundation::paths::{PathKey, ResolvedPath};
use serde::Serialize;

/// What an approval pins a filesystem location to.
///
/// `path` is the interchange form — the only spelling shown to a person or sent
/// to a model. `key` is the identity, and it alone enters the approval digest,
/// so a grant survives a case-only rename and cannot be defeated by respelling
/// the same location.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PathIdentity {
    pub path: String,
    pub key: PathKey,
    pub kind: WorkspacePathKind,
    pub size: u64,
    pub modified_nanos: u128,
    pub platform_identity_a: u64,
    pub platform_identity_b: u64,
}

impl Serialize for PathIdentity {
    /// Serialized only into approval canonical bytes, so it deliberately emits
    /// the identity digest rather than the rendered path.
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;

        let mut state = serializer.serialize_struct("PathIdentity", 6)?;
        state.serialize_field("pathDigest", &sha256_hex(&self.key.digest_bytes()))?;
        state.serialize_field("kind", &self.kind)?;
        state.serialize_field("size", &self.size)?;
        state.serialize_field("modifiedNanos", &self.modified_nanos)?;
        state.serialize_field("platformIdentityA", &self.platform_identity_a)?;
        state.serialize_field("platformIdentityB", &self.platform_identity_b)?;
        state.end()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ExistingPathPlan {
    pub resolved: ResolvedPath,
    pub identity: PathIdentity,
}

#[derive(Clone, Debug)]
pub(crate) struct MissingPathPlan {
    pub target: ResolvedPath,
    pub parent: ExistingPathPlan,
}

#[derive(Clone, Debug)]
pub(crate) enum TargetPlan {
    Existing(ExistingPathPlan),
    Missing(MissingPathPlan),
}

#[derive(Clone, Debug)]
pub(crate) enum WorkspaceOperationPlan {
    ListDirectory {
        root: ExistingPathPlan,
        max_depth: usize,
        max_entries: usize,
    },
    SearchText {
        root: ExistingPathPlan,
        matcher: RegexMatcher,
        max_depth: usize,
        max_results: usize,
        max_bytes: usize,
    },
    ReadFile {
        file: ExistingPathPlan,
        offset: usize,
        max_bytes: usize,
    },
    WriteFile {
        target: TargetPlan,
        content: Vec<u8>,
        content_sha256: String,
        overwrite: bool,
    },
    ApplyPatch {
        changes: Vec<PatchFilePlan>,
    },
    MovePath {
        source: ExistingPathPlan,
        destination: MissingPathPlan,
    },
    DeletePath {
        target: ExistingPathPlan,
        recursive: bool,
    },
}

#[derive(Clone, Debug)]
pub(crate) enum PatchFilePlan {
    Add {
        target: TargetPlan,
        content: Vec<u8>,
        content_sha256: String,
    },
    Delete {
        target: ExistingPathPlan,
    },
    Update {
        source: ExistingPathPlan,
        source_sha256: String,
        destination: Option<Box<TargetPlan>>,
        content: Vec<u8>,
        content_sha256: String,
    },
}

pub struct PreparedWorkspaceOperation {
    pub(crate) plan: WorkspaceOperationPlan,
    pub(super) action: ApprovalAction,
}

impl PreparedWorkspaceOperation {
    pub fn action(&self) -> &ApprovalAction {
        &self.action
    }
}
