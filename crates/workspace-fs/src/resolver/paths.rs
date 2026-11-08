use super::*;

pub(crate) fn resolve_existing(
    policy: &WorkspaceFsPolicy,
    requested: &str,
    expected_kind: Option<WorkspacePathKind>,
) -> Result<ExistingPathPlan, WorkspaceFsError> {
    let unresolved = policy.requested_path(requested)?;
    let resolved = ResolvedPath::resolve(&unresolved).map_err(|error| {
        // Keep "does not exist" distinct from every other failure. A caller
        // that retries as a create must not also swallow a permission error or
        // a disconnected drive and report it to the model as "missing".
        if error.is_not_found() {
            WorkspaceFsError::PathMissing(unresolved.display().to_string())
        } else {
            WorkspaceFsError::Resolution(error.to_string())
        }
    })?;
    policy.ensure_unprotected(&resolved)?;
    let identity = identity(&resolved)?;
    if expected_kind.is_some_and(|expected| identity.kind != expected) {
        return Err(WorkspaceFsError::InvalidRequest(format!(
            "{} is not a {:?}",
            identity.path,
            expected_kind.unwrap()
        )));
    }
    Ok(ExistingPathPlan { resolved, identity })
}

pub(crate) fn resolve_missing(
    policy: &WorkspaceFsPolicy,
    requested: &str,
) -> Result<MissingPathPlan, WorkspaceFsError> {
    let target = policy.requested_path(requested)?;
    if fs::symlink_metadata(&target).is_ok() {
        return Err(WorkspaceFsError::InvalidRequest(format!(
            "destination already exists: {}",
            target.display()
        )));
    }
    let parent_path = target.parent().ok_or_else(|| {
        WorkspaceFsError::Resolution("target has no parent directory".to_string())
    })?;
    let parent_text = utf8_path(parent_path)?;
    let parent = resolve_existing(policy, &parent_text, Some(WorkspacePathKind::Directory))?;
    let file_name = target
        .file_name()
        .ok_or_else(|| WorkspaceFsError::InvalidRequest("target has no file name".to_string()))?;
    // Built from the resolved parent, so the child inherits the parent's system
    // form for I/O and its native form for display without a second lookup.
    let target =
        ResolvedPath::resolve_child(&parent.resolved, file_name).map_err(resolution_error)?;
    policy.ensure_unprotected(&target)?;
    Ok(MissingPathPlan { target, parent })
}

pub(super) fn resolve_target(
    policy: &WorkspaceFsPolicy,
    requested: &str,
) -> Result<TargetPlan, WorkspaceFsError> {
    match resolve_existing(policy, requested, None) {
        Ok(path) => Ok(TargetPlan::Existing(path)),
        Err(WorkspaceFsError::PathMissing(_)) => {
            resolve_missing(policy, requested).map(TargetPlan::Missing)
        }
        Err(error) => Err(error),
    }
}

pub(crate) fn target_resolved(target: &TargetPlan) -> &ResolvedPath {
    match target {
        TargetPlan::Existing(path) => &path.resolved,
        TargetPlan::Missing(path) => &path.target,
    }
}

pub(super) fn target_path_text(target: &TargetPlan) -> &str {
    match target {
        TargetPlan::Existing(path) => &path.identity.path,
        TargetPlan::Missing(path) => path.target.native_text(),
    }
}

pub(crate) fn utf8_path(path: &Path) -> Result<String, WorkspaceFsError> {
    path.to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| WorkspaceFsError::Resolution("path is not valid UTF-8".to_string()))
}
