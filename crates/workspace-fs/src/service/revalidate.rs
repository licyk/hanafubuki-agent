use crate::model::WorkspaceFsError;
use crate::resolver::{identity, ExistingPathPlan, MissingPathPlan, TargetPlan};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_foundation::paths::ResolvedPath;
use std::fs;

pub(super) fn revalidate_target(target: &TargetPlan) -> Result<(), WorkspaceFsError> {
    match target {
        TargetPlan::Existing(path) => revalidate_existing(path),
        TargetPlan::Missing(path) => revalidate_missing_after_staging(path),
    }
}

pub(super) fn revalidate_existing(path: &ExistingPathPlan) -> Result<(), WorkspaceFsError> {
    let current =
        ResolvedPath::resolve(path.resolved.native()).map_err(|_| WorkspaceFsError::PathChanged)?;
    // Compared on identity: two spellings of one location are the same place,
    // and a byte-wise path comparison would report a spurious change.
    if current != path.resolved || identity(&current)? != path.identity {
        return Err(WorkspaceFsError::PathChanged);
    }
    Ok(())
}

pub(super) fn revalidate_missing(path: &MissingPathPlan) -> Result<(), WorkspaceFsError> {
    if fs::symlink_metadata(path.target.system()).is_ok() {
        return Err(WorkspaceFsError::PathChanged);
    }
    revalidate_existing(&path.parent)
}

pub(super) fn revalidate_missing_after_staging(
    path: &MissingPathPlan,
) -> Result<(), WorkspaceFsError> {
    if fs::symlink_metadata(path.target.system()).is_ok() {
        return Err(WorkspaceFsError::PathChanged);
    }
    let resolved = ResolvedPath::resolve(path.parent.resolved.native())
        .map_err(|_| WorkspaceFsError::PathChanged)?;
    let current = identity(&resolved)?;
    if resolved != path.parent.resolved
        || current.kind != path.parent.identity.kind
        || current.platform_identity_a != path.parent.identity.platform_identity_a
        || current.platform_identity_b != path.parent.identity.platform_identity_b
    {
        return Err(WorkspaceFsError::PathChanged);
    }
    Ok(())
}

pub(super) fn check_cancel(cancellation: &CancellationToken) -> Result<(), WorkspaceFsError> {
    if cancellation.is_cancelled() {
        Err(WorkspaceFsError::Cancelled)
    } else {
        Ok(())
    }
}
