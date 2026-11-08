//! Validating a consolidated workspace and publishing it as the current
//! artifacts.

use super::manifest::{
    ArtifactManifest, ManifestNote, ManifestRollout, ManifestSource, MANIFEST_SCHEMA_VERSION,
    MAX_MANIFEST_BYTES,
};
use super::workspace::SEMANTIC_DIFF_RESOURCE;
use super::*;

impl AgentMemoryRepository {
    pub(in crate::repository) fn validate_current_artifacts(
        &self,
        expected_generation: u64,
    ) -> Result<(), AgentMemoryError> {
        validate_artifact_directory(&self.inner.paths.artifacts, Some(expected_generation))
            .map(|_| ())
    }

    /// Completes a workspace as the artifacts of `generation` (its manifest
    /// written and the whole directory validated) without publishing it.
    pub(in crate::repository) fn stage_artifacts(
        &self,
        generation: u64,
        workspace: &MemoryConsolidationWorkspace,
    ) -> Result<(), AgentMemoryError> {
        let staging = &workspace.path;
        if staging.parent() != Some(self.inner.paths.staging.as_path()) || !staging.is_dir() {
            return Err(AgentMemoryError::InvalidInput(
                "memory workspace is outside staging".into(),
            ));
        }
        for staging_only in [SEMANTIC_DIFF_RESOURCE, "ad_hoc_notes.md"] {
            let path = staging.join(staging_only);
            if path.exists() {
                fs::remove_file(path).map_err(io_error)?;
            }
        }
        let memory = read_bounded(&staging.join("MEMORY.md"), MAX_MEMORY_RESOURCE_BYTES)?;
        let summary = read_bounded(&staging.join("memory_summary.md"), MAX_MEMORY_SUMMARY_BYTES)?;
        if !summary.starts_with("v1\n") {
            return Err(AgentMemoryError::InvalidInput(
                "memory_summary.md must begin with v1".into(),
            ));
        }
        let raw = read_bounded(&staging.join("raw_memories.md"), MAX_ARTIFACT_TOTAL_BYTES)?;
        let rollout_files = read_rollout_files(staging)?;
        let rollout_summaries = rollout_files
            .iter()
            .map(|(path, contents)| ManifestRollout {
                path: path.clone(),
                sha256: hex_digest(contents.as_bytes()),
            })
            .collect::<Vec<_>>();
        let manifest = ArtifactManifest {
            schema_version: MANIFEST_SCHEMA_VERSION,
            generation,
            memory_sha256: hex_digest(memory.as_bytes()),
            summary_sha256: hex_digest(summary.as_bytes()),
            raw_memories_sha256: hex_digest(raw.as_bytes()),
            sources: workspace.sources.iter().map(ManifestSource::from).collect(),
            notes: workspace.notes.iter().map(ManifestNote::from).collect(),
            rollout_summaries,
        };
        validate_manifest_provenance(&manifest)?;
        let manifest_bytes = serde_json::to_vec_pretty(&manifest).map_err(json_error)?;
        if manifest_bytes.len() > MAX_MANIFEST_BYTES {
            return Err(AgentMemoryError::InvalidInput(
                "memory manifest exceeds its byte bound".into(),
            ));
        }
        let total = memory
            .len()
            .saturating_add(summary.len())
            .saturating_add(raw.len())
            .saturating_add(manifest_bytes.len())
            .saturating_add(
                rollout_files
                    .iter()
                    .map(|(_, contents)| contents.len())
                    .sum::<usize>(),
            );
        if total > MAX_ARTIFACT_TOTAL_BYTES {
            return Err(AgentMemoryError::InvalidInput(
                "rendered memory artifacts exceed total bound".into(),
            ));
        }
        write_synced(&staging.join("manifest.json"), &manifest_bytes)?;
        validate_artifact_directory(staging, Some(generation)).map(|_| ())
    }

    /// Makes the staged artifacts of a committed `generation` the current
    /// ones. On a failure the previous artifacts stay current and the staged
    /// ones stay where they are, for [`Self::install_pending_artifacts`].
    pub(in crate::repository) fn install_artifacts(
        &self,
        staging: &Path,
        generation: u64,
    ) -> Result<(), AgentMemoryError> {
        let old = self
            .inner
            .paths
            .reset_trash
            .join(format!("artifacts-before-{generation}"));
        if old.exists() {
            fs::remove_dir_all(&old).map_err(io_error)?;
        }
        if self.inner.paths.artifacts.exists() {
            fs::rename(&self.inner.paths.artifacts, &old).map_err(io_error)?;
        }
        if let Err(error) = fs::rename(staging, &self.inner.paths.artifacts) {
            if old.exists() {
                let _ = fs::rename(&old, &self.inner.paths.artifacts);
            }
            return Err(io_error(error));
        }
        if old.exists() {
            fs::remove_dir_all(old).map_err(io_error)?;
        }
        Ok(())
    }

