use super::*;

use std::collections::{BTreeMap, BTreeSet};

use hanafubuki_agent_foundation::paths::PathKey;
use serde::{Deserialize, Serialize};

mod manifest;
mod publish;
mod workspace;

pub(super) use publish::validated_manifest_note_ids;

pub(super) fn stage_one_id(
    session_id: &str,
    source_revision: u64,
    raw_memory_sha256: &str,
    rollout_summary_sha256: &str,
    rollout_path: &str,
) -> Result<String, AgentMemoryError> {
    serde_json::to_vec(&(
        session_id,
        source_revision,
        raw_memory_sha256,
        rollout_summary_sha256,
        rollout_path,
    ))
    .map(|bytes| hex_digest(&bytes))
    .map_err(json_error)
}

fn read_bounded(path: &Path, maximum: usize) -> Result<String, AgentMemoryError> {
    let bytes = fs::read(path).map_err(io_error)?;
    if bytes.len() > maximum {
        return Err(AgentMemoryError::InvalidInput(
            "memory artifact exceeds its byte bound".into(),
        ));
    }
    String::from_utf8(bytes)
        .map_err(|_| AgentMemoryError::InvalidInput("memory artifact is not UTF-8".into()))
}

fn read_bounded_corrupt(path: &Path, maximum: usize) -> Result<String, AgentMemoryError> {
    read_bounded(path, maximum).map_err(|error| AgentMemoryError::Corrupt(error.to_string()))
}

fn write_synced(path: &Path, contents: &[u8]) -> Result<(), AgentMemoryError> {
    let mut file = fs::File::create(path).map_err(io_error)?;
    file.write_all(contents).map_err(io_error)?;
    file.sync_all().map_err(io_error)
}

pub(super) fn hex_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
