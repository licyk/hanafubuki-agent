use super::*;

pub(crate) fn prepare_operation(
    policy: &WorkspaceFsPolicy,
    request: &WorkspaceFsRequest,
) -> Result<PreparedWorkspaceOperation, WorkspaceFsError> {
    let (plan, action_kind, risk, summary, canonical) = match request {
        WorkspaceFsRequest::ListDirectory(request) => {
            validate_depth_entries(request.max_depth, request.max_entries)?;
            let root = resolve_existing(policy, &request.path, Some(WorkspacePathKind::Directory))?;
            let canonical = serialize_action(&CanonicalFileAction::ListDirectory {
                root: &root.identity,
                max_depth: request.max_depth,
                max_entries: request.max_entries,
            })?;
            let summary = format!("List directory {}", root.identity.path);
            (
                WorkspaceOperationPlan::ListDirectory {
                    root,
                    max_depth: request.max_depth,
                    max_entries: request.max_entries,
                },
                ApprovalActionKind::FileRead,
                ApprovalRiskClass::Low,
                summary,
                canonical,
            )
        }
        WorkspaceFsRequest::SearchText(request) => {
            validate_depth_entries(request.max_depth, request.max_results)?;
            if request.query.is_empty()
                || request.query.len() > MAX_SEARCH_QUERY_BYTES
                || request.query.as_bytes().contains(&0)
            {
                return Err(WorkspaceFsError::InvalidRequest(format!(
                    "search query must contain 1 to {MAX_SEARCH_QUERY_BYTES} bytes without NUL"
                )));
            }
            if request.max_results == 0 || request.max_results > MAX_SEARCH_RESULTS {
                return Err(WorkspaceFsError::InvalidRequest(format!(
                    "search results must be between 1 and {MAX_SEARCH_RESULTS}"
                )));
            }
            if request.max_bytes == 0 || request.max_bytes > MAX_SEARCH_BYTES {
                return Err(WorkspaceFsError::InvalidRequest(format!(
                    "search bytes must be between 1 and {MAX_SEARCH_BYTES}"
                )));
            }
            let matcher =
                grep_regex::RegexMatcher::new_line_matcher(&request.query).map_err(|error| {
                    WorkspaceFsError::InvalidRequest(format!(
                        "invalid search regular expression: {error}"
                    ))
                })?;
            let root = resolve_existing(policy, &request.path, Some(WorkspacePathKind::Directory))?;
            let canonical = serialize_action(&CanonicalFileAction::SearchText {
                root: &root.identity,
                query: &request.query,
                max_depth: request.max_depth,
                max_results: request.max_results,
                max_bytes: request.max_bytes,
            })?;
            let summary = format!(
                "Search text in {} (query {} bytes)",
                root.identity.path,
                request.query.len()
            );
            (
                WorkspaceOperationPlan::SearchText {
                    root,
                    matcher,
                    max_depth: request.max_depth,
                    max_results: request.max_results,
                    max_bytes: request.max_bytes,
                },
                ApprovalActionKind::FileRead,
                ApprovalRiskClass::Medium,
                summary,
                canonical,
            )
        }
        WorkspaceFsRequest::ReadFile(request) => {
            if request.max_bytes < MIN_READ_PAGE_BYTES || request.max_bytes > MAX_READ_PAGE_BYTES {
                return Err(WorkspaceFsError::InvalidRequest(format!(
                    "read page bytes must be between {MIN_READ_PAGE_BYTES} and {MAX_READ_PAGE_BYTES}"
                )));
            }
            let file = resolve_existing(policy, &request.path, Some(WorkspacePathKind::File))?;
            if file.identity.size > MAX_READ_FILE_BYTES {
                return Err(WorkspaceFsError::LimitExceeded(format!(
                    "read file exceeds {MAX_READ_FILE_BYTES} bytes"
                )));
            }
            if request.offset as u64 > file.identity.size {
                return Err(WorkspaceFsError::InvalidRequest(format!(
                    "read offset {} exceeds file size {}",
                    request.offset, file.identity.size
                )));
            }
            let canonical = serialize_action(&CanonicalFileAction::ReadFile {
                file: &file.identity,
                offset: request.offset,
                max_bytes: request.max_bytes,
            })?;
            let summary = format!("Read file {}", file.identity.path);
            (
                WorkspaceOperationPlan::ReadFile {
                    file,
                    offset: request.offset,
                    max_bytes: request.max_bytes,
                },
                ApprovalActionKind::FileRead,
                ApprovalRiskClass::Medium,
                summary,
                canonical,
            )
        }
        WorkspaceFsRequest::WriteFile(request) => {
            validate_content(&request.content)?;
            let target = resolve_target(policy, &request.path)?;
            if matches!(target, TargetPlan::Existing(_)) && !request.overwrite {
                return Err(WorkspaceFsError::InvalidRequest(
                    "target exists and overwrite is false".to_string(),
                ));
            }
            if matches!(&target, TargetPlan::Existing(path) if path.identity.kind != WorkspacePathKind::File)
            {
                return Err(WorkspaceFsError::InvalidRequest(
                    "write target must be a file".to_string(),
                ));
            }
            let content = request.content.as_bytes().to_vec();
            let content_sha256 = sha256_hex(&content);
            let canonical_target = canonical_target(&target);
            let canonical = serialize_action(&CanonicalFileAction::WriteFile {
                target: canonical_target,
                content_sha256: &content_sha256,
                content_bytes: content.len(),
                overwrite: request.overwrite,
            })?;
            let summary = format!(
                "Write {} bytes to {}",
                content.len(),
                target_path_text(&target)
            );
            (
                WorkspaceOperationPlan::WriteFile {
                    target,
                    content,
                    content_sha256,
                    overwrite: request.overwrite,
                },
                ApprovalActionKind::FileWrite,
                ApprovalRiskClass::High,
                summary,
                canonical,
            )
        }
        WorkspaceFsRequest::ApplyPatch(request) => {
            let PreparedPatch {
                changes,
                external_paths,
            } = prepare_patch_changes(policy, &request.patch)?;
            let patch_sha256 = sha256_hex(request.patch.as_bytes());
            // The external paths are part of what is approved, not a note
            // beside it: an approval granted for an in-workspace patch must not
            // be replayable against one that reaches outside.
            let canonical = serialize_action(&CanonicalFileAction::ApplyPatch {
                patch_sha256: &patch_sha256,
                changes: changes.iter().map(canonical_patch_change).collect(),
                external_paths: &external_paths,
            })?;
            let summary = if external_paths.is_empty() {
                format!(
                    "Apply Codex patch to {} file{}",
                    changes.len(),
                    if changes.len() == 1 { "" } else { "s" }
                )
            } else {
                // Name the escape explicitly. Whoever approves this is being
                // asked for more than an ordinary workspace edit.
                format!(
                    "Apply Codex patch to {} file{}, writing outside the workspace: {}",
                    changes.len(),
                    if changes.len() == 1 { "" } else { "s" },
                    external_paths
                        .iter()
                        .map(|report| report.resolved.clone())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            let risk = if external_paths.is_empty() {
                ApprovalRiskClass::High
            } else {
                ApprovalRiskClass::Critical
            };
            (
                WorkspaceOperationPlan::ApplyPatch { changes },
                ApprovalActionKind::FilePatch,
                risk,
                summary,
                canonical,
            )
        }
        WorkspaceFsRequest::MovePath(request) => {
            let source = resolve_existing(policy, &request.source, None)?;
            let destination = resolve_missing(policy, &request.destination)?;
            let destination_digest = sha256_hex(&destination.target.key().digest_bytes());
            let canonical = serialize_action(&CanonicalFileAction::MovePath {
                source: &source.identity,
                destination: CanonicalMissingTarget {
                    path_digest: destination_digest,
                    parent: &destination.parent.identity,
                },
            })?;
            let summary = format!(
                "Move {} to {}",
                source.identity.path,
                destination.target.native_text()
            );
            (
                WorkspaceOperationPlan::MovePath {
                    source,
                    destination,
                },
                ApprovalActionKind::FileMove,
                ApprovalRiskClass::High,
                summary,
                canonical,
            )
        }
        WorkspaceFsRequest::DeletePath(request) => {
            let target = resolve_existing(policy, &request.path, None)?;
            if target.identity.kind == WorkspacePathKind::Directory && !request.recursive {
                // Non-recursive delete is deliberately allowed only for an empty
                // directory; execution verifies emptiness using remove_dir.
            }
            let canonical = serialize_action(&CanonicalFileAction::DeletePath {
                target: &target.identity,
                recursive: request.recursive,
            })?;
            let summary = if request.recursive {
                format!("Recursively delete {}", target.identity.path)
            } else {
                format!("Delete {}", target.identity.path)
            };
            (
                WorkspaceOperationPlan::DeletePath {
                    target,
                    recursive: request.recursive,
                },
                ApprovalActionKind::FileDelete,
                if request.recursive {
                    ApprovalRiskClass::Critical
                } else {
                    ApprovalRiskClass::High
                },
                summary,
                canonical,
            )
        }
    };
    let mut canonical_bytes = b"agent-workspace-fs-v1\0".to_vec();
    canonical_bytes.extend(canonical);
    let policy_paths = operation_policy_paths(&plan);
    let action = ApprovalAction::from_canonical_bytes(
        action_kind,
        risk,
        ApprovalOrigin::Model,
        summary,
        &canonical_bytes,
    )
    .and_then(|action| {
        action.with_details(PermissionActionDetails::Paths {
            canonical_paths: policy_paths,
        })
    })
    .map_err(|error| WorkspaceFsError::Resolution(error.to_string()))?;
    Ok(PreparedWorkspaceOperation { plan, action })
}

fn validate_depth_entries(depth: usize, entries: usize) -> Result<(), WorkspaceFsError> {
    if depth == 0 || depth > MAX_DIRECTORY_DEPTH {
        return Err(WorkspaceFsError::InvalidRequest(format!(
            "directory depth must be between 1 and {MAX_DIRECTORY_DEPTH}"
        )));
    }
    if entries == 0 || entries > MAX_LIST_ENTRIES {
        return Err(WorkspaceFsError::InvalidRequest(format!(
            "entry count must be between 1 and {MAX_LIST_ENTRIES}"
        )));
    }
    Ok(())
}

pub(super) fn validate_content(content: &str) -> Result<(), WorkspaceFsError> {
    if content.len() > MAX_WRITE_BYTES || content.as_bytes().contains(&0) {
        return Err(WorkspaceFsError::InvalidRequest(format!(
            "text content must not contain NUL or exceed {MAX_WRITE_BYTES} bytes"
        )));
    }
    Ok(())
}
