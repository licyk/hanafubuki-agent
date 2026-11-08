use super::*;

impl WorkspaceFsService {
    pub(super) fn move_path(
        &self,
        source: ExistingPathPlan,
        destination: MissingPathPlan,
        cancellation: &CancellationToken,
    ) -> Result<WorkspaceFsResult, WorkspaceFsError> {
        check_cancel(cancellation)?;
        revalidate_existing(&source)?;
        revalidate_missing(&destination)?;
        move_no_replace(
            source.resolved.system(),
            destination.target.system(),
            source.identity.kind,
        )?;
        log::debug!(
            "workspace fs moved path kind={:?} {} -> {}",
            source.identity.kind,
            source.identity.path,
            destination.target.native_text()
        );
        Ok(WorkspaceFsResult::PathMoved(MovePathResult {
            source: source.identity.path,
            destination: destination.target.native_text().to_string(),
            kind: source.identity.kind,
        }))
    }

    pub(super) fn delete_path(
        &self,
        target: ExistingPathPlan,
        recursive: bool,
        cancellation: &CancellationToken,
    ) -> Result<WorkspaceFsResult, WorkspaceFsError> {
        check_cancel(cancellation)?;
        revalidate_existing(&target)?;
        let system = target.resolved.system();
        match (target.identity.kind, recursive) {
            (WorkspacePathKind::File, _) => fs::remove_file(system),
            (WorkspacePathKind::Directory, false) => fs::remove_dir(system),
            (WorkspacePathKind::Directory, true) => fs::remove_dir_all(system),
        }
        .map_err(|error| WorkspaceFsError::Io(format!("delete path: {error}")))?;
        log::debug!(
            "workspace fs deleted path kind={:?} recursive={recursive} {}",
            target.identity.kind,
            target.identity.path
        );
        Ok(WorkspaceFsResult::PathDeleted(FileMutationResult {
            path: target.identity.path,
            kind: target.identity.kind,
            bytes: None,
            sha256: None,
        }))
    }
}
