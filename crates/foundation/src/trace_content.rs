//! The bound on content a `trace` record carries (AGENTS.md, "Agent Content
//! Logging"): wire bodies, model output, tool arguments and results, and a
//! plugin guest's output.

/// The most content a single `trace` record carries, cut on a UTF-8 boundary.
pub const MAX_TRACE_CONTENT_BYTES: usize = 64 * 1024;

/// `value` cut to [`MAX_TRACE_CONTENT_BYTES`] on a UTF-8 boundary, for every
/// `trace` record that carries wire, model, tool or guest content.
pub fn bounded_trace_content(value: &str) -> String {
    if value.len() <= MAX_TRACE_CONTENT_BYTES {
        return value.to_owned();
    }
    let mut boundary = MAX_TRACE_CONTENT_BYTES;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value[..boundary].to_owned()
}

#[cfg(test)]
mod tests;
