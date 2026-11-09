use super::*;

impl WorkspaceFsService {
    pub(super) fn list_directory(
        &self,
        root: ExistingPathPlan,
        max_depth: usize,
        max_entries: usize,
        cancellation: &CancellationToken,
    ) -> Result<WorkspaceFsResult, WorkspaceFsError> {
        revalidate_existing(&root)?;
        let mut stack = vec![(root.clone(), 0usize)];
        let mut entries = Vec::new();
        let mut truncated = false;
        while let Some((directory_plan, parent_depth)) = stack.pop() {
            check_cancel(cancellation)?;
            revalidate_existing(&directory_plan)?;
            let directory = &directory_plan.resolved;
            let mut children = fs::read_dir(directory.system())
                .map_err(|error| {
                    WorkspaceFsError::Io(format!("list {}: {error}", directory.native_text()))
                })?
                .map(|entry| entry.map(|entry| entry.file_name()))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| {
                    WorkspaceFsError::Io(format!("list {}: {error}", directory.native_text()))
                })?;
            children.sort();
            let mut directories = Vec::new();
            for name in children {
                check_cancel(cancellation)?;
                if entries.len() >= max_entries {
                    truncated = true;
                    break;
                }
                // Derived from the already-resolved parent rather than
                // canonicalized again: one lookup, and the child inherits both
                // the parent's system form for I/O and its native form for
                // display.
                let child = ResolvedPath::resolve_child(directory, &name)
                    .map_err(|error| WorkspaceFsError::Resolution(error.to_string()))?;
                let metadata = fs::symlink_metadata(child.system()).map_err(|error| {
                    WorkspaceFsError::Io(format!("inspect {}: {error}", child.native_text()))
                })?;
                if metadata.file_type().is_symlink() {
                    truncated = true;
                    continue;
                }
                let depth = parent_depth + 1;
                let kind = if metadata.is_file() {
                    WorkspacePathKind::File
                } else if metadata.is_dir() {
                    WorkspacePathKind::Directory
                } else {
                    truncated = true;
                    continue;
                };
                self.policy().ensure_unprotected(&child)?;
                let directory_identity = if kind == WorkspacePathKind::Directory {
                    Some(identity_from_metadata(&child, &metadata)?)
                } else {
                    None
                };
                entries.push(DirectoryEntry {
                    path: child.native_text().to_string(),
                    kind,
                    size: metadata.len(),
                    depth,
                });
                if kind == WorkspacePathKind::Directory {
                    if depth < max_depth {
                        directories.push((
                            ExistingPathPlan {
                                resolved: child,
                                identity: directory_identity.expect("directory identity"),
                            },
                            depth,
                        ));
                    } else if fs::read_dir(child.system())
                        .map(|mut entries| entries.next().is_some())
                        .unwrap_or(true)
                    {
                        truncated = true;
                    }
                }
            }
            if truncated && entries.len() >= max_entries {
                break;
            }
            for child in directories.into_iter().rev() {
                stack.push(child);
            }
        }
        Ok(WorkspaceFsResult::DirectoryListing(DirectoryListing {
            root: root.identity.path,
            entries,
            truncated,
        }))
    }
}
