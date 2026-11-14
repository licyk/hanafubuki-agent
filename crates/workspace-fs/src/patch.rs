use std::path::PathBuf;

use crate::{PatchHunkStart, WorkspaceFsError, MAX_PATCH_BYTES, MAX_PATCH_FILES};

const BEGIN_PATCH: &str = "*** Begin Patch";
const END_PATCH: &str = "*** End Patch";
const ADD_FILE: &str = "*** Add File: ";
const DELETE_FILE: &str = "*** Delete File: ";
const UPDATE_FILE: &str = "*** Update File: ";
const MOVE_TO: &str = "*** Move to: ";
const END_OF_FILE: &str = "*** End of File";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PatchHunk {
    Add {
        path: PathBuf,
        content: String,
    },
    Delete {
        path: PathBuf,
    },
    Update {
        path: PathBuf,
        move_to: Option<PathBuf>,
        chunks: Vec<PatchChunk>,
    },
}

/// A parsed patch: the file operations that may be applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ParsedPatch {
    pub(crate) hunks: Vec<PatchHunk>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChunkLineKind {
    Added,
    Removed,
    Context,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PatchChunk {
    context: Option<String>,
    old_lines: Vec<String>,
    new_lines: Vec<String>,
    end_of_file: bool,
    /// The chunk's lines in the order the model wrote them.
    ///
    /// `old_lines` and `new_lines` are split for matching, and splitting is
    /// lossy: a context line lands in both and a `+`/`-` run cannot be
    /// reinterleaved afterwards. The interface renders a diff, so it needs the
    /// original sequence — and it must be *this* parser's reading of it, not a
    /// second parser's guess.
    display: Vec<(ChunkLineKind, String)>,
}

impl PatchChunk {
    pub(crate) fn context(&self) -> Option<&str> {
        self.context.as_deref()
    }

    pub(crate) fn display_lines(&self) -> &[(ChunkLineKind, String)] {
        &self.display
    }

    pub(crate) fn is_end_of_file(&self) -> bool {
        self.end_of_file
    }
}

pub(crate) fn parse_patch(input: &str) -> Result<ParsedPatch, WorkspaceFsError> {
    if input.is_empty() || input.len() > MAX_PATCH_BYTES || input.as_bytes().contains(&0) {
        return invalid(format!(
            "patch must contain 1 to {MAX_PATCH_BYTES} bytes without NUL"
        ));
    }
    let normalized = input.replace("\r\n", "\n");
    let lines = normalized.trim().split('\n').collect::<Vec<_>>();
    if lines.first().map(|line| line.trim()) != Some(BEGIN_PATCH) {
        return invalid("the first line must be '*** Begin Patch'");
    }
    if lines.last().map(|line| line.trim()) != Some(END_PATCH) {
        return invalid("the last line must be '*** End Patch'");
    }

    let mut hunks = Vec::new();
    let mut index = 1usize;
    while index + 1 < lines.len() {
        let header = lines[index].trim();
        if header.is_empty() {
            return invalid_at(index + 1, "expected a file operation header");
        }
        if let Some(path) = header.strip_prefix(ADD_FILE) {
            let path = patch_path(path, index + 1)?;
            index += 1;
            let mut content = String::new();
            while index + 1 < lines.len() && !is_file_header(lines[index].trim()) {
                let Some(line) = lines[index].strip_prefix('+') else {
                    return invalid_at(
                        index + 1,
                        "every Add File content line must start with '+'",
                    );
                };
                content.push_str(line);
                content.push('\n');
                index += 1;
            }
            // A hunk with no `+` lines creates an empty file. That is the only
            // way to express one — `+` alone is a blank line, not nothing — and
            // empty files are ordinary (`__init__.py`, `.gitkeep`, `py.typed`).
            hunks.push(PatchHunk::Add { path, content });
        } else if let Some(path) = header.strip_prefix(DELETE_FILE) {
            hunks.push(PatchHunk::Delete {
                path: patch_path(path, index + 1)?,
            });
            index += 1;
        } else if let Some(path) = header.strip_prefix(UPDATE_FILE) {
            let path = patch_path(path, index + 1)?;
            index += 1;
            let mut move_to = None;
            if index + 1 < lines.len() {
                if let Some(destination) = lines[index].trim_end().strip_prefix(MOVE_TO) {
                    move_to = Some(patch_path(destination, index + 1)?);
                    index += 1;
                }
            }
            let mut chunks = Vec::new();
            while index + 1 < lines.len() && !is_update_file_header(lines[index]) {
                let line = lines[index];
                let trimmed = line.trim_end();
                if trimmed == "@@" || trimmed.starts_with("@@ ") {
                    if chunks.last().is_some_and(empty_chunk) {
                        return invalid_at(index + 1, "update chunk does not contain any lines");
                    }
                    chunks.push(PatchChunk {
                        context: trimmed.strip_prefix("@@ ").map(str::to_owned),
                        old_lines: Vec::new(),
                        new_lines: Vec::new(),
                        end_of_file: false,
                        display: Vec::new(),
                    });
                } else if chunks.last().is_some_and(|chunk| chunk.end_of_file) {
                    // After an End of File marker, Codex tolerates only a blank
                    // line (ignored) or a new `@@` context header, which the
                    // branch above already pushed a chunk for. A second
                    // `*** End of File` marker is rejected, as upstream does.
                    if !trimmed.is_empty() {
                        return invalid_at(index + 1, "End of File must finish an update chunk");
                    }
                } else if trimmed == END_OF_FILE {
                    let Some(chunk) = chunks.last_mut() else {
                        return invalid_at(index + 1, "End of File requires an update chunk");
                    };
                    if empty_chunk(chunk) {
                        return invalid_at(index + 1, "update chunk does not contain any lines");
                    }
                    chunk.end_of_file = true;
                } else {
                    if chunks.is_empty() {
                        chunks.push(PatchChunk {
                            context: None,
                            old_lines: Vec::new(),
                            new_lines: Vec::new(),
                            end_of_file: false,
                            display: Vec::new(),
                        });
                    }
                    let chunk = chunks.last_mut().expect("update chunk was inserted");
                    match line.chars().next() {
                        Some(' ') => {
                            let value = line[1..].to_string();
                            chunk.old_lines.push(value.clone());
                            chunk.display.push((ChunkLineKind::Context, value.clone()));
                            chunk.new_lines.push(value);
                        }
                        Some('+') => {
                            let value = line[1..].to_string();
                            chunk.display.push((ChunkLineKind::Added, value.clone()));
                            chunk.new_lines.push(value);
                        }
                        Some('-') => {
                            let value = line[1..].to_string();
                            chunk.display.push((ChunkLineKind::Removed, value.clone()));
                            chunk.old_lines.push(value);
                        }
                        None => {
                            chunk.old_lines.push(String::new());
                            chunk.new_lines.push(String::new());
                            chunk.display.push((ChunkLineKind::Context, String::new()));
                        }
                        _ => {
                            return invalid_at(
                                index + 1,
                                "update lines must start with ' ', '+', '-', or '@@'",
                            );
                        }
                    }
                }
                index += 1;
            }
            if chunks.is_empty() || chunks.last().is_some_and(empty_chunk) {
                return invalid_at(index, "Update File must contain a non-empty change");
            }
            hunks.push(PatchHunk::Update {
                path,
                move_to,
                chunks,
            });
        } else {
            return invalid_at(index + 1, "unknown file operation header");
        }
        if hunks.len() > MAX_PATCH_FILES {
            return invalid(format!("patch may affect at most {MAX_PATCH_FILES} files"));
        }
    }
    if hunks.is_empty() {
        return invalid("patch must contain at least one file operation");
    }
    Ok(ParsedPatch { hunks })
}

/// A file's content after an update, and where each of its chunks landed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UpdatedContent {
    pub(crate) text: String,
    /// One per chunk, in the patch's order.
    pub(crate) hunk_starts: Vec<PatchHunkStart>,
}

