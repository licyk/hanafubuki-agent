// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// the host side of its core tools in `src/decompress-shared.ts` at commit
// `4be21b08e`. Copyright (c) 2026 ranxianglei. MIT License with an
// additional attribution term (NOTICE).
// Modified for hanafubuki-agent: the tools run on the session's record, and
// `acp_cache` reports from the usage the Agent records.

//! The compression tools: `compress`, `decompress` and the opt-in
//! features' `absorb`, `acp_retrieve` and `acp_rule` change the state;
//! `search_context`, `acp_status` and `acp_cache` only read it. Each runs
//! on the session's record, under its lock.

mod decompress;
mod export;
mod opt_in;
mod search;

use hanafubuki_agent_compression::{
    apply_compression, build_cache_report, build_status_report, format_cache_report,
    parse_compress_args, store_covered_originals, ApplyInput, CacheDetail, CacheReportOptions,
    CacheSample, FoldEvent, FormatCacheReportOptions, StatusReportOptions, StatusScope, StatusSort,
    StatusView,
};
use hanafubuki_agent_tool_contracts::{ToolOutput, ToolOutputContent};
use serde_json::Value;

use super::*;
use crate::AgentImageAttachmentRef;
pub(in crate::service) use export::ToolExport;

/// A compression tool's output, the state and content store it leaves when
/// it changed them, the images it brings back and the file it asks to
/// write.
pub(in crate::service) struct CompressionToolRun {
    pub(in crate::service) output: ToolOutput,
    pub(in crate::service) state: Option<AgentCompressionState>,
    /// Images already in the history, attached to the output again.
    pub(in crate::service) images: Vec<AgentImageAttachmentRef>,
    /// Text the output is written to instead: the call's output is
    /// [`ToolExport::output`] once the file is written, with no lock held.
    pub(in crate::service) export: Option<ToolExport>,
    /// The content store with the originals a fold covered added.
    pub(in crate::service) content_store: Option<AgentContentStore>,
    /// What a `compress` call folded.
    pub(in crate::service) fold: Option<Fold>,
}

/// What a `compress` call folded: the blocks it made, the tokens they
/// replaced and the highest tier among the blocks it made or refolded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::service) struct Fold {
    pub(in crate::service) blocks_created: usize,
    pub(in crate::service) tokens_compressed: u64,
    pub(in crate::service) tier: u8,
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
    let images_supported = record.settings.effective_input_capabilities.image_input;
    let run = catch_panic(|| {
        panic_if_asked(&state);
        match tool {
            crate::AgentCoreTool::Compress => compress(
                &log,
                &config,
                &state,
                record.runtime.compression_runtime.content_store(),
                Call {
                    call_id,
                    arguments,
                    now_ms,
                },
            ),
            crate::AgentCoreTool::Decompress => {
                decompress::decompress(&log, &state, arguments, images_supported)
            }
            crate::AgentCoreTool::SearchContext => search::search_context(
                &log,
                &state,
                arguments,
                &mut record.runtime.compression_search.lock(),
            ),
            crate::AgentCoreTool::AcpStatus => {
                read_only(acp_status(&log, &config, &state, arguments))
            }
            crate::AgentCoreTool::AcpCache => {
                read_only(acp_cache(record, &state, arguments, now_ms))
            }
            crate::AgentCoreTool::Absorb => {
                opt_in::absorb(&log, &config, &state, call_id, arguments, now_ms)
            }
            crate::AgentCoreTool::AcpRetrieve => opt_in::retrieve(
                record.runtime.compression_runtime.content_store(),
                &config,
                &state,
                arguments,
            ),
            crate::AgentCoreTool::AcpRule => opt_in::rule(&config, &state, arguments),
            _ => unreachable!("only the compression tools reach here"),
        }
    });
    run.unwrap_or_else(|message| {
        output_run(
            text_output(
                receipts::failed(&format!("{message} (internal error)")),
                true,
            ),
            None,
        )
    })
}

/// A call as the model made it.
struct Call<'a> {
    call_id: &'a str,
    arguments: &'a Value,
    now_ms: u64,
}

fn compress(
    log: &MessageLog<'_>,
    config: &hanafubuki_agent_compression::Config,
    state: &AgentCompressionState,
    content_store: Option<&AgentContentStore>,
    call: Call<'_>,
) -> CompressionToolRun {
    let parsed = parse_compress_args(call.arguments, Some(call.call_id));
    if parsed.ranges.is_empty() {
        return output_run(text_output(receipts::unparsed(&parsed), true), None);
    }
    let outcome = apply_compression(&ApplyInput {
        ranges: &parsed.ranges,
        messages: &log.messages,
        state,
        config,
        protected_refs: None,
        now_ms: call.now_ms,
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
    let content_store = (folded && config.content_store.enabled)
        .then(|| {
            let mut store = content_store.cloned().unwrap_or_default();
            let entries = store.by_ref.len();
            let block_ids = changed
                .iter()
                .map(|block| block.block_id.clone())
                .collect::<Vec<_>>();
            store_covered_originals(
                &mut store,
                &log.messages,
                &outcome.state,
                &block_ids,
                config.content_store.max_head_chars,
            );
            (store.by_ref.len() > entries).then_some(store)
        })
        .flatten();
    let fold = folded.then(|| Fold {
        blocks_created: result.blocks_created,
        tokens_compressed: result.tokens_compressed,
        tier: changed.iter().map(|block| block.tier).max().unwrap_or(1),
    });
    CompressionToolRun {
        content_store,
        fold,
        ..output_run(
            text_output(receipt, !folded && !result.errors.is_empty()),
            folded.then_some(outcome.state),
        )
    }
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

/// `acp_cache`: the cache report, in summary or (`detail: "full"`) in
/// full.
fn acp_cache(
    record: &SessionRecord,
    state: &AgentCompressionState,
    arguments: &Value,
    now_ms: u64,
) -> String {
    let detail = match arguments.get("detail").and_then(Value::as_str) {
        Some("full") => CacheDetail::Full,
        _ => CacheDetail::Summary,
    };
    cache_report(record.runtime.usage, state, detail, now_ms)
}

/// The cache report from what the Agent records: the last response's input
/// and output (the model API reports no cached tokens, so none count as
/// cached) and the folds of the session's blocks.
pub(super) fn cache_report(
    usage: Option<AgentModelUsage>,
    state: &AgentCompressionState,
    detail: CacheDetail,
    now_ms: u64,
) -> String {
    let samples = usage
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
    output_run(text_output(text, false), None)
}

fn output_run(output: ToolOutput, state: Option<AgentCompressionState>) -> CompressionToolRun {
    CompressionToolRun {
        output,
        state,
        images: Vec::new(),
        export: None,
        content_store: None,
        fold: None,
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

/// The part of `text` from UTF-16 unit `start` to `end` (billion-context's
/// offsets), never splitting a character: one a bound falls inside is left
/// out.
fn utf16_window(text: &str, start: usize, end: usize) -> &str {
    let mut units = 0;
    let mut from = None;
    let mut to = text.len();
    for (byte, character) in text.char_indices() {
        if from.is_none() && units >= start {
            from = Some(byte);
        }
        if units + character.len_utf16() > end {
            to = byte;
            break;
        }
        units += character.len_utf16();
    }
    let from = from.unwrap_or(text.len());
    if from >= to {
        return "";
    }
    &text[from..to]
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}
