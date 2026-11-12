//! How an interface shows the filesystem tools' calls: the path or query
//! each acts on, and what it read, listed, found or changed. A patch is
//! read by the parser that applies it, so a rejected patch shows as the
//! tool read it.

use hanafubuki_agent_contracts::{
    structured_output, AgentListingEntry, AgentListingEntryKind, AgentMutationKind, AgentPatchFile,
    AgentPatchLine, AgentPatchLineKind, AgentPatchOperation, AgentPatchPreview, AgentSearchMatch,
    AgentToolDisplay, AgentToolPresenter,
};
use hanafubuki_agent_tool_contracts::ToolOutput;
use hanafubuki_agent_workspace_fs::{
    preview_patch, PatchPreview, PatchPreviewLineKind, PatchPreviewOperation, WorkspaceFsResult,
    WorkspacePathKind,
};
use serde_json::Value;

pub(crate) struct FsPresenter;

const LIST_DIRECTORY: &str = "list_directory";
const SEARCH_TEXT: &str = "search_text";
const READ_FILE: &str = "read_file";
const APPLY_PATCH: &str = "apply_patch";
const MOVE_PATH: &str = "move_path";
const DELETE_PATH: &str = "delete_path";

impl AgentToolPresenter for FsPresenter {
    fn tools(&self) -> &[&'static str] {
        &[
            LIST_DIRECTORY,
            SEARCH_TEXT,
            READ_FILE,
            APPLY_PATCH,
            MOVE_PATH,
            DELETE_PATH,
        ]
    }

    fn subject(&self, tool: &str, arguments: &Value) -> Option<String> {
        match tool {
            LIST_DIRECTORY | READ_FILE | DELETE_PATH => text(arguments, "path").map(str::to_string),
            SEARCH_TEXT => {
                let query = text(arguments, "query")?;
                Some(match text(arguments, "path") {
                    Some(path) => format!("\"{query}\" in {path}"),
                    None => format!("\"{query}\""),
                })
            }
            APPLY_PATCH => patch_subject(&preview_patch(patch(arguments)?)),
            MOVE_PATH => Some(format!(
                "{} → {}",
                text(arguments, "source")?,
                text(arguments, "destination")?
            )),
            _ => None,
        }
    }

    /// A patch shows its diff whether or not it applied; the other tools
    /// what their result says, and the fallback for an error.
    fn display(
        &self,
        tool: &str,
        arguments: &Value,
        output: &ToolOutput,
    ) -> Option<AgentToolDisplay> {
        if tool == APPLY_PATCH {
            return Some(AgentToolDisplay::Patch {
                preview: agent_preview(preview_patch(patch(arguments)?)),
            });
        }
        let result: WorkspaceFsResult =
            serde_json::from_value(structured_output(output)?.clone()).ok()?;
        Some(match result {
            WorkspaceFsResult::FileRead(read) => AgentToolDisplay::FileRead {
                path: read.path,
                // A read recorded before the reader counted lines knows its
                // first line's number only from the start.
                start_line: read.start_line.or((read.offset == 0).then_some(1)),
                text: read.content,
                truncated: read.truncated,
            },
            WorkspaceFsResult::DirectoryListing(listing) => AgentToolDisplay::Listing {
                path: listing.root,
                entries: listing
                    .entries
                    .into_iter()
                    .map(|entry| AgentListingEntry {
                        path: entry.path,
                        size: (entry.kind == WorkspacePathKind::File).then_some(entry.size),
                        kind: match entry.kind {
                            WorkspacePathKind::File => AgentListingEntryKind::File,
                            WorkspacePathKind::Directory => AgentListingEntryKind::Directory,
                        },
                    })
                    .collect(),
                truncated: listing.truncated,
            },
            WorkspaceFsResult::SearchResults(results) => AgentToolDisplay::Search {
                query: text(arguments, "query").unwrap_or_default().to_string(),
                path: Some(results.root),
                matches: results
                    .matches
                    .into_iter()
                    .map(|found| AgentSearchMatch {
                        path: found.path,
                        line: found.line as u64,
                        text: found.preview,
                    })
                    .collect(),
                truncated: results.truncated,
            },
            WorkspaceFsResult::PathMoved(moved) => AgentToolDisplay::Mutation {
                kind: AgentMutationKind::Moved,
                paths: vec![moved.source, moved.destination],
            },
            WorkspaceFsResult::PathDeleted(deleted) => AgentToolDisplay::Mutation {
                kind: AgentMutationKind::Deleted,
                paths: vec![deleted.path],
            },
            WorkspaceFsResult::FileWritten(_) | WorkspaceFsResult::PatchApplied(_) => return None,
        })
    }
}

fn text<'a>(arguments: &'a Value, key: &str) -> Option<&'a str> {
    arguments
        .get(key)?
        .as_str()
        .filter(|text| !text.trim().is_empty())
}

/// A freeform call's patch is the arguments themselves; a function call's
/// is its `patch`.
fn patch(arguments: &Value) -> Option<&str> {
    match arguments {
        Value::String(patch) => Some(patch),
        arguments => arguments.get("patch")?.as_str(),
    }
}

/// `src/lib.rs`, or `src/lib.rs and 2 more`.
fn patch_subject(preview: &PatchPreview) -> Option<String> {
    let first = preview.files.first()?;
    Some(match preview.files.len() {
        1 => first.path.clone(),
        files => format!("{} and {} more", first.path, files - 1),
    })
}

fn agent_preview(preview: PatchPreview) -> AgentPatchPreview {
    AgentPatchPreview {
        files: preview
            .files
            .into_iter()
            .map(|file| AgentPatchFile {
                operation: match file.operation {
                    PatchPreviewOperation::Add => AgentPatchOperation::Add,
                    PatchPreviewOperation::Update => AgentPatchOperation::Update,
                    PatchPreviewOperation::Delete => AgentPatchOperation::Delete,
                },
                path: file.path,
                moved_to: file.moved_to,
                lines: file
                    .lines
                    .into_iter()
                    .map(|line| AgentPatchLine {
                        kind: match line.kind {
                            PatchPreviewLineKind::Added => AgentPatchLineKind::Added,
                            PatchPreviewLineKind::Removed => AgentPatchLineKind::Removed,
                            PatchPreviewLineKind::Context => AgentPatchLineKind::Context,
                            PatchPreviewLineKind::Hunk => AgentPatchLineKind::Hunk,
                            PatchPreviewLineKind::Marker => AgentPatchLineKind::Marker,
                        },
                        text: line.text,
                    })
                    .collect(),
                added: file.added,
                removed: file.removed,
                omitted_lines: file.omitted_lines,
            })
            .collect(),
        error: preview.error,
    }
}

#[cfg(test)]
mod tests;