pub(crate) fn derive_updated_content(
    original: &str,
    path: &str,
    chunks: &[PatchChunk],
) -> Result<UpdatedContent, WorkspaceFsError> {
    let mut original_lines = original.split('\n').map(str::to_owned).collect::<Vec<_>>();
    if original_lines.last().is_some_and(String::is_empty) {
        original_lines.pop();
    }
    let mut replacements = Vec::<(usize, usize, Vec<String>)>::new();
    let mut cursor = 0usize;
    for chunk in chunks {
        if let Some(context) = &chunk.context {
            let Some(found) = seek_sequence(
                &original_lines,
                std::slice::from_ref(context),
                cursor,
                false,
            ) else {
                return mismatch(format!("failed to find context '{context}' in {path}"));
            };
            cursor = found + 1;
        }
        if chunk.old_lines.is_empty() {
            // Codex semantics: a chunk with no old lines appends at end of
            // file. The cursor only narrows where later chunks may match; it
            // must not decide where a pure addition lands.
            replacements.push((original_lines.len(), 0, chunk.new_lines.clone()));
            continue;
        }
        let mut pattern = chunk.old_lines.as_slice();
        let mut replacement = chunk.new_lines.as_slice();
        let mut found = seek_sequence(&original_lines, pattern, cursor, chunk.end_of_file);
        if found.is_none() && pattern.last().is_some_and(String::is_empty) {
            pattern = &pattern[..pattern.len() - 1];
            if replacement.last().is_some_and(String::is_empty) {
                replacement = &replacement[..replacement.len() - 1];
            }
            found = seek_sequence(&original_lines, pattern, cursor, chunk.end_of_file);
        }
        let Some(found) = found else {
            return mismatch(format!(
                "failed to find expected lines in {path}:\n{}",
                chunk.old_lines.join("\n")
            ));
        };
        replacements.push((found, pattern.len(), replacement.to_vec()));
        cursor = found + pattern.len();
    }
    let hunk_starts = hunk_starts(&replacements);
    for (start, old_len, replacement) in replacements.into_iter().rev() {
        original_lines.splice(start..start + old_len, replacement);
    }
    original_lines.push(String::new());
    Ok(UpdatedContent {
        text: original_lines.join("\n"),
        hunk_starts,
    })
}

