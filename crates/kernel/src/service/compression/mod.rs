//! The `hierarchical` strategy: billion-context's algorithm
//! (`hanafubuki-agent-compression`) over the session's history. The history
//! is never rewritten: each request is built from a projection of it, in
//! which the blocks the model folded replace the messages they cover. The
//! state (refs, blocks, the nudge's baselines) is persisted with the runtime
//! checkpoint. Every call into the algorithm is caught: a panic there costs
//! the projection (the request carries the full history and the turn goes on
//! as `standard`) or the tool call, never the session.

mod adapter;
mod inherit;
mod receipts;
mod tools;

use std::sync::OnceLock;

use hanafubuki_agent_compression::{
    hide_consumed_compress_calls, process_turn, prune, render_nudge_text, DocCache, Prompts,
    RenderStrategy, TokenCountSource, TurnInput,
};
use hanafubuki_agent_foundation::panic::catch_panic;

use super::*;
use crate::{AgentCompressionState, AgentHierarchicalCompression};
use adapter::MessageLog;
pub(super) use inherit::inherited_state;
pub(super) use tools::compression_tool_output;

/// `search_context`'s document features, kept per session while it is
/// loaded and never persisted: the summaries and the folded originals
/// change little between searches. Shared by the record's rollback copies.
#[derive(Clone, Default)]
pub(super) struct SearchCache(Arc<parking_lot::Mutex<DocCache>>);

impl SearchCache {
    pub(super) fn lock(&self) -> parking_lot::MutexGuard<'_, DocCache> {
        self.0.lock()
    }
}

/// What a request does with the session's compression.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CompressionMode {
    /// The full history, as under `standard`.
    Off,
    /// The blocks a session left when it switched away keep projecting it,
    /// so the next request does not carry every folded message again; no
    /// nudge, prompt or tool makes new ones.
    Carry,
    /// The strategy is `hierarchical`: the projection, the static prompt,
    /// the nudges and the tools.
    Hierarchical,
}

impl CompressionMode {
    pub(super) fn of(record: &SessionRecord) -> Self {
        if record
            .runtime
            .turn
            .as_ref()
            .is_some_and(|turn| turn.compression_fallback)
        {
            return Self::Off;
        }
        if record.settings.config.compaction_strategy
            == crate::AgentCompactionStrategy::Hierarchical
        {
            return Self::Hierarchical;
        }
        if record
            .runtime
            .compression
            .as_deref()
            .is_some_and(|state| state.active_blocks().next().is_some())
        {
            Self::Carry
        } else {
            Self::Off
        }
    }
}

/// One request's view of the history.
pub(super) struct RequestProjection {
    /// The history the request is built from: covered items out, summaries
    /// in, ref tags and compacted `compress` arguments written in.
    pub(super) history: Vec<ConversationItem>,
    /// The state after this request, which the step persists.
    pub(super) state: AgentCompressionState,
    /// The nudge or the status line, the request's last input; never
    /// persisted.
    pub(super) trailing: Option<String>,
    /// Neither compression nor truncation can bring the context under the
    /// limit any more: a standard compaction is due (decision 10).
    pub(super) terminal_escape: bool,
}

/// The projection of `items` (the window's history the request shows) for
/// `mode`, or the panic message of the algorithm.
pub(super) fn project_request(
    record: &SessionRecord,
    items: &[Arc<AgentHistoryItem>],
    mode: CompressionMode,
) -> Result<RequestProjection, String> {
    let log = MessageLog::new(items);
    let config = hierarchical_config(record);
    let state = record
        .runtime
        .compression
        .as_deref()
        .cloned()
        .unwrap_or_default();
    let (token_count, token_count_source) = token_count(record, items);
    let output = catch_panic(|| {
        panic_if_asked(&state);
        process_turn(TurnInput {
            messages: &log.messages,
            state,
            config: &config,
            token_count,
            token_count_source: Some(token_count_source),
            render: RenderStrategy::TextAndToolResults,
        })
    })?;
    let trailing = (mode == CompressionMode::Hierarchical)
        .then(|| {
            if output.nudge.should_inject {
                let anthropic = record.settings.config.provider_protocol
                    == hanafubuki_agent_model_api::ModelApiProtocol::AnthropicMessages;
                catch_panic(|| {
                    render_nudge_text(&output.nudge, &Prompts::default(), anthropic).text
                })
                .map(Some)
            } else {
                Ok(output.status_line.clone())
            }
        })
        .transpose()?
        .flatten();
    Ok(RequestProjection {
        history: log.apply_view(&output.messages),
        state: output.state,
        trailing,
        terminal_escape: output.terminal_escape.is_some(),
    })
}

