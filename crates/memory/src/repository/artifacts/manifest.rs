//! The manifest a consolidation workspace is published under.

use super::*;

pub(super) const MANIFEST_SCHEMA_VERSION: u64 = 2;
pub(super) const MAX_MANIFEST_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ArtifactManifest {
    pub(super) schema_version: u64,
    pub(super) generation: u64,
    pub(super) memory_sha256: String,
    pub(super) summary_sha256: String,
    pub(super) raw_memories_sha256: String,
    pub(super) sources: Vec<ManifestSource>,
    pub(super) notes: Vec<ManifestNote>,
    pub(super) rollout_summaries: Vec<ManifestRollout>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ManifestSource {
    pub(super) session_id: String,
    pub(super) source_revision: u64,
    pub(super) stage_one_id: String,
    pub(super) raw_memory_sha256: String,
    pub(super) rollout_summary_sha256: String,
    pub(super) rollout_path: String,
    pub(super) rollout_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ManifestRollout {
    pub(super) path: String,
    pub(super) sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ManifestNote {
    pub(super) note_id: String,
    pub(super) session_id: String,
    pub(super) turn_id: u64,
    pub(super) source_revision: u64,
    pub(super) reset_epoch: u64,
    pub(super) operation: crate::AdHocMemoryNoteOperation,
    pub(super) content_sha256: String,
}

impl From<&AdHocMemoryNote> for ManifestNote {
    fn from(note: &AdHocMemoryNote) -> Self {
        Self {
            note_id: note.note_id.clone(),
            session_id: note.session_id.clone(),
            turn_id: note.turn_id,
            source_revision: note.source_revision,
            reset_epoch: note.reset_epoch,
            operation: note.operation,
            content_sha256: hex_digest(note.content.as_bytes()),
        }
    }
}

impl From<&MemoryWorkspaceSource> for ManifestSource {
    fn from(source: &MemoryWorkspaceSource) -> Self {
        Self {
            session_id: source.session_id.clone(),
            source_revision: source.source_revision,
            stage_one_id: source.stage_one_id.clone(),
            raw_memory_sha256: source.raw_memory_sha256.clone(),
            rollout_summary_sha256: source.rollout_summary_sha256.clone(),
            rollout_path: source.rollout_path.clone(),
            rollout_sha256: source.rollout_sha256.clone(),
        }
    }
}
