//! A presentation projection of a Codex patch, produced by the same parser that
//! decides whether the patch may be applied.
//!
//! The interface used to carry its own patch reader. It was lenient where this
//! one is strict, so a patch the backend rejected could still render as a clean
//! diff — the user saw `+1 −1` on a file while the tool reported a mismatch,
//! with nothing on screen to explain the contradiction. One format deserves one
//! parser; this module is how the reading gets to the interface.
//!
//! Projection only. It reports what the patch *says*, never whether the paths
//! resolve, whether they are in the workspace, or whether the preimage matches
//! — those need the policy and the filesystem, and they belong to `prepare`.

use serde::{Deserialize, Serialize};

use crate::patch::{parse_patch, ChunkLineKind, PatchHunk};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchPreviewOperation {
    Add,
    Update,
    Delete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchPreviewLineKind {
    Added,
    Removed,
    Context,
    /// A `@@` chunk header, with any context text the model supplied.
    Hunk,
    /// A structural note from the patch itself, such as end-of-file.
    Marker,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchPreviewLine {
    pub kind: PatchPreviewLineKind,
    /// Without its diff marker, so a reader copies the real content.
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchPreviewFile {
    pub operation: PatchPreviewOperation,
    /// Exactly the header the model wrote. Not resolved: resolution needs the
    /// workspace policy, and a preview must render a patch that would be
    /// rejected just as faithfully as one that would apply.
    pub path: String,
    /// Rename destination when the hunk carried `*** Move to:`.
    pub moved_to: Option<String>,
    pub lines: Vec<PatchPreviewLine>,
    pub added: usize,
    pub removed: usize,
    /// Lines beyond the painting cap; zero when the whole file is shown.
    pub omitted_lines: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchPreview {
    pub files: Vec<PatchPreviewFile>,
    /// Why the patch could not be read as a diff. When set, `files` is empty and
    /// the caller should show the raw text — hiding a malformed patch would be
    /// worse than showing what the model actually asked for.
    pub error: Option<String>,
}

/// Painting cap per file. The patch text itself is already bounded upstream.
const MAX_PREVIEW_LINES: usize = 500;

/// Read a patch into per-file diffs using the authoritative parser.
///
/// A patch this parser rejects yields an empty preview carrying the parser's
/// own message, so the interface and the tool tell the user the same story.
pub fn preview_patch(patch: &str) -> PatchPreview {
    let parsed = match parse_patch(patch) {
        Ok(parsed) => parsed,
        Err(error) => {
            return PatchPreview {
                files: Vec::new(),
                error: Some(error.to_string()),
            }
        }
    };

    let files = parsed
        .hunks
        .into_iter()
        .map(|hunk| match hunk {
            PatchHunk::Add { path, content } => {
                let mut file = new_file(PatchPreviewOperation::Add, &path, None);
                // `content` is the reconstructed file body; every line of it is
                // an addition. An Add with no `+` lines creates an empty file,
                // and an empty body has no lines at all — splitting "" would
                // otherwise invent one blank one.
                if !content.is_empty() {
                    for line in content.strip_suffix('\n').unwrap_or(&content).split('\n') {
                        push(&mut file, PatchPreviewLineKind::Added, line);
                    }
                }
                file
            }
            PatchHunk::Delete { path } => new_file(PatchPreviewOperation::Delete, &path, None),
            PatchHunk::Update {
                path,
                move_to,
                chunks,
            } => {
                let mut file = new_file(
                    PatchPreviewOperation::Update,
                    &path,
                    move_to
                        .as_deref()
                        .map(|path| path.to_string_lossy().into_owned()),
                );
                for chunk in chunks {
                    push(
                        &mut file,
                        PatchPreviewLineKind::Hunk,
                        chunk.context().unwrap_or(""),
                    );
                    for (kind, text) in chunk.display_lines() {
                        let kind = match kind {
                            ChunkLineKind::Added => PatchPreviewLineKind::Added,
                            ChunkLineKind::Removed => PatchPreviewLineKind::Removed,
                            ChunkLineKind::Context => PatchPreviewLineKind::Context,
                        };
                        push(&mut file, kind, text);
                    }
                    if chunk.is_end_of_file() {
                        push(&mut file, PatchPreviewLineKind::Marker, "*** End of File");
                    }
                }
                file
            }
        })
        .collect();

    PatchPreview { files, error: None }
}

fn new_file(
    operation: PatchPreviewOperation,
    path: &std::path::Path,
    moved_to: Option<String>,
) -> PatchPreviewFile {
    PatchPreviewFile {
        operation,
        path: path.to_string_lossy().into_owned(),
        moved_to,
        lines: Vec::new(),
        added: 0,
        removed: 0,
        omitted_lines: 0,
    }
}

fn push(file: &mut PatchPreviewFile, kind: PatchPreviewLineKind, text: &str) {
    match kind {
        PatchPreviewLineKind::Added => file.added += 1,
        PatchPreviewLineKind::Removed => file.removed += 1,
        _ => {}
    }
    if file.lines.len() >= MAX_PREVIEW_LINES {
        file.omitted_lines += 1;
        return;
    }
    file.lines.push(PatchPreviewLine {
        kind,
        text: text.to_string(),
    });
}

#[cfg(test)]
mod tests;
