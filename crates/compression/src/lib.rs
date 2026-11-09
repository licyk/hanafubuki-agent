// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// the module layout of acp-kernel 0.0.189 (`kernel/src`) at commit
// `4be21b08e`. Copyright (c) 2026 ranxianglei. MIT License with an
// additional attribution term (NOTICE).

//! Model-driven hierarchical context compression, ported from
//! billion-context's acp-kernel.
//!
//! The agent's own model folds ranges of its history into summary blocks by
//! calling a `compress` tool; this crate parses and applies those calls and
//! projects the full history into the view the model sees. It is pure: the
//! state goes into and comes out of every call, and nothing here does I/O.
//!
//! Messages carry the host's opaque ids. The crate mints its own refs
//! (`m00001`, ...) in history order and names blocks `b1`, `b2`, ...; refs
//! and block ids are never reused.

mod apply;
mod boundaries;
mod decompress;
mod ids;
mod latch;
mod media;
mod model;
mod nudge;
mod number;
mod parse;
mod project;
mod prompts;
mod protect;
mod prune;
mod rebuild;
mod recommend;
mod refs;
mod render;
mod report;
mod search;
mod text;
mod truncate;
mod turn;

pub use apply::{
    apply_compression, evaluate_refold, ApplyInput, ApplyOutcome, ApplyResult, RangeSpec,
    RefoldDecision,
};
pub use boundaries::{
    active_block_spans, format_created_blocks, parse_boundary, resolve_block_span,
    resolve_boundaries, BlockSpan, Boundary, BoundaryError, BoundaryErrorKind, BoundaryKind,
    Endpoint, ResolvedRange,
};
pub use decompress::{
    active_ancestor_ids, build_restored_content_preview, collect_block_content, deactivate_blocks,
    expand_block, find_active_ancestor, find_blocks_overlapping_messages,
    mark_block_restored_inline, parse_block_id_arg, CollectedContent, InlineRestore,
    RestoredPreview,
};
pub use latch::{apply_view_latch, view_latch_config_digest};
pub use media::{
    append_media_refs_section, collect_fold_media_refs, extract_media_refs,
    looks_ephemeral_signed_url, parse_media_refs_section, MEDIA_REFS_MARKER,
};
pub use model::{
    Block, CompressLimits, Config, Generation, Kind, Message, NudgeConfig, NudgeState, RefMap,
    Role, State, Stats, TierConfig, TierGrowthTokens, TruncateConfig, TruncateLatchEntry,
    ViewLatch,
};
pub use nudge::{
    decide_nudge, format_ranges, render_nudge_text, ContextBreakdown, NudgeBreakdown,
    NudgeDecision, NudgeInput, NudgeVoice, RenderedNudge, TokenCountSource, ONE_CALL_HINT,
};
pub use parse::{
    derive_topic_from_summary, parse_compress_args, ParseDiagnostics, ParseKind, ParsedCompressArgs,
};
pub use project::{
    adjust_boundaries_for_reasoning_pairs, adjust_boundaries_for_tool_pairs,
    compute_integrity_withdrawals, compute_turn_groups, project_unit, segment_groups,
    viable_ranges, Fatal, FatalKind, IntegrityWithdrawals, ProjectionContext, SegmentItem,
    UnitProjection, UnitRequest, VIABLE_RANGE_MIN_TOKENS,
};
pub use prompts::{
    acp_tools, build_compress_system_prompt, compress_parameters, Prompts, ToolSchema,
    ACP_CACHE_TOOL_DESCRIPTION, ACP_CACHE_TOOL_NAME, ACP_MUTATING_TOOLS, ACP_READONLY_TOOLS,
    ACP_STATUS_TOOL_DESCRIPTION, ACP_STATUS_TOOL_NAME, ACP_TOOL_NAMES, COMPRESS_PHILOSOPHY,
    COMPRESS_TOOL_DESCRIPTION, DECOMPRESS_TOOL_DESCRIPTION, DECOMPRESS_TOOL_NAME,
    HOW_TO_COMPRESS_RULES, LANGUAGE_PRESERVATION_RULE, SEARCH_CONTEXT_TOOL_DESCRIPTION,
    SEARCH_CONTEXT_TOOL_NAME, TIER2_DISTILL_RULES, TIER3_CONDENSE_RULES,
};
pub use protect::{
    collect_aged_compress_call_ids, collect_failed_compress_call_ids, collect_latest_protected,
    compute_protected_refs, is_failed_compress_result, is_message_latest_protected,
    is_message_protected, is_never_preserve_recent, last_user_anchor, match_tool_path,
    match_tool_pattern, LatestProtected, ALWAYS_PROTECTED_TOOLS, COMPRESS_EXCHANGE_KEEP_RECENT,
    COMPRESS_TOOL_NAME, FAILED_COMPRESS_RESULT_PREFIXES, NEVER_PRESERVE_RECENT_TOOLS,
};
pub use prune::{
    hide_consumed_compress_calls, is_rendered_summary, prune, summary_message_id, sync_blocks,
    HiddenCompressCalls, SUMMARY_HEADER,
};
pub use rebuild::{rebuild_state, RebuildResult};
pub use recommend::{
    build_compressible_ranges, merge_ranges_to_threshold, recommend, screen_recommended_units,
    CompressibleRange, ContextRanges, ProtectedRange, Recommendation,
};
pub use refs::{
    assign_message_refs, assign_refs, highest_used_index, index_to_ref, ordered_ref_pair,
    ref_to_index, AssignedRefs, BLOCKED_REF,
};
pub use render::{count_message_tokens, default_count_tokens, render_refs, RenderStrategy};
pub use report::{
    build_cache_report, build_recap, build_status_panel, build_status_report, cache_hit_stats,
    compute_fold_economics, decompose_sample, format_cache_report, format_compact_tokens,
    format_hit_rate, summarize_fold_economics, topic_fallback, CacheDetail, CacheHitSummary,
    CacheReport, CacheReportLine, CacheReportOptions, CacheSample, CacheTotals, CacheUsageSample,
    EconomicsSummary, FoldEconomics, FoldEconomicsInput, FoldEvent, FormatCacheReportOptions,
    PriceProfile, SampleDecomposition, StatusPanelInput, StatusReportOptions, StatusScope,
    StatusSort, StatusView, FOLD_CLAIM_COLD_ROUNDS, FOLD_CLAIM_WARM_HIT_PCT,
};
pub use search::{
    block_docs, char_bigrams, folded_message_inputs, message_docs, search_blocks, stem_word,
    term_frequencies, tokenize, Algorithm, DocCache, DocCacheInfo, DocFeatures, MessageInput,
    RoleWeights, SearchDoc, SearchDocKind, SearchOptions, SearchResult, DEFAULT_DOC_CACHE_CAP,
};
pub use truncate::{truncate_large_tool_outputs, TerminalEscape, TruncateOptions, TruncateResult};
pub use turn::{process_turn, TurnInput, TurnOutput};
