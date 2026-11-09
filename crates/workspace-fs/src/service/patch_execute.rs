use super::*;

impl WorkspaceFsService {
    pub(super) fn apply_patch(
        &self,
        changes: Vec<PatchFilePlan>,
        cancellation: &CancellationToken,
    ) -> Result<WorkspaceFsResult, WorkspaceFsError> {
        self.revalidate_patch_changes(&changes, cancellation)?;
        let mut added = Vec::new();
        let mut modified = Vec::new();
        let mut deleted = Vec::new();
        for change in changes {
            check_cancel(cancellation)?;
            match change {
                PatchFilePlan::Add {
                    target,
                    content,
                    content_sha256,
                } => {
                    let path = target_resolved(&target).native_text().to_string();
                    self.write_file(target, content, content_sha256, true, true, cancellation)?;
                    added.push(path);
                }
                PatchFilePlan::Delete { target } => {
                    let path = target.identity.path.clone();
                    self.delete_path(target, false, cancellation)?;
                    deleted.push(path);
                }
                PatchFilePlan::Update {
                    source,
                    destination,
                    content,
                    content_sha256,
                    ..
                } => {
                    let modified_path = destination
                        .as_ref()
                        .map(|target| target_resolved(target).native_text().to_string())
                        .unwrap_or_else(|| source.identity.path.clone());
                    if let Some(destination) = destination {
                        self.write_file(
                            *destination,
                            content,
                            content_sha256,
                            true,
                            true,
                            cancellation,
                        )?;
                        self.delete_path(source, false, cancellation)?;
                    } else {
                        self.write_file(
                            TargetPlan::Existing(source),
                            content,
                            content_sha256,
                            true,
                            true,
                            cancellation,
                        )?;
                    }
                    modified.push(modified_path);
                }
            }
        }
        Ok(WorkspaceFsResult::PatchApplied(PatchMutationResult {
            added,
            modified,
            deleted,
        }))
    }

    fn revalidate_patch_changes(
        &self,
        changes: &[PatchFilePlan],
        cancellation: &CancellationToken,
    ) -> Result<(), WorkspaceFsError> {
        for change in changes {
            check_cancel(cancellation)?;
            match change {
                PatchFilePlan::Add { target, .. } => revalidate_target(target)?,
                PatchFilePlan::Delete { target } => revalidate_existing(target)?,
                PatchFilePlan::Update {
                    source,
                    source_sha256,
                    destination,
                    ..
                } => {
                    revalidate_existing(source)?;
                    let current = fs::read(source.resolved.system()).map_err(|error| {
                        WorkspaceFsError::Io(format!("verify patch preimage: {error}"))
                    })?;
                    if sha256_hex(&current) != *source_sha256 {
                        return Err(WorkspaceFsError::PreimageMismatch(format!(
                            "{} changed after approval",
                            source.identity.path
                        )));
                    }
                    if let Some(destination) = destination {
                        revalidate_target(destination)?;
                    }
                }
            }
        }
        Ok(())
    }
}
