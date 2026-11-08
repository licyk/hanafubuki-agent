use super::{platform_identity, PathIdentity};
use crate::model::{WorkspaceFsError, WorkspacePathKind};
use hanafubuki_agent_foundation::paths::ResolvedPath;
use std::fs;
use std::time::UNIX_EPOCH;

pub(crate) fn identity(path: &ResolvedPath) -> Result<PathIdentity, WorkspaceFsError> {
    let metadata = fs::metadata(path.system()).map_err(|error| {
        WorkspaceFsError::Resolution(format!("{}: {error}", path.native_text()))
    })?;
    identity_from_metadata(path, &metadata)
}

pub(crate) fn identity_from_metadata(
    path: &ResolvedPath,
    metadata: &fs::Metadata,
) -> Result<PathIdentity, WorkspaceFsError> {
    let kind = if metadata.is_file() {
        WorkspacePathKind::File
    } else if metadata.is_dir() {
        WorkspacePathKind::Directory
    } else {
        return Err(WorkspaceFsError::InvalidRequest(
            "filesystem target must be a regular file or directory".to_string(),
        ));
    };
    let modified_nanos = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    // The system form on purpose: opening by handle is exactly the case the
    // verbatim prefix exists for.
    let (platform_identity_a, platform_identity_b) = platform_identity(path.system(), metadata)?;
    Ok(PathIdentity {
        path: path.native_text().to_string(),
        key: path.key().clone(),
        kind,
        size: metadata.len(),
        modified_nanos,
        platform_identity_a,
        platform_identity_b,
    })
}
