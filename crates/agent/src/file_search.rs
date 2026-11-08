use std::path::{Path, PathBuf};

use hanafubuki_agent_contracts::TurnExtensions;
use ignore::WalkBuilder;

use super::AgentApplicationService;
use serde::Serialize;

const MAXIMUM_FILE_QUERY_BYTES: usize = 512;
const MAXIMUM_FILE_RESULTS: usize = 50;
const MAXIMUM_WALK_ENTRIES: usize = 10_000;

/// A file search over the roots the host's workspace port names for `host`.
pub struct AgentFileSearch {
    pub query: String,
    /// What the host submitted with the search.
    pub host: TurnExtensions,
    pub limit: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentFileCandidate {
    pub path: String,
    pub label: String,
    pub root_label: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentFileSearchResult {
    pub candidates: Vec<AgentFileCandidate>,
    pub truncated: bool,
}

impl AgentApplicationService {
    pub fn search_user_files(
        &self,
        request: AgentFileSearch,
    ) -> Result<AgentFileSearchResult, String> {
        if request.query.len() > MAXIMUM_FILE_QUERY_BYTES || request.query.as_bytes().contains(&0) {
            return Err(format!(
                "file query must contain at most {MAXIMUM_FILE_QUERY_BYTES} bytes without NUL"
            ));
        }
        if request.limit == 0 || request.limit > MAXIMUM_FILE_RESULTS {
            return Err(format!(
                "file result limit must be between 1 and {MAXIMUM_FILE_RESULTS}"
            ));
        }
        let roots = self.workspace.search_roots(&request.host)?;
        search_roots(&roots, &request.query, request.limit)
    }
}

fn search_roots(
    roots: &[(String, PathBuf)],
    query: &str,
    limit: usize,
) -> Result<AgentFileSearchResult, String> {
    let needle = query.to_lowercase();
    let mut candidates = Vec::new();
    let mut visited = 0usize;
    let mut truncated = false;
    'roots: for (root_label, root) in roots {
        let walker = WalkBuilder::new(root)
            .hidden(false)
            .git_ignore(true)
            .git_exclude(true)
            .ignore(true)
            .parents(true)
            .require_git(false)
            .max_depth(Some(12))
            .build();
        for entry in walker {
            visited = visited.saturating_add(1);
            if visited > MAXIMUM_WALK_ENTRIES {
                truncated = true;
                break 'roots;
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    truncated = true;
                    continue;
                }
            };
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            let relative = entry.path().strip_prefix(root).unwrap_or(entry.path());
            let label = label_text(relative)?;
            if !needle.is_empty() && !label.to_lowercase().contains(&needle) {
                continue;
            }
            candidates.push(AgentFileCandidate {
                path: path_text(entry.path())?,
                label,
                root_label: root_label.clone(),
            });
            if candidates.len() >= limit {
                truncated = true;
                break 'roots;
            }
        }
    }
    candidates.sort_by(|left, right| {
        let left_name = Path::new(&left.label)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or(&left.label)
            .to_lowercase();
        let right_name = Path::new(&right.label)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or(&right.label)
            .to_lowercase();
        let left_rank = (
            !left_name.starts_with(&needle),
            left.label.len(),
            &left.label,
        );
        let right_rank = (
            !right_name.starts_with(&needle),
            right.label.len(),
            &right.label,
        );
        left_rank.cmp(&right_rank)
    });
    Ok(AgentFileSearchResult {
        candidates,
        truncated,
    })
}

fn path_text(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| "file candidate path is not valid UTF-8".to_string())
}

fn label_text(path: &Path) -> Result<String, String> {
    path_text(path).map(|value| value.replace('\\', "/"))
}

#[cfg(test)]
mod tests;
