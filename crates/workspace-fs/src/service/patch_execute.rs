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
        let mut hunk_starts = Vec::with_capacity(changes.len());
        for change in changes {
            check_cancel(cancellation)?;
            match change {
                PatchFilePlan::Add {
                    target,
                    content,
                    content_sha256,
                } => {
                    let path = target_resolved(&target).native_text().to_string();
                    let target = create_patch_parents(target, cancellation)?;
                    self.write_file(target, content, content_sha256, true, true, cancellation)?;
                    added.push(path);
                    hunk_starts.push(Vec::new());
                }
                PatchFilePlan::Delete { target } => {
                    let path = target.identity.path.clone();
                    self.delete_path(target, false, cancellation)?;
                    deleted.push(path);
                    hunk_starts.push(Vec::new());
                }
                PatchFilePlan::Update {
                    source,
                    destination,
                    content,
                    content_sha256,
                    hunk_starts: starts,
                    ..
                } => {
                    let modified_path = destination
                        .as_ref()
                        .map(|target| target_resolved(target).native_text().to_string())
                        .unwrap_or_else(|| source.identity.path.clone());
                    if let Some(destination) = destination {
                        let destination = create_patch_parents(*destination, cancellation)?;
                        self.write_file(
                            destination,
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
                    hunk_starts.push(starts);
                }
            }
        }
        Ok(WorkspaceFsResult::PatchApplied(PatchMutationResult {
            added,
            modified,
            deleted,
            hunk_starts,
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

fn create_patch_parents(
    target: TargetPlan,
    cancellation: &CancellationToken,
) -> Result<TargetPlan, WorkspaceFsError> {
    let TargetPlan::Missing(mut path) = target else {
        return Ok(target);
    };
    revalidate_missing_after_staging(&path)?;
    let relative = path
        .target
        .native()
        .strip_prefix(path.parent.resolved.native())
        .map_err(|_| WorkspaceFsError::PathChanged)?;
    let mut components = relative.components();
    components.next_back();
    let mut parent = path.parent.resolved.clone();
    for component in components {
        check_cancel(cancellation)?;
        let expected = ResolvedPath::resolve_child(&parent, component.as_os_str())
            .map_err(|error| WorkspaceFsError::Resolution(error.to_string()))?;
        match fs::create_dir(expected.system()) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(WorkspaceFsError::Io(error.to_string())),
        }
        parent =
            ResolvedPath::resolve(expected.native()).map_err(|_| WorkspaceFsError::PathChanged)?;
        if parent != expected || !parent.system().is_dir() {
            return Err(WorkspaceFsError::PathChanged);
        }
    }
    path.parent = ExistingPathPlan {
        identity: crate::resolver::identity(&parent)?,
        resolved: parent,
    };
    Ok(TargetPlan::Missing(path))
}