/// Each replacement's first line in the original and in the result. A
/// chunk's new position moves by what the replacements before it in the
/// file added or removed; a pure addition lands at the end of the file, so
/// it can come after a later chunk's position.
fn hunk_starts(replacements: &[(usize, usize, Vec<String>)]) -> Vec<PatchHunkStart> {
    let mut in_file: Vec<usize> = (0..replacements.len()).collect();
    in_file.sort_by_key(|&index| (replacements[index].0, index));
    let mut starts = vec![PatchHunkStart::default(); replacements.len()];
    let mut shift = 0isize;
    for index in in_file {
        let (start, old_len, new) = &replacements[index];
        starts[index] = PatchHunkStart {
            old_line: *start as u64 + 1,
            new_line: start.saturating_add_signed(shift) as u64 + 1,
        };
        shift += new.len() as isize - *old_len as isize;
    }
    starts
}

fn seek_sequence(lines: &[String], pattern: &[String], start: usize, eof: bool) -> Option<usize> {
    if pattern.is_empty() {
        return Some(start.min(lines.len()));
    }
    if pattern.len() > lines.len() {
        return None;
    }
    let last = lines.len() - pattern.len();
    if !eof && start > last {
        return None;
    }
    let first = if eof { last } else { start };
    for compare in [
        exact_line as fn(&str, &str) -> bool,
        rstrip_line,
        trimmed_line,
        normalized_line,
    ] {
        for index in first..=last {
            if lines[index..index + pattern.len()]
                .iter()
                .zip(pattern)
                .all(|(left, right)| compare(left, right))
            {
                return Some(index);
            }
        }
    }
    None
}

fn exact_line(left: &str, right: &str) -> bool {
    left == right
}

fn rstrip_line(left: &str, right: &str) -> bool {
    left.trim_end() == right.trim_end()
}

fn trimmed_line(left: &str, right: &str) -> bool {
    left.trim() == right.trim()
}

fn normalized_line(left: &str, right: &str) -> bool {
    normalize_punctuation(left) == normalize_punctuation(right)
}

fn normalize_punctuation(value: &str) -> String {
    value
        .trim()
        .chars()
        .map(|character| match character {
            '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2015}'
            | '\u{2212}' => '-',
            '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{201b}' => '\'',
            '\u{201c}' | '\u{201d}' | '\u{201e}' | '\u{201f}' => '"',
            '\u{00a0}' | '\u{2002}' | '\u{2003}' | '\u{2004}' | '\u{2005}' | '\u{2006}'
            | '\u{2007}' | '\u{2008}' | '\u{2009}' | '\u{200a}' | '\u{202f}' | '\u{205f}'
            | '\u{3000}' => ' ',
            character => character,
        })
        .collect()
}

fn empty_chunk(chunk: &PatchChunk) -> bool {
    chunk.old_lines.is_empty() && chunk.new_lines.is_empty()
}

fn is_file_header(line: &str) -> bool {
    line == END_PATCH
        || line.starts_with(ADD_FILE)
        || line.starts_with(DELETE_FILE)
        || line.starts_with(UPDATE_FILE)
}

fn is_update_file_header(line: &str) -> bool {
    is_file_header(line.trim_end())
}

fn patch_path(value: &str, line: usize) -> Result<PathBuf, WorkspaceFsError> {
    let value = value.trim();
    if value.is_empty() || value.as_bytes().contains(&0) {
        return invalid_at(line, "file path must not be empty");
    }
    Ok(PathBuf::from(value))
}

fn invalid<T>(message: impl Into<String>) -> Result<T, WorkspaceFsError> {
    Err(WorkspaceFsError::InvalidRequest(format!(
        "invalid apply_patch: {}",
        message.into()
    )))
}

fn invalid_at<T>(line: usize, message: impl Into<String>) -> Result<T, WorkspaceFsError> {
    invalid(format!("line {line}: {}", message.into()))
}

fn mismatch<T>(message: impl Into<String>) -> Result<T, WorkspaceFsError> {
    Err(WorkspaceFsError::PreimageMismatch(message.into()))
}

#[cfg(test)]
mod tests;
