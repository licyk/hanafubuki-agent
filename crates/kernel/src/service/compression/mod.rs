//! The `hierarchical` strategy: billion-context's algorithm
//! (`hanafubuki-agent-compression`) over the session's history. The history
//! is never rewritten: each request is built from a projection of it, in
//! which the blocks the model folded replace the messages they cover. The
//! state (refs, blocks, the nudge's baselines) is persisted with the runtime
//! checkpoint, the content store's originals in a file of their own
//! (`runtime`). Every call into the algorithm is caught: a panic there costs
//! the projection (the request carries the full history and the turn goes on
//! as `standard`) or the tool call, never the session.

mod adapter;
mod cache;
mod inherit;
mod receipts;
mod runtime;
mod status;
mod tools;

use std::sync::OnceLock;

use hanafubuki_agent_compression::{
    build_absorb_system_prompt, format_rules_for_prompt, hide_consumed_compress_calls,
    process_turn_with, prune, render_nudge_text, DocCache, LocalTime, Prompts, RenderStrategy,
    TokenCountSource, TurnExtras, TurnInput, RULES_USAGE_PROMPT,
};
use hanafubuki_agent_foundation::panic::catch_panic;

use super::*;
use crate::{AgentCompressionState, AgentContentStore, AgentHierarchicalCompression};
use adapter::MessageLog;
use cache::ProjectionKey;
pub(super) use inherit::inherited_state;
pub(super) use runtime::CompressionRuntime;
pub(super) use tools::{compression_tool_output, Fold};

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

/// What a committed compression tool leaves: the state and the content
/// store, each when it changed them, and the fold of a `compress` call,
/// recorded as `ContextCompressed` in the same commit.
#[derive(Default)]
pub(super) struct CompressionChange {
    pub(super) state: Option<AgentCompressionState>,
    pub(super) content_store: Option<AgentContentStore>,
    pub(super) fold: Option<(String, Fold)>,
}

/// One request's view of the history.
#[derive(Clone)]
pub(super) struct RequestProjection {
    /// The history the request is built from: covered items out, summaries
    /// in, ref tags and compacted `compress` arguments written in.
    pub(super) history: Arc<Vec<ConversationItem>>,
    /// The state after this request, which the step persists.
    pub(super) state: Arc<AgentCompressionState>,
    /// The nudge or the status line, the request's last input; never
    /// persisted.
    pub(super) trailing: Option<String>,
    /// Neither compression nor truncation can bring the context under the
    /// limit any more: a standard compaction is due (decision 10).
    pub(super) terminal_escape: bool,
    /// The content store, when this request stored originals in it.
    pub(super) content_store: Option<Arc<AgentContentStore>>,
    /// What `hierarchical` adds to the instructions: the static prompt and
    /// the sections of the opt-in features that are on. None while the
    /// blocks are only carried.
    pub(super) instructions: Option<String>,
    /// The token count the request was projected with.
    pub(super) token_count: u64,
    /// A user interrupt was booked into the nudge.
    pub(super) abort_booked: bool,
}

/// The projection of `items` (the window's history the request shows) for
/// `mode`, or the panic message of the algorithm. A step's request takes the
/// projection its admission estimate made from the same inputs.
pub(super) fn project_request(
    record: &SessionRecord,
    items: &[Arc<AgentHistoryItem>],
    mode: CompressionMode,
) -> Result<RequestProjection, String> {
    let settings = hierarchical_settings(record);
    let mut config = settings.config(window(record));
    let clock = message_clock(record, &settings);
    if mode != CompressionMode::Hierarchical {
        // Carried blocks keep projecting, but no feature acts without its
        // tool: absorbed pairs stay hidden, stored originals show again.
        config.absorb.enabled = false;
        config.crush.enabled = false;
        config.content_store.enabled = false;
        config.rules.enabled = false;
    }
    let runtime = &record.runtime.compression_runtime;
    let key = ProjectionKey {
        items: items.to_vec(),
        state: record.runtime.compression.clone(),
        // Only while the feature is on: an empty store stores nothing.
        content_store: config
            .content_store
            .enabled
            .then(|| runtime.shared_content_store().cloned())
            .flatten(),
        mode,
        config,
        clock_offset: clock.map(|clock| clock.utc_offset_seconds),
        token_count: token_count(record, items),
        abort_anchor: runtime
            .abort_anchor
            .filter(|_| mode == CompressionMode::Hierarchical),
        anthropic: record.settings.config.provider_protocol
            == hanafubuki_agent_model_api::ModelApiProtocol::AnthropicMessages,
    };
    if let Some(projection) = runtime.projection.take(&key) {
        return Ok(projection);
    }
    let projection = project(&key, clock)?;
    runtime.projection.keep(key, projection.clone());
    Ok(projection)
}

