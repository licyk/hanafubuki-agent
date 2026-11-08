use super::*;

impl WorkspaceFsService {
    pub(super) fn write_file(
        &self,
        target: TargetPlan,
        content: Vec<u8>,
        content_sha256: String,
        overwrite: bool,
        allow_parent_metadata_change: bool,
        cancellation: &CancellationToken,
    ) -> Result<WorkspaceFsResult, WorkspaceFsError> {
        let destination = target_resolved(&target).clone();
        let publish_mode = if matches!(target, TargetPlan::Existing(_)) {
            AtomicPublishMode::ReplaceExisting
        } else {
            AtomicPublishMode::CreateNew
        };
        let permissions = match &target {
            TargetPlan::Existing(path) => {
                if !overwrite {
                    return Err(WorkspaceFsError::PathChanged);
                }
                revalidate_existing(path)?;
                Some(
                    fs::metadata(path.resolved.system())
                        .map_err(|error| WorkspaceFsError::Io(error.to_string()))?
                        .permissions(),
                )
            }
            TargetPlan::Missing(path) => {
                if allow_parent_metadata_change {
                    revalidate_missing_after_staging(path)?;
                } else {
                    revalidate_missing(path)?;
                }
                None
            }
        };
        let revalidation = target.clone();
        atomic_write(
            destination.system(),
            &content,
            &content_sha256,
            permissions,
            publish_mode,
            cancellation,
            &self.publish_hook,
            move || revalidate_target(&revalidation),
        )?;
        Ok(WorkspaceFsResult::FileWritten(FileMutationResult {
            path: destination.native_text().to_string(),
            kind: WorkspacePathKind::File,
            bytes: Some(content.len()),
            sha256: Some(content_sha256),
        }))
    }
}
