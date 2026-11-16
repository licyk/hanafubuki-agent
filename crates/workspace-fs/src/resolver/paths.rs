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
    create_parents: bool,
) -> Result<MissingPathPlan, WorkspaceFsError> {
    let target = policy.requested_path(requested)?;
    if fs::symlink_metadata(&target).is_ok() {
        return Err(WorkspaceFsError::InvalidRequest(format!(
            "destination already exists: {}",
            target.display()
        )));
    }
    let mut parent_path = target.parent().ok_or_else(|| {
        WorkspaceFsError::Resolution("target has no parent directory".to_string())
    })?;
    let file_name = target
        .file_name()
        .ok_or_else(|| WorkspaceFsError::InvalidRequest("target has no file name".to_string()))?;
    let mut names = vec![file_name];
    let parent = loop {
        match resolve_existing(
            policy,
            &utf8_path(parent_path)?,
            Some(WorkspacePathKind::Directory),
        ) {
            Ok(parent) => break parent,
            Err(WorkspaceFsError::PathMissing(_)) if create_parents => {
                // A dangling symlink is not a missing directory we can create.
                if fs::symlink_metadata(parent_path).is_ok() {
                    return Err(WorkspaceFsError::PathChanged);
                }
                names.push(parent_path.file_name().ok_or_else(|| {
                    WorkspaceFsError::Resolution("target has no existing ancestor".to_string())
                })?);
                parent_path = parent_path.parent().ok_or_else(|| {
                    WorkspaceFsError::Resolution("target has no existing ancestor".to_string())
                })?;
            }
            Err(error) => return Err(error),
        }
    };
    let mut target = parent.resolved.clone();
    for name in names.into_iter().rev() {
        target = ResolvedPath::resolve_child(&target, name).map_err(resolution_error)?;
    }
    policy.ensure_unprotected(&target)?;
    Ok(MissingPathPlan { target, parent })
}

pub(super) fn resolve_target(
    policy: &WorkspaceFsPolicy,
    requested: &str,
    create_parents: bool,
) -> Result<TargetPlan, WorkspaceFsError> {
    match resolve_existing(policy, requested, None) {
        Ok(path) => Ok(TargetPlan::Existing(path)),
        Err(WorkspaceFsError::PathMissing(_)) => {
            resolve_missing(policy, requested, create_parents).map(TargetPlan::Missing)
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