    /// Installs the staged artifacts of the committed generation when a
    /// crash or a failed rename left them in staging, for a caller that
    /// holds the gate. A consolidation is prepared on the current artifacts,
    /// so this runs before one is claimed as well as at open.
    pub(in crate::repository) fn install_pending_artifacts(&self) -> Result<(), AgentMemoryError> {
        let generation = self.published_generation()?;
        let staging = self
            .inner
            .paths
            .staging
            .join(format!("generation-{generation}"));
        if generation == 0
            || !staging.is_dir()
            || validate_artifact_directory(&staging, Some(generation)).is_err()
        {
            return Ok(());
        }
        log::info!("installing the committed Agent memory generation={generation} from staging");
        self.install_artifacts(&staging, generation)
    }
}

pub(in crate::repository) fn validated_manifest_note_ids(
    directory: &Path,
    expected_generation: u64,
) -> Result<BTreeSet<String>, AgentMemoryError> {
    validate_artifact_directory(directory, Some(expected_generation)).map(|manifest| {
        manifest
            .notes
            .into_iter()
            .map(|note| note.note_id)
            .collect()
    })
}

pub(super) fn validate_artifact_directory(
    directory: &Path,
    expected_generation: Option<u64>,
) -> Result<ArtifactManifest, AgentMemoryError> {
    let root_metadata = fs::symlink_metadata(directory).map_err(io_error)?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(AgentMemoryError::Corrupt(
            "memory artifact root is not a regular directory".into(),
        ));
    }
    let mut root_entries = BTreeSet::new();
    for entry in fs::read_dir(directory).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let kind = entry.file_type().map_err(io_error)?;
        if kind.is_symlink() {
            return Err(AgentMemoryError::Corrupt(
                "memory artifacts contain a symbolic link".into(),
            ));
        }
        root_entries.insert(name);
    }
    let expected = [
        "MEMORY.md",
        "manifest.json",
        "memory_summary.md",
        "raw_memories.md",
        "rollout_summaries",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<BTreeSet<_>>();
    if root_entries != expected {
        return Err(AgentMemoryError::Corrupt(
            "memory artifact file set does not match its schema".into(),
        ));
    }
    let manifest_bytes = fs::read(directory.join("manifest.json")).map_err(io_error)?;
    if manifest_bytes.len() > MAX_MANIFEST_BYTES {
        return Err(AgentMemoryError::Corrupt(
            "memory manifest exceeds its byte bound".into(),
        ));
    }
    let manifest: ArtifactManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|_| AgentMemoryError::Corrupt("memory manifest is invalid".into()))?;
    if manifest.schema_version != MANIFEST_SCHEMA_VERSION
        || expected_generation.is_some_and(|value| manifest.generation != value)
    {
        return Err(AgentMemoryError::Corrupt(
            "memory manifest generation or schema is invalid".into(),
        ));
    }
    validate_manifest_provenance(&manifest)?;
    let memory = read_bounded_corrupt(&directory.join("MEMORY.md"), MAX_MEMORY_RESOURCE_BYTES)?;
    let summary = read_bounded_corrupt(
        &directory.join("memory_summary.md"),
        MAX_MEMORY_SUMMARY_BYTES,
    )?;
    if !summary.starts_with("v1\n") {
        return Err(AgentMemoryError::Corrupt(
            "memory_summary.md does not begin with v1".into(),
        ));
    }
    let raw = read_bounded_corrupt(&directory.join("raw_memories.md"), MAX_ARTIFACT_TOTAL_BYTES)?;
    if manifest.memory_sha256 != hex_digest(memory.as_bytes())
        || manifest.summary_sha256 != hex_digest(summary.as_bytes())
        || manifest.raw_memories_sha256 != hex_digest(raw.as_bytes())
    {
        return Err(AgentMemoryError::Corrupt(
            "memory artifact digest does not match its manifest".into(),
        ));
    }
    let rollout_files = read_rollout_files(directory)?;
    let actual_rollouts = rollout_files
        .iter()
        .map(|(path, contents)| ManifestRollout {
            path: path.clone(),
            sha256: hex_digest(contents.as_bytes()),
        })
        .collect::<Vec<_>>();
    if actual_rollouts != manifest.rollout_summaries {
        return Err(AgentMemoryError::Corrupt(
            "rollout summaries do not match the memory manifest".into(),
        ));
    }
    let total = memory
        .len()
        .saturating_add(summary.len())
        .saturating_add(raw.len())
        .saturating_add(manifest_bytes.len())
        .saturating_add(
            rollout_files
                .iter()
                .map(|(_, contents)| contents.len())
                .sum::<usize>(),
        );
    if total > MAX_ARTIFACT_TOTAL_BYTES {
        return Err(AgentMemoryError::Corrupt(
            "memory artifacts exceed the total byte bound".into(),
        ));
    }
    Ok(manifest)
}

