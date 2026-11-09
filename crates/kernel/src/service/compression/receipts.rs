// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// the `compress` receipts of `src/stream.ts` (`summaryFingerprintLine`,
// `droppedEntriesNote`, `applyErrorNote` and the success, no-op and failure
// receipts) at commit `4be21b08e`. Copyright (c) 2026 ranxianglei. MIT
// License with an additional attribution term (NOTICE).
// Modified for hanafubuki-agent: the proxy's loop guards, circuit breaker,
// recovery hints and degenerate-fold notes are not ported; the texts the
// model reads are kept verbatim.

//! The texts a `compress` call returns: the success receipt, and the
//! `[Compression FAILED`, `[Compression refused` and `[Compression no-op`
//! receipts of a call that folded nothing.

use hanafubuki_agent_compression::{
    ApplyResult, Block, ParseDiagnostics, ParseKind, ParsedCompressArgs, RangeSpec,
};

const RETURN_TO_TASK_SUFFIX: &str = " This failure only concerns the compress call. Return to the original task now and continue it (visible output or a tool call).";

const FINGERPRINT_NOTE: &str = "Fingerprints show head/tail only — every summary is stored in full. To retrieve a specific detail from a new block, call search_context({ query }) first; decompress only if the excerpt doesn't answer it.";

/// The receipt of a call whose arguments held no range.
pub(super) fn unparsed(parsed: &ParsedCompressArgs) -> String {
    let diagnostics = &parsed.diagnostics;
    let kind = kind_name(diagnostics.kind);
    match diagnostics.kind {
        ParseKind::EmptyInput | ParseKind::MissingContent => format!(
            "[Compression FAILED: the call carried no content at all (kind={kind}) — an empty compress() compresses nothing and can never succeed. Do NOT re-issue an empty call; if you meant to compress, put content as ONE non-empty string in that SAME single call — each block headed by a 'mNNNNN–mNNNNN optional topic' line followed by its summary markdown (a JSON-encoded array of {{startId, endId, summary}} objects is also accepted INSIDE that same string).{RETURN_TO_TASK_SUFFIX}]"
        ),
        _ => {
            let why = if diagnostics.invalid_reasons.is_empty() {
                String::new()
            } else {
                format!(" Reasons: {}.", reasons(diagnostics))
            };
            format!(
                "[Compression FAILED: no valid ranges parsed (kind={kind}, dropped={}).{why} compress takes ONE 'content' STRING holding every range: each block starts with its header line 'mNNNNN–mNNNNN optional topic' followed by that block's summary markdown; a JSON-encoded array of {{startId, endId, summary}} objects is also accepted INSIDE that same string. startId/endId are mNNNNN message refs from the conversation (call acp_status to see current refs).{RETURN_TO_TASK_SUFFIX}]",
                diagnostics.invalid_items
            )
        }
    }
}

/// The receipt of an applied call: what it folded, or why nothing was.
pub(super) fn applied(
    ranges: &[RangeSpec],
    diagnostics: &ParseDiagnostics,
    result: &ApplyResult,
    changed: &[&Block],
) -> String {
    let detail = ranges
        .iter()
        .map(|range| format!("{}–{}", range.start_ref, range.end_ref))
        .collect::<Vec<_>>()
        .join(", ");
    let dropped = dropped_entries_note(diagnostics);
    if result.blocks_created == 0 {
        if result.errors.is_empty() {
            let warnings = if result.warnings.is_empty() {
                "every requested range was already folded or protected.".to_owned()
            } else {
                result.warnings.join(" ")
            };
            return format!("[Compression no-op: nothing was folded and nothing failed — {warnings} Continue your current task.]");
        }
        let errors = result.errors.join("; ");
        let dropped = if dropped.is_empty() {
            String::new()
        } else {
            format!(" {dropped}")
        };
        return format!(
            "[Compression FAILED: {errors}{dropped}{}{RETURN_TO_TASK_SUFFIX}]",
            apply_error_note(result)
        );
    }
    let warnings = if result.warnings.is_empty() {
        String::new()
    } else {
        format!(" {}", result.warnings.join("; "))
    };
    let mut receipt = format!(
        "[Compressed {detail} → {} block(s), ~{} tokens saved.{warnings}{}]",
        result.blocks_created,
        result.tokens_compressed,
        apply_error_note(result)
    );
    if !dropped.is_empty() {
        receipt.push('\n');
        receipt.push_str(&dropped);
    }
    for block in changed {
        receipt.push('\n');
        receipt.push_str(&fingerprint_line(&block.block_id, &block.summary));
    }
    if !changed.is_empty() {
        receipt.push('\n');
        receipt.push_str(FINGERPRINT_NOTE);
    }
    receipt
}

