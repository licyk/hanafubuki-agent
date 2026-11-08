//! The model's filesystem tool arguments, validated into domain requests.

use hanafubuki_agent_contracts::object_arguments;
use hanafubuki_agent_contracts::tools::arguments::{
    clamped_usize, required_bool, required_bounded_string, required_text, ArgumentNotes,
};
use hanafubuki_agent_contracts::ToolCallValidationError;
use hanafubuki_agent_workspace_fs::{
    ApplyPatchRequest, DeletePathRequest, ListDirectoryRequest, MovePathRequest, ReadFileRequest,
    SearchTextRequest, WorkspaceFsRequest,
};
use serde_json::Value;

use crate::tool::FsToolKind;

/// The bounds of the limit arguments, as the schemas in `specs.rs` state
/// them. A limit the model omits takes the default here; one out of range is
/// clamped into these bounds (never past them) and the result says so.
const MAX_DEPTH: usize = 8;
const MAX_LIST_ENTRIES: usize = 500;
const MAX_SEARCH_RESULTS: usize = 200;
const MAX_SEARCH_BYTES: usize = 4 * 1024 * 1024;
const MAX_READ_OFFSET: usize = 128 * 1024 * 1024;
const MAX_READ_BYTES: usize = 256 * 1024;

/// Parses a call into its domain request and the notes on how its limits
/// were adjusted. Unknown keys are ignored: path resolution and approval are
/// the domain's, so no argument can hand in a pre-approved or pre-resolved
/// target, whatever the model adds.
pub(crate) fn parse(
    kind: FsToolKind,
    raw: &Value,
) -> Result<(WorkspaceFsRequest, ArgumentNotes), ToolCallValidationError> {
    let mut notes = ArgumentNotes::default();
    let arguments = match kind {
        FsToolKind::ApplyPatch => return Ok((apply_patch(raw)?, notes)),
        _ => object_arguments(raw)?,
    };
    let request = match kind {
        FsToolKind::ListDirectory => WorkspaceFsRequest::ListDirectory(ListDirectoryRequest {
            path: required_bounded_string(arguments, "path", 4096)?,
            max_depth: clamped_usize(arguments, "maxDepth", 1, MAX_DEPTH, 2, &mut notes)?,
            max_entries: clamped_usize(
                arguments,
                "maxEntries",
                1,
                MAX_LIST_ENTRIES,
                200,
                &mut notes,
            )?,
        }),
        FsToolKind::SearchText => WorkspaceFsRequest::SearchText(SearchTextRequest {
            path: required_bounded_string(arguments, "path", 4096)?,
            query: required_bounded_string(arguments, "query", 1024)?,
            max_depth: clamped_usize(arguments, "maxDepth", 1, MAX_DEPTH, MAX_DEPTH, &mut notes)?,
            max_results: clamped_usize(
                arguments,
                "maxResults",
                1,
                MAX_SEARCH_RESULTS,
                50,
                &mut notes,
            )?,
            max_bytes: clamped_usize(
                arguments,
                "maxBytes",
                1,
                MAX_SEARCH_BYTES,
                1024 * 1024,
                &mut notes,
            )?,
        }),
        FsToolKind::ReadFile => WorkspaceFsRequest::ReadFile(ReadFileRequest {
            path: required_bounded_string(arguments, "path", 4096)?,
            offset: clamped_usize(arguments, "offset", 0, MAX_READ_OFFSET, 0, &mut notes)?,
            max_bytes: clamped_usize(
                arguments,
                "maxBytes",
                4,
                MAX_READ_BYTES,
                64 * 1024,
                &mut notes,
            )?,
        }),
        FsToolKind::MovePath => WorkspaceFsRequest::MovePath(MovePathRequest {
            source: required_bounded_string(arguments, "source", 4096)?,
            destination: required_bounded_string(arguments, "destination", 4096)?,
        }),
        FsToolKind::DeletePath => WorkspaceFsRequest::DeletePath(DeletePathRequest {
            path: required_bounded_string(arguments, "path", 4096)?,
            // Destructive intent stays explicit: no default.
            recursive: required_bool(arguments, "recursive")?,
        }),
        FsToolKind::ApplyPatch => apply_patch(raw)?,
    };
    Ok((request, notes))
}

/// `apply_patch` is a freeform tool where the provider supports one: the
/// model then sends the patch itself instead of `{"patch": ...}`.
pub(crate) fn apply_patch(
    arguments: &Value,
) -> Result<WorkspaceFsRequest, ToolCallValidationError> {
    let patch = match arguments {
        Value::String(patch) => patch.clone(),
        Value::Object(arguments) => required_text(arguments, "patch", 256 * 1024, false)?,
        _ => return Err(ToolCallValidationError::ArgumentsMustBeObject),
    };
    if patch.is_empty() || patch.len() > 256 * 1024 || patch.as_bytes().contains(&0) {
        return Err(ToolCallValidationError::InvalidArgument("patch"));
    }
    Ok(WorkspaceFsRequest::ApplyPatch(ApplyPatchRequest { patch }))
}

#[cfg(test)]
mod tests;