/// What a standard compaction compacts while blocks are active: the
/// projection of `items`, the summaries as text, with no tags. `None` when
/// no block is active.
pub(super) fn project_compaction_source(
    record: &SessionRecord,
    items: &[Arc<AgentHistoryItem>],
) -> Result<Option<Vec<ConversationItem>>, String> {
    let Some(state) = record
        .runtime
        .compression
        .as_deref()
        .filter(|state| state.active_blocks().next().is_some())
    else {
        return Ok(None);
    };
    let log = MessageLog::new(items);
    let view = catch_panic(|| {
        let pruned = prune(&log.messages, state);
        hide_consumed_compress_calls(state, &pruned).messages
    })?;
    Ok(Some(log.apply_view(&view)))
}

/// The static prompt appended to a `hierarchical` session's instructions;
/// the same bytes for every request.
pub(super) fn compression_instructions() -> &'static str {
    static PROMPT: OnceLock<String> = OnceLock::new();
    PROMPT.get_or_init(|| {
        hanafubuki_agent_compression::build_compress_system_prompt(&Prompts::default())
    })
}

/// After a standard compaction installs: the items the blocks covered left
/// the window. Under `hierarchical` the blocks deactivate at the next
/// projection and the terminal escape starts over; a session that switched
/// away drops the state, which has nothing left to project.
pub(super) fn after_standard_compaction(record: &mut SessionRecord) {
    let Some(state) = record.runtime.compression.as_deref() else {
        return;
    };
    record.runtime.compression = if record.settings.config.compaction_strategy
        == crate::AgentCompactionStrategy::Hierarchical
    {
        let mut state = state.clone();
        state.terminal_streak = 0;
        Some(Arc::new(state))
    } else {
        None
    };
}

/// A panic in the algorithm for the tests of its containment: under test
/// support, a state whose view-latch digest is
/// [`COMPRESSION_PANIC_MARKER`](crate::test_support::COMPRESSION_PANIC_MARKER)
/// panics in the calls that project and run the tools. The algorithm never
/// writes that digest.
#[cfg(any(test, feature = "test-support"))]
fn panic_if_asked(state: &AgentCompressionState) {
    if state.view_latch_config_digest.as_deref()
        == Some(crate::test_support::COMPRESSION_PANIC_MARKER)
    {
        panic!("a test asked the compression algorithm to panic");
    }
}

#[cfg(not(any(test, feature = "test-support")))]
fn panic_if_asked(_: &AgentCompressionState) {}

/// The algorithm's configuration: `[compaction.hierarchical]` from the
/// turn's extensions (the defaults without), over the session's context
/// window. With no known window nothing presses: no nudge, no truncation.
fn hierarchical_config(record: &SessionRecord) -> hanafubuki_agent_compression::Config {
    let window = record.settings.config.context_window_tokens.unwrap_or(0);
    match record
        .runtime
        .turn
        .as_ref()
        .and_then(|turn| turn.extensions.get::<AgentHierarchicalCompression>())
    {
        Some(settings) => settings.config(window),
        None => AgentHierarchicalCompression::default().config(window),
    }
}

/// The context's size: the provider's count of the last request when it
/// reported one, otherwise an estimate of the window's history.
fn token_count(record: &SessionRecord, items: &[Arc<AgentHistoryItem>]) -> (u64, TokenCountSource) {
    if let Some(tokens) = record
        .runtime
        .usage
        .filter(|usage| usage.source == AgentUsageSource::ProviderReported)
        .and_then(|usage| usage.context_consumed_tokens)
    {
        return (tokens, TokenCountSource::Usage);
    }
    let contents = items
        .iter()
        .map(|item| item.content.clone())
        .collect::<Vec<_>>();
    (
        estimate_conversation_items_tokens(&contents).unwrap_or(0),
        TokenCountSource::Estimate,
    )
}