/// The projection of what `key` holds, the new messages stamped with
/// `clock`.
fn project(key: &ProjectionKey, clock: Option<LocalTime>) -> Result<RequestProjection, String> {
    let log = MessageLog::new(&key.items);
    let config = &key.config;
    let mut state = key.state.as_deref().cloned().unwrap_or_default();
    let (token_count, token_count_source) = key.token_count;
    let content_store = key.content_store.as_deref().cloned().unwrap_or_default();
    let entries = content_store.by_ref.len();
    let (output, content_store, abort_booked) = catch_panic(|| {
        panic_if_asked(&state);
        let abort_booked = key
            .abort_anchor
            .is_some_and(|anchor| state.nudge.absorb_aborted_turn_growth(anchor, token_count));
        let (output, content_store) = process_turn_with(
            TurnInput {
                messages: &log.messages,
                state,
                config,
                token_count,
                token_count_source: Some(token_count_source),
                render: RenderStrategy::TextAndToolResults,
                clock,
            },
            TurnExtras {
                content_store,
                filters: &[],
            },
        );
        (output, content_store, abort_booked)
    })?;
    let (trailing, instructions) = if key.mode == CompressionMode::Hierarchical {
        catch_panic(|| {
            let trailing = if output.nudge.should_inject {
                Some(render_nudge_text(&output.nudge, &Prompts::default(), key.anthropic).text)
            } else {
                output.status_line.clone()
            };
            (
                trailing,
                Some(instructions(config, &output.state, clock.is_some())),
            )
        })?
    } else {
        (None, None)
    };
    Ok(RequestProjection {
        history: Arc::new(log.apply_view(&output.messages)),
        state: Arc::new(output.state),
        trailing,
        terminal_escape: output.terminal_escape.is_some(),
        content_store: (content_store.by_ref.len() > entries).then(|| Arc::new(content_store)),
        instructions,
        token_count,
        abort_booked,
    })
}

/// The compression tools a request of `record` plans: none unless the
/// strategy is `hierarchical`, then the core ones and those of the opt-in
/// features that are on.
pub(super) fn planned_tools(record: &SessionRecord) -> Vec<crate::AgentCoreTool> {
    if CompressionMode::of(record) != CompressionMode::Hierarchical {
        return Vec::new();
    }
    let config = hierarchical_config(record);
    let opt_in = [
        (config.absorb.enabled, crate::AgentCoreTool::Absorb),
        (
            config.content_store.enabled,
            crate::AgentCoreTool::AcpRetrieve,
        ),
        (config.rules.enabled, crate::AgentCoreTool::AcpRule),
    ];
    crate::AgentCoreTool::COMPRESSION
        .into_iter()
        .chain(
            opt_in
                .into_iter()
                .filter_map(|(enabled, tool)| enabled.then_some(tool)),
        )
        .collect()
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

/// What a `hierarchical` request appends to the instructions: the static
/// prompt (its tag section showing the time attribute with
/// `message_timestamps`), then the absorb section while `absorb` is on,
/// then the rules' usage and the rules recorded while `rules` is on. The
/// same bytes for every request until a rule is recorded or removed or the
/// setting changes.
fn instructions(
    config: &hanafubuki_agent_compression::Config,
    state: &AgentCompressionState,
    message_timestamps: bool,
) -> String {
    static PROMPTS: [OnceLock<String>; 2] = [OnceLock::new(), OnceLock::new()];
    let mut text = PROMPTS[usize::from(message_timestamps)]
        .get_or_init(|| {
            hanafubuki_agent_compression::build_compress_system_prompt(
                &Prompts::default(),
                message_timestamps,
            )
        })
        .clone();
    if config.absorb.enabled {
        text.push_str("\n\n");
        text.push_str(&build_absorb_system_prompt());
    }
    if config.rules.enabled {
        text.push_str("\n\n");
        text.push_str(RULES_USAGE_PROMPT);
        let rules = format_rules_for_prompt(state);
        if !rules.is_empty() {
            text.push_str("\n\n");
            text.push_str(&rules);
        }
    }
    text
}

/// A user interrupt ended a `hierarchical` turn: the nudge books the growth
/// it skipped once the context grows past what the turn was sent with.
pub(super) fn note_interrupt(record: &mut SessionRecord) {
    if CompressionMode::of(record) == CompressionMode::Hierarchical {
        record.runtime.compression_runtime.interrupted();
    }
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

/// The algorithm's configuration: [`hierarchical_settings`] over the
/// session's context window. With no known window nothing presses: no
/// nudge, no truncation.
fn hierarchical_config(record: &SessionRecord) -> hanafubuki_agent_compression::Config {
    hierarchical_settings(record).config(window(record))
}

/// `[compaction.hierarchical]` from the turn's extensions, the defaults
/// without.
fn hierarchical_settings(record: &SessionRecord) -> AgentHierarchicalCompression {
    record
        .runtime
        .turn
        .as_ref()
        .and_then(|turn| turn.extensions.get::<AgentHierarchicalCompression>())
        .cloned()
        .unwrap_or_default()
}

fn window(record: &SessionRecord) -> u64 {
    record.settings.config.context_window_tokens.unwrap_or(0)
}

/// The clock a projection stamps first-seen messages with while
/// `messageTimestamps` is on: now, in the offset the turn was admitted in.
/// A stamp is made once and keeps that offset, so a tag already sent never
/// changes.
fn message_clock(
    record: &SessionRecord,
    settings: &AgentHierarchicalCompression,
) -> Option<LocalTime> {
    if !settings.message_timestamps {
        return None;
    }
    Some(fixed_clock(record).unwrap_or_else(|| LocalTime {
        at: now_millis_u64(),
        utc_offset_seconds: settings.utc_offset_seconds,
    }))
}

/// The clock a test fixed for the session
/// ([`AgentService::test_fix_message_clock`]), so the times it pins are
/// the same on every run.
#[cfg(any(test, feature = "test-support"))]
fn fixed_clock(record: &SessionRecord) -> Option<LocalTime> {
    record.runtime.compression_runtime.fixed_clock
}

#[cfg(not(any(test, feature = "test-support")))]
fn fixed_clock(_: &SessionRecord) -> Option<LocalTime> {
    None
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
    (
        estimate_conversation_items_tokens(items.iter().map(|item| &item.content)).unwrap_or(0),
        TokenCountSource::Estimate,
    )
}
