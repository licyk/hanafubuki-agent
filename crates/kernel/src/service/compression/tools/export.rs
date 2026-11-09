// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `toFilePointer` in `src/decompress-shared.ts` at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Modified for hanafubuki-agent: `toFile` asks for the session's export
// file; an `acp_retrieve` export the history store cannot write comes back
// inline.

//! The text a compression tool writes to one of the session's export files
//! instead of returning it: a `decompress` with `toFile`, or an
//! `acp_retrieve` of an original too large to inline.

use hanafubuki_agent_compression::{
    build_retrieval_pointer, frame_retrieved_original, StoredEntry,
};

use super::*;

/// A tool's text bound for one of the session's export files.
pub(in crate::service) struct ToolExport {
    pub(in crate::service) file_name: String,
    body: String,
    kind: ExportKind,
}

enum ExportKind {
    /// A `decompress` restore, introduced by its header.
    Restore { header: String },
    /// A stored original `acp_retrieve` returns.
    Retrieval {
        r#ref: String,
        entry: StoredEntry,
        lines: usize,
    },
}

impl ToolExport {
    pub(super) fn restore(file_name: String, header: String, body: String) -> Self {
        Self {
            file_name,
            body,
            kind: ExportKind::Restore { header },
        }
    }

    pub(super) fn retrieval(
        file_name: String,
        r#ref: String,
        entry: StoredEntry,
        body: String,
        lines: usize,
    ) -> Self {
        Self {
            file_name,
            body,
            kind: ExportKind::Retrieval {
                r#ref,
                entry,
                lines,
            },
        }
    }

    pub(in crate::service) fn body(&self) -> &str {
        &self.body
    }

    /// The output once the body was written to `written`'s path, or failed
    /// with its error.
    pub(in crate::service) fn output(&self, written: Result<String, String>) -> ToolOutput {
        match (&self.kind, written) {
            (ExportKind::Restore { header }, written) => self.restore_output(header, written),
            (
                ExportKind::Retrieval {
                    r#ref,
                    entry,
                    lines,
                },
                Ok(path),
            ) => text_output(build_retrieval_pointer(r#ref, entry, &path, *lines), false),
            // A store that keeps no files (an ephemeral session's) cannot
            // export: the original comes back inline, which the request's
            // tool-output cap still bounds.
            (ExportKind::Retrieval { r#ref, entry, .. }, Err(error)) => text_output(
                format!(
                    "[acp_retrieve: the original could not be exported ({error}); it follows inline]\n{}",
                    frame_retrieved_original(r#ref, entry, &self.body)
                ),
                false,
            ),
        }
    }

    fn restore_output(&self, header: &str, written: Result<String, String>) -> ToolOutput {
        let length = self.body.encode_utf16().count();
        match written {
            Ok(path) => text_output(
                format!(
                    "{header}\nContent ({length} chars) written to: {path} (this session's export file; toFile only asks for one)\nUse the read tool to access it."
                ),
                false,
            ),
            Err(error) => {
                let preview = utf16_window(&self.body, 0, 4000);
                let shown = preview.encode_utf16().count();
                let scope = if shown < length {
                    format!("partial content follows ({shown} of {length} chars)")
                } else {
                    "full content follows".to_owned()
                };
                text_output(
                    format!(
                        "[decompress FAILED: could not write export {}: {error}]\nThe export did NOT complete — {scope}. Omit toFile to restore inline.\n{preview}",
                        self.file_name
                    ),
                    true,
                )
            }
        }
    }
}
