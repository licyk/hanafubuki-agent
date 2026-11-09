// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// the host side of its core tools in `src/decompress-shared.ts`
// (`resolveDecompress`, `refoldHint`, `executeSearchContext`) at commit
// `4be21b08e`. Copyright (c) 2026 ranxianglei. MIT License with an
// additional attribution term (NOTICE).
// Modified for hanafubuki-agent: `decompress` restores inline only (no
// `toFile`, ranges or images yet), `search_context` searches the block
// summaries without plan steering or the folded originals, and `acp_cache`
// reports from the usage the Agent records.

//! The compression tools: `compress` and `decompress` change the state,
//! `search_context`, `acp_status` and `acp_cache` only read it. Each runs
//! on the session's record, under its lock.

use hanafubuki_agent_compression::{
    apply_compression, block_docs, build_cache_report, build_status_report, collect_block_content,
    format_cache_report, mark_block_restored_inline, parse_block_id_arg, parse_compress_args,
    search_blocks, Algorithm, ApplyInput, CacheDetail, CacheReportOptions, CacheSample, DocCache,
    FoldEvent, FormatCacheReportOptions, InlineRestore, SearchOptions, StatusReportOptions,
    StatusScope, StatusSort, StatusView, DEFAULT_DOC_CACHE_CAP,
};
use hanafubuki_agent_tool_contracts::{ToolOutput, ToolOutputContent};
use serde_json::Value;

use super::*;

/// A compression tool's output, and the state it leaves when it changed it.
pub(in crate::service) struct CompressionToolRun {
    pub(in crate::service) output: ToolOutput,
    pub(in crate::service) state: Option<AgentCompressionState>,
}

/// Runs the compression tool `tool` called as `call_id` with `arguments`
/// on `record` as it stands. A panic in the algorithm is the call's error.
pub(in crate::service) fn compression_tool_output(
    record: &SessionRecord,
    tool: crate::AgentCoreTool,
    call_id: &str,
    arguments: &Value,
    now_ms: u64,
) -> CompressionToolRun {
    let log = MessageLog::new(&record.conversation.history);
    let config = hierarchical_config(record);
    let state = record
        .runtime
        .compression
        .as_deref()
        .cloned()
        .unwrap_or_default();
    let run = catch_panic(|| {
        panic_if_asked(&state);
        match tool {
            crate::AgentCoreTool::Compress => {
                compress(&log, &config, &state, call_id, arguments, now_ms)
            }
            crate::AgentCoreTool::Decompress => decompress(&log, &state, arguments),
            crate::AgentCoreTool::SearchContext => read_only(search_context(&state, arguments)),
            crate::AgentCoreTool::AcpStatus => {
                read_only(acp_status(&log, &config, &state, arguments))
            }
            crate::AgentCoreTool::AcpCache => {
                read_only(acp_cache(record, &state, arguments, now_ms))
            }
            _ => unreachable!("only the compression tools reach here"),
        }
    });
    run.unwrap_or_else(|message| CompressionToolRun {
        output: text_output(
            receipts::failed(&format!("{message} (internal error)")),
            true,
        ),
        state: None,
    })
}

fn compress(
    log: &MessageLog<'_>,
    config: &hanafubuki_agent_compression::Config,
    state: &AgentCompressionState,
    call_id: &str,
    arguments: &Value,
    now_ms: u64,
) -> CompressionToolRun {
    let parsed = parse_compress_args(arguments, Some(call_id));
    if parsed.ranges.is_empty() {
        return CompressionToolRun {
            output: text_output(receipts::unparsed(&parsed), true),
            state: None,
        };
    }
    let outcome = apply_compression(&ApplyInput {
        ranges: &parsed.ranges,
        messages: &log.messages,
        state,
        config,
        protected_refs: None,
        now_ms,
    });
    let result = &outcome.result;
    let changed = outcome
        .state
        .blocks
        .iter()
        .filter(|block| {
            state
                .block(&block.block_id)
                .is_none_or(|before| before.summary != block.summary)
        })
        .collect::<Vec<_>>();
    let receipt = receipts::applied(&parsed.ranges, &parsed.diagnostics, result, &changed);
    let folded = result.blocks_created > 0;
    CompressionToolRun {
        output: text_output(receipt, !folded && !result.errors.is_empty()),
        state: folded.then_some(outcome.state),
    }
}

