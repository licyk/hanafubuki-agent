use super::*;
use crate::patch::ParsedPatch;

pub(super) struct PreparedPatch {
    pub(super) changes: Vec<PatchFilePlan>,
    /// Resolved paths this patch touches that lie outside the workspace. Empty
    /// for an ordinary in-workspace patch.
    pub(super) external_paths: Vec<ExternalPathReport>,
}

pub(super) fn prepare_patch_changes(
    policy: &WorkspaceFsPolicy,
    patch: &str,
) -> Result<PreparedPatch, WorkspaceFsError> {
    let ParsedPatch { hunks } = parse_patch(patch)?;
    let mut claimed_paths = HashSet::new();
    let mut external_paths = Vec::new();
    let mut changes = Vec::with_capacity(hunks.len());
    for hunk in hunks {
        match hunk {
            PatchHunk::Add { path, content } => {
                validate_content(&content)?;
                let target = resolve_target(policy, &patch_path_text(&path)?)?;
                ensure_patch_file_target(&target)?;
                claim_patch_path(&mut claimed_paths, target_resolved(&target))?;
                note_external_path(policy, &path, target_resolved(&target), &mut external_paths);
                let content = content.into_bytes();
                changes.push(PatchFilePlan::Add {
                    content_sha256: sha256_hex(&content),
                    target,
                    content,
                });
            }
            PatchHunk::Delete { path } => {
                let target = resolve_existing(
                    policy,
                    &patch_path_text(&path)?,
                    Some(WorkspacePathKind::File),
                )?;
                claim_patch_path(&mut claimed_paths, &target.resolved)?;
                note_external_path(policy, &path, &target.resolved, &mut external_paths);
                changes.push(PatchFilePlan::Delete { target });
            }
            PatchHunk::Update {
                path,
                move_to,
                chunks,
            } => {
                let source = resolve_existing(
                    policy,
                    &patch_path_text(&path)?,
                    Some(WorkspacePathKind::File),
                )?;
                claim_patch_path(&mut claimed_paths, &source.resolved)?;
                note_external_path(policy, &path, &source.resolved, &mut external_paths);
                let source_bytes = read_patch_preimage(&source)?;
                let source_sha256 = sha256_hex(&source_bytes);
                let original =
                    String::from_utf8(source_bytes).map_err(|_| WorkspaceFsError::BinaryFile)?;
                // Naming both spellings is what makes a mismatch actionable:
                // the model can only correct the header it actually wrote.
                let label = patch_path_label(&path, &source.identity.path);
                let updated = derive_updated_content(&original, &label, &chunks)?;
                validate_content(&updated)?;
                let destination = move_to
                    .map(|path| {
                        let target = resolve_target(policy, &patch_path_text(&path)?)?;
                        ensure_patch_file_target(&target)?;
                        claim_patch_path(&mut claimed_paths, target_resolved(&target))?;
                        note_external_path(
                            policy,
                            &path,
                            target_resolved(&target),
                            &mut external_paths,
                        );
                        Ok(target)
                    })
                    .transpose()?;
                let content = updated.into_bytes();
                changes.push(PatchFilePlan::Update {
                    source,
                    source_sha256,
                    destination,
                    content_sha256: sha256_hex(&content),
                    content,
                });
            }
        }
    }
    // The resolved external paths are collected above for the approval action:
    // a patch that leaves the workspace is classified and reviewed there, with
    // the exact paths frozen into the action's canonical form, summary, and
    // risk. The parser no longer demands an envelope declaration; elevation is
    // decided by the approval channel, not by patch grammar.
    external_paths.sort();
    external_paths.dedup();
    Ok(PreparedPatch {
        changes,
        external_paths,
    })
}

/// Record a resolved path that lies outside the workspace root.
///
/// Containment is decided on path identity, not on the text of either side, so
/// a workspace spelled `\\?\E:\ws` still contains a request spelled `E:\WS\x`.
fn note_external_path(
    policy: &WorkspaceFsPolicy,
    requested: &Path,
    resolved: &ResolvedPath,
    external: &mut Vec<ExternalPathReport>,
) {
    if !policy.is_within_workspace(resolved) {
        external.push(ExternalPathReport {
            requested: requested.to_string_lossy().into_owned(),
            resolved: resolved.native_text().to_string(),
        });
    }
}

/// Render a patch target for a diagnostic, naming the model's own spelling
/// whenever it differs from where the text resolved.
fn patch_path_label(requested: &Path, resolved: &str) -> String {
    let requested = requested.to_string_lossy();
    if requested == resolved {
        resolved.to_string()
    } else {
        format!("{resolved} (written as {requested})")
    }
}

fn read_patch_preimage(file: &ExistingPathPlan) -> Result<Vec<u8>, WorkspaceFsError> {
    if file.identity.size > MAX_WRITE_BYTES as u64 {
        return Err(WorkspaceFsError::LimitExceeded(format!(
            "patch preimage exceeds {MAX_WRITE_BYTES} bytes"
        )));
    }
    let bytes = fs::read(file.resolved.system())
        .map_err(|error| WorkspaceFsError::Io(format!("read patch preimage: {error}")))?;
    if bytes.contains(&0) {
        return Err(WorkspaceFsError::BinaryFile);
    }
    Ok(bytes)
}

fn patch_path_text(path: &Path) -> Result<String, WorkspaceFsError> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| WorkspaceFsError::InvalidRequest("patch path is not valid UTF-8".into()))
}

fn ensure_patch_file_target(target: &TargetPlan) -> Result<(), WorkspaceFsError> {
    if matches!(target, TargetPlan::Existing(path) if path.identity.kind != WorkspacePathKind::File)
    {
        return Err(WorkspaceFsError::InvalidRequest(
            "patch target must be a file".into(),
        ));
    }
    Ok(())
}

/// Claimed by identity, so two hunks cannot reach the same file by spelling it
/// two different ways and defeat the one-change-per-path rule.
fn claim_patch_path(
    paths: &mut HashSet<PathKey>,
    path: &ResolvedPath,
) -> Result<(), WorkspaceFsError> {
    if !paths.insert(path.key().clone()) {
        return Err(WorkspaceFsError::InvalidRequest(
            "a patch may affect each resolved path only once".into(),
        ));
    }
    Ok(())
}