fn validate_manifest_provenance(manifest: &ArtifactManifest) -> Result<(), AgentMemoryError> {
    let mut sessions = BTreeSet::new();
    let mut paths = BTreeSet::new();
    for source in &manifest.sources {
        validate_id(&source.session_id, "manifest session id")?;
        if !sessions.insert(source.session_id.as_str())
            || !paths.insert(source.rollout_path.as_str())
            || source.stage_one_id
                != stage_one_id(
                    &source.session_id,
                    source.source_revision,
                    &source.raw_memory_sha256,
                    &source.rollout_summary_sha256,
                    &source.rollout_path,
                )?
            || !valid_digest(&source.raw_memory_sha256)
            || !valid_digest(&source.rollout_summary_sha256)
            || !valid_digest(&source.rollout_sha256)
        {
            return Err(AgentMemoryError::Corrupt(
                "memory manifest source provenance is invalid".into(),
            ));
        }
    }
    let mut note_ids = BTreeSet::new();
    for note in &manifest.notes {
        validate_id(&note.session_id, "manifest note session id")?;
        if !note_ids.insert(note.note_id.as_str())
            || !valid_digest(&note.note_id)
            || !valid_digest(&note.content_sha256)
        {
            return Err(AgentMemoryError::Corrupt(
                "memory manifest note provenance is invalid".into(),
            ));
        }
    }
    if manifest
        .sources
        .windows(2)
        .any(|pair| pair[0].session_id >= pair[1].session_id)
        || manifest
            .rollout_summaries
            .windows(2)
            .any(|pair| pair[0].path >= pair[1].path)
        || manifest
            .notes
            .windows(2)
            .any(|pair| pair[0].note_id >= pair[1].note_id)
    {
        return Err(AgentMemoryError::Corrupt(
            "memory manifest provenance is not deterministically ordered".into(),
        ));
    }
    let manifest_rollouts = manifest
        .rollout_summaries
        .iter()
        .map(|rollout| (rollout.path.as_str(), rollout.sha256.as_str()))
        .collect::<BTreeMap<_, _>>();
    let source_rollouts = manifest
        .sources
        .iter()
        .map(|source| source.rollout_path.as_str())
        .collect::<BTreeSet<_>>();
    if source_rollouts != manifest_rollouts.keys().copied().collect::<BTreeSet<_>>()
        || manifest.sources.iter().any(|source| {
            manifest_rollouts.get(source.rollout_path.as_str()).copied()
                != Some(source.rollout_sha256.as_str())
        })
    {
        return Err(AgentMemoryError::Corrupt(
            "memory manifest rollout provenance is incomplete".into(),
        ));
    }
    Ok(())
}

fn read_rollout_files(directory: &Path) -> Result<Vec<(String, String)>, AgentMemoryError> {
    let rollout_directory = directory.join("rollout_summaries");
    let metadata = fs::symlink_metadata(&rollout_directory).map_err(io_error)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(AgentMemoryError::Corrupt(
            "rollout_summaries is not a regular directory".into(),
        ));
    }
    let mut rollout_files = Vec::new();
    for entry in fs::read_dir(rollout_directory).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        let kind = entry.file_type().map_err(io_error)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if kind.is_symlink() || !kind.is_file() || !name.ends_with(".md") || name.starts_with('.') {
            return Err(AgentMemoryError::Corrupt(
                "rollout_summaries contains an invalid entry".into(),
            ));
        }
        rollout_files.push((
            format!("rollout_summaries/{name}"),
            read_bounded_corrupt(&entry.path(), MAX_MEMORY_RESOURCE_BYTES)?,
        ));
    }
    rollout_files.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(rollout_files)
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