fn decompress(
    log: &MessageLog<'_>,
    state: &AgentCompressionState,
    arguments: &Value,
) -> CompressionToolRun {
    let fail = |text: String| CompressionToolRun {
        output: text_output(text, true),
        state: None,
    };
    let Some(raw) = arguments
        .get("blockId")
        .and_then(Value::as_str)
        .filter(|raw| !raw.trim().is_empty())
    else {
        return fail("[decompress FAILED: blockId is required]".to_owned());
    };
    if arguments
        .get("toFile")
        .and_then(Value::as_str)
        .is_some_and(|path| !path.trim().is_empty())
    {
        return fail(
            "[decompress FAILED: toFile is not available here yet — omit it to restore the block inline]"
                .to_owned(),
        );
    }
    let raw = raw.trim();
    let Some(block) = parse_block_id_arg(raw).and_then(|id| state.block(&id)) else {
        return fail(format!("[Block {raw} not found]"));
    };
    let block_id = block.block_id.clone();
    let full = matches!(arguments.get("full"), Some(Value::Bool(true)))
        || arguments.get("full").and_then(Value::as_str) == Some("true");
    let collected = collect_block_content(state, block, &log.messages, full);
    let body = if collected.text.is_empty() {
        block.summary.clone()
    } else {
        collected.text
    };
    let header = format!(
        "[Block {block_id} content — {} item(s){}]",
        collected.count,
        if full { ", full" } else { "" }
    );
    if !block.active {
        return CompressionToolRun {
            output: text_output(format!("{header}\n{body}"), false),
            state: None,
        };
    }
    // The content is now in the conversation: a later compress over the same
    // span refolds the block in place.
    let first_restore = !block.restored_inline;
    let mut restored = state.clone();
    let marked = mark_block_restored_inline(&mut restored, &block_id);
    CompressionToolRun {
        output: text_output(
            format!("{header}\n{body}\n\n{}", refold_hint(&block_id, marked)),
            false,
        ),
        state: first_restore.then_some(restored),
    }
}

fn refold_hint(block_id: &str, restore: Option<InlineRestore>) -> String {
    match restore.and_then(|restore| restore.restored_start_ref.zip(restore.restored_end_ref)) {
        Some((start, end)) => format!(
            "Re-fold: call compress(\"{start}–{end}\", <fresh summary>) → updates block {block_id} in place (same id, new summary)."
        ),
        None => format!(
            "Re-fold: call compress over the restored messages with a fresh summary → updates block {block_id} in place (same id, new summary)."
        ),
    }
}

/// Every block, active or folded away, ranked by the literal occurrences
/// of the query's terms (billion-context's host: substring, any occurrence
/// counts), so a query that matches nothing finds nothing.
fn search_context(state: &AgentCompressionState, arguments: &Value) -> String {
    let query = arguments
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if query.is_empty() {
        return "[search_context FAILED: query is required]".to_owned();
    }
    let limit = positive_number(arguments.get("limit")).unwrap_or(5);
    let pool = search_blocks(
        &block_docs(state),
        query,
        &SearchOptions {
            algorithm: Algorithm::Substring,
            limit: state.blocks.len(),
            min_score: 1e-9,
            ..SearchOptions::default()
        },
        &mut DocCache::new(DEFAULT_DOC_CACHE_CAP),
    );
    if pool.is_empty() {
        if state.blocks.is_empty() {
            return "[No compressed blocks exist yet — nothing to search.]".to_owned();
        }
        return format!(
            "[No blocks matched \"{query}\" among {} block(s). Scope = compressed topics/summaries (active + folded-away) — NOT visible messages or other sessions; absence here ≠ never-discussed.]",
            state.blocks.len()
        );
    }
    let lines = pool
        .iter()
        .take(limit)
        .filter_map(|result| {
            let block = state.block(result.block_id.as_deref()?)?;
            let topic = block.topic.as_deref().unwrap_or("(no topic)");
            let inactive = if block.active {
                String::new()
            } else {
                format!(
                    " [inactive — folded away; decompress {} restores it]",
                    block.block_id
                )
            };
            Some(format!(
                "{} (T{}) \"{topic}\"{inactive}\n  {}",
                block.block_id, block.tier, result.preview
            ))
        })
        .collect::<Vec<_>>();
    let truncated = if pool.len() > lines.len() {
        format!(
            "\n\n[truncated: showing {} of {} candidates by relevance — pass limit:{} to see the rest]",
            lines.len(),
            pool.len(),
            pool.len()
        )
    } else {
        String::new()
    };
    format!(
        "Found {} block(s) for \"{query}\":\n\n{}{truncated}",
        lines.len(),
        lines.join("\n\n")
    )
}