/// The receipt of a call that failed in the algorithm itself.
pub(super) fn failed(message: &str) -> String {
    format!("[Compression FAILED: {message}]")
}

/// A block's integrity line: the summary's length and its two ends, so the
/// model can check it was stored whole without decompressing.
fn fingerprint_line(block_id: &str, summary: &str) -> String {
    let head: String = summary.chars().take(30).collect::<String>();
    let tail_start = summary.chars().count().saturating_sub(100);
    let tail: String = summary.chars().skip(tail_start).collect();
    let flatten = |text: &str| text.replace("\r\n", " ").replace('\n', " ");
    format!(
        " · {block_id} full summary {}ch · head \"{}\" … tail \"{}\"",
        summary.encode_utf16().count(),
        flatten(&head),
        flatten(&tail)
    )
}

fn dropped_entries_note(diagnostics: &ParseDiagnostics) -> String {
    if diagnostics.kind == ParseKind::Truncated && diagnostics.invalid_items == 0 {
        return "[The compress arguments arrived TRUNCATED — only the complete leading entries could be salvaged; any requested range not listed above was LOST, not compressed. Check acp_status for what is still compressible and re-issue the missing range(s).]".to_owned();
    }
    let count = diagnostics.invalid_items;
    if count == 0 {
        return String::new();
    }
    let why = if diagnostics.invalid_reasons.is_empty() {
        format!(
            "{count} entr(ies) failed validation (parse kind={})",
            kind_name(diagnostics.kind)
        )
    } else {
        reasons(diagnostics)
    };
    let (entries, verb, ranges) = if count == 1 {
        ("entry", "was", "range")
    } else {
        ("entries", "were", "ranges")
    };
    format!("[{count} of the submitted {entries} {verb} REJECTED and NOT compressed: {why}. Re-issue the rejected {ranges} in a new compress call.]")
}

fn apply_error_note(result: &ApplyResult) -> String {
    if result.errors.is_empty() {
        return String::new();
    }
    let errors = result
        .errors
        .iter()
        .take(3)
        .map(|error| clipped(error, 200, "…"))
        .collect::<Vec<_>>()
        .join(" | ");
    format!(" Errors: {errors}")
}

fn reasons(diagnostics: &ParseDiagnostics) -> String {
    diagnostics
        .invalid_reasons
        .iter()
        .take(3)
        .map(|reason| clipped(reason, 160, "..."))
        .collect::<Vec<_>>()
        .join(" | ")
}

fn clipped(text: &str, limit: usize, ellipsis: &str) -> String {
    if text.chars().count() > limit {
        format!("{}{ellipsis}", text.chars().take(limit).collect::<String>())
    } else {
        text.to_owned()
    }
}

fn kind_name(kind: ParseKind) -> &'static str {
    match kind {
        ParseKind::Ok => "ok",
        ParseKind::EmptyInput => "empty-input",
        ParseKind::NotObject => "not-object",
        ParseKind::MissingContent => "missing-content",
        ParseKind::ContentNotArray => "content-not-array",
        ParseKind::AlreadyFoldedEcho => "already-folded-echo",
        ParseKind::MalformedJson => "malformed-json",
        ParseKind::Truncated => "truncated",
        ParseKind::NoValidRanges => "no-valid-ranges",
    }
}
