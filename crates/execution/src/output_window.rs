//! What one `exec_command` or `write_stdin` poll returns of the output it has
//! not read yet.
//!
//! A poll returns everything since the previous one when it fits the model's
//! read bound. When it does not, or when the buffer dropped output the poll
//! had not read, it returns the head and the tail of what is there, cut at
//! line breaks, with the size of each gap between them: nothing is left out
//! without saying so.

use hanafubuki_agent_foundation::terminal_text::{aligned_start, end_point};

/// One piece of a poll's output, in order.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum WindowPart<'a> {
    /// Output bytes from `offset` on.
    Text { bytes: &'a [u8], offset: u64 },
    /// Bytes of output left out here.
    Omitted(u64),
}

/// Split `bytes`, the output from `start` on, that a poll from `from` read
/// (`start` is past `from` when the buffer no longer held the bytes between),
/// into what the poll returns within `bound` bytes.
pub(crate) fn output_window(
    bytes: &[u8],
    from: u64,
    start: u64,
    bound: usize,
) -> Vec<WindowPart<'_>> {
    let dropped = start.saturating_sub(from);
    let text = |range: std::ops::Range<usize>| WindowPart::Text {
        bytes: &bytes[range.clone()],
        offset: start + range.start as u64,
    };
    if bytes.len() <= bound {
        let mut parts = Vec::with_capacity(2);
        if dropped > 0 {
            parts.push(WindowPart::Omitted(dropped));
        }
        if !bytes.is_empty() {
            parts.push(text(0..bytes.len()));
        }
        return parts;
    }
    // Output before the dropped bytes was returned already: the tail alone
    // follows the gap.
    let head_end = if dropped == 0 {
        end_point(bytes, bound / 2)
    } else {
        0
    };
    let tail_start = tail_start(bytes, bytes.len() - (bound - head_end)).max(head_end);
    let mut parts = Vec::with_capacity(3);
    if head_end > 0 {
        parts.push(text(0..head_end));
    }
    parts.push(WindowPart::Omitted(
        dropped + (tail_start - head_end) as u64,
    ));
    if tail_start < bytes.len() {
        parts.push(text(tail_start..bytes.len()));
    }
    parts
}

/// Where the tail that may begin at `cut` starts: at a line, unless the only
/// line break near is the output's last byte.
fn tail_start(bytes: &[u8], cut: usize) -> usize {
    match aligned_start(bytes, cut) {
        start if start < bytes.len() => start,
        _ => aligned_start(&bytes[..bytes.len() - 1], cut).min(bytes.len()),
    }
}

/// The marker a gap of `omitted` bytes leaves in the text.
pub(crate) fn omission_marker(omitted: u64) -> String {
    format!("[… {omitted} bytes omitted …]")
}

#[cfg(test)]
mod tests;
