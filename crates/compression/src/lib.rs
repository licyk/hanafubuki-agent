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
mod ids;
mod model;
mod parse;
mod project;
mod protect;
mod prune;
mod refs;
mod render;
mod text;

pub use apply::{
    apply_compression, evaluate_refold, ApplyInput, ApplyOutcome, ApplyResult, RangeSpec,
    RefoldDecision,
};
pub use boundaries::{
    active_block_spans, format_created_blocks, parse_boundary, resolve_block_span,
    resolve_boundaries, BlockSpan, Boundary, BoundaryError, BoundaryErrorKind, BoundaryKind,
    Endpoint, ResolvedRange,
};
pub use model::{
    Block, CompressLimits, Config, Generation, Kind, Message, NudgeConfig, NudgeState, RefMap,
    Role, State, Stats, TierConfig, TierGrowthTokens, TruncateConfig, TruncateLatchEntry,
    ViewLatch,
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
pub use refs::{
    assign_message_refs, assign_refs, highest_used_index, index_to_ref, ordered_ref_pair,
    ref_to_index, AssignedRefs, BLOCKED_REF,
};
pub use render::{count_message_tokens, default_count_tokens, render_refs, RenderStrategy};
