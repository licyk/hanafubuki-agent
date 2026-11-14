// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! What a tool's text and items show, after Codex's MCP result cell
//! (`history_cell/mcp_result.rs`): a text that is JSON on one line,
//! spaced, collapsed (Codex's `format_json_compact`) and indented expanded;
//! an image or audio clip as its type and size, a link and an embedded
//! resource by their address, never their bytes. `/raw` keeps a text as it
//! came, as Codex's raw lines do.

use hanafubuki_agent::AgentOutputBlock;

use super::{size_label, text, text_lines, Extent, ToolCall, View};
use crate::history_cell::ActivityDisclosure;
use crate::text_formatting::{format_json_compact, format_json_pretty};

/// A text's or an output's rows, and whether a JSON value among them was
/// laid out on one line that expanding indents over several.
#[derive(Default)]
pub(super) struct Rows {
    rows: Vec<String>,
    laid_out: bool,
}

/// [`text::view`] of `rows`; expanding a JSON value shows more than its
/// collapsed rows count, so its control says only that there are details.
pub(super) fn view(call: &ToolCall, failed: bool, rows: Rows, extent: Extent) -> View {
    let mut view = text::view(call, failed, rows.rows, extent);
    if rows.laid_out {
        view.hidden = Some(ActivityDisclosure::Generic);
    }
    view
}

/// The rows of a tool's text in `extent`'s form.
pub(super) fn text_rows(text: &str, extent: Extent) -> Rows {
    let formatted = if extent.raw {
        None
    } else if extent.expanded {
        format_json_pretty(text)
    } else {
        format_json_compact(text)
    };
    // A value with something in a container is indented over several rows.
    let laid_out = formatted
        .as_deref()
        .is_some_and(|json| json.starts_with(['{', '[']) && json.len() > 2);
    Rows {
        rows: text_lines(formatted.as_deref().unwrap_or(text)),
        laid_out,
    }
}

/// The rows of each item, one after another.
pub(super) fn block_rows(blocks: &[AgentOutputBlock], extent: Extent) -> Rows {
    let mut all = Rows::default();
    let mut push = |rows: Rows| {
        all.rows.extend(rows.rows);
        all.laid_out |= rows.laid_out;
    };
    for block in blocks {
        match block {
            AgentOutputBlock::Text { text } => push(text_rows(text, extent)),
            AgentOutputBlock::Image { mime_type, bytes }
            | AgentOutputBlock::Audio { mime_type, bytes } => push(row(format!(
                "{} · {}",
                one_line(mime_type),
                size_label(*bytes)
            ))),
            AgentOutputBlock::Link { uri, .. } => push(row(format!("link: {}", one_line(uri)))),
            AgentOutputBlock::Resource {
                uri,
                mime_type,
                text,
                bytes,
            } => {
                let mut summary = format!("embedded resource: {}", one_line(uri));
                if let Some(bytes) = bytes {
                    for part in mime_type.iter().map(|mime| one_line(mime)) {
                        summary.push_str(&format!(" · {part}"));
                    }
                    summary.push_str(&format!(" · {}", size_label(*bytes)));
                }
                push(row(summary));
                // A resource read for its text (`read_mcp_resource`) shows it.
                if let Some(text) = text {
                    push(text_rows(text, extent));
                }
            }
        }
    }
    all
}

fn row(text: String) -> Rows {
    Rows {
        rows: vec![text],
        laid_out: false,
    }
}

/// What a server put in a summary's field, as one line without controls.
fn one_line(text: &str) -> String {
    text_lines(&text.replace(['\n', '\r'], " ")).concat()
}