fn acp_status(
    log: &MessageLog<'_>,
    config: &hanafubuki_agent_compression::Config,
    state: &AgentCompressionState,
    arguments: &Value,
) -> String {
    let text = |key: &str| arguments.get(key).and_then(Value::as_str);
    let scope = match text("scope") {
        Some("uncompressed") => Some(StatusScope::Uncompressed),
        Some("compressed") => Some(StatusScope::Compressed),
        _ => None,
    };
    let view = match text("view") {
        Some("messages") => StatusView::Messages,
        _ => StatusView::Ranges,
    };
    let sort = match text("sort") {
        Some("time") => StatusSort::Time,
        Some("tool") => StatusSort::Tool,
        Some("age") => StatusSort::Age,
        _ => StatusSort::Size,
    };
    let defaults = StatusReportOptions::default();
    build_status_report(
        state,
        &log.messages,
        &StatusReportOptions {
            scope,
            view,
            tool: text("tool"),
            sort,
            limit: positive_number(arguments.get("limit")).unwrap_or(defaults.limit),
            screen: Some((config, &log.messages)),
        },
    )
}

/// The cache report from what the Agent records: the last response's input
/// and output (the model API reports no cached tokens, so none count as
/// cached) and the folds of the session's blocks.
fn acp_cache(
    record: &SessionRecord,
    state: &AgentCompressionState,
    arguments: &Value,
    now_ms: u64,
) -> String {
    let samples = record
        .runtime
        .usage
        .filter(|usage| usage.source == AgentUsageSource::ProviderReported)
        .map(|usage| CacheSample {
            at: now_ms,
            input: usage.last_response_input_tokens,
            cached: 0,
            output: Some(usage.last_response_output_tokens),
        })
        .into_iter()
        .collect::<Vec<_>>();
    let folds = state
        .blocks
        .iter()
        .map(|block| FoldEvent {
            at: block.created_at,
            tokens_compressed: block.compressed_tokens,
            ..FoldEvent::default()
        })
        .collect::<Vec<_>>();
    let report = build_cache_report(&samples, &folds, &CacheReportOptions::default(), now_ms);
    let detail = match arguments.get("detail").and_then(Value::as_str) {
        Some("full") => CacheDetail::Full,
        _ => CacheDetail::Summary,
    };
    format_cache_report(
        &report,
        None,
        &FormatCacheReportOptions {
            detail,
            utc_offset_seconds: 0,
        },
    )
}

fn read_only(text: String) -> CompressionToolRun {
    CompressionToolRun {
        output: text_output(text, false),
        state: None,
    }
}

fn text_output(text: String, is_error: bool) -> ToolOutput {
    ToolOutput {
        content: vec![ToolOutputContent::Text { text }],
        structured_content: None,
        is_error,
    }
}

/// A positive whole number the model wrote as a number or a numeric string.
fn positive_number(value: Option<&Value>) -> Option<usize> {
    let number = match value? {
        Value::Number(number) => number.as_f64()?,
        Value::String(text) => text.trim().parse::<f64>().ok()?,
        _ => return None,
    };
    (number.is_finite() && number >= 1.0).then(|| number.floor() as usize)
}
