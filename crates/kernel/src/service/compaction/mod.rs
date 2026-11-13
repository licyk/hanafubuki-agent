//! A compaction, in the order it runs: `admission` decides to compact,
//! `prepare` reserves it and snapshots its `source` under the session lock,
//! `execute` asks the model and builds the `replacement` from the snapshot
//! alone, and `commit` installs it; `status` holds the transitions shared by
//! every step and the terminal ones. `background` runs the same compaction
//! on a task of its own and installs it later, rebased.

use super::*;

mod admission;
mod background;
mod commit;
mod execute;
mod prepare;
mod replacement;
mod source;
mod status;

pub(in crate::service) use background::{
    AdoptedCompaction, BackgroundAdoption, BackgroundCompactionSlot,
};

// Verbatim from codex-rs/prompts/templates/compact/prompt.md at bdd3118.
const SUMMARIZATION_PROMPT: &str = include_str!("../../prompts/compact.md");
const SUMMARY_PREFIX: &str = "Another language model started to solve this problem and produced a summary of its thinking process. You also have access to the state of the tools that were used by that language model. Use this to build on the work that has already been done and avoid duplicating work. Here is the summary produced by the other language model, use the information in this summary to assist with your own analysis:";
const COMPACT_USER_MESSAGE_MAX_TOKENS: u64 = 20_000;
const REMOTE_V2_RETAINED_MESSAGE_TOKEN_BUDGET: u64 = 64_000;

struct CompactionExecutionGuard {
    service: AgentService,
    identity: CompactionIdentity,
    armed: bool,
}

/// What a compaction sends with and checks the next request against: the
/// model, and the tool plan and prompt of the step it compacts for (a
/// standalone compaction's own).
pub(in crate::service) struct CompactionTransport {
    model: Arc<dyn ModelSession>,
    pub(in crate::service) tool_plan: AgentToolPlan,
    pub(in crate::service) prompt: RequestContext,
}

/// A reserved compaction and everything it read from the session when it
/// began: executing it reads nothing else from the session record but its
/// cancellation.
struct CompactionPlan {
    /// A background compaction's generation is the install's, not this one.
    identity: CompactionIdentity,
    phase: AgentCompactionPhase,
    implementation: AgentCompactionImplementation,
    request: ModelRequest,
    next_tool_plan: AgentToolPlan,
    next_prompt: RequestContext,
    model: Arc<dyn ModelSession>,
    cancellation: CancellationToken,
    /// What the compaction read: the history before `source_boundary`,
    /// projected over the active compression blocks, so it may have fewer
    /// items than that history.
    source_items: Vec<ConversationItem>,
    /// How many items of the history come before the compaction boundary:
    /// the ones it replaces, whatever `source_items` counts.
    source_boundary: usize,
    /// The whole history's ids when the compaction began, not only those of
    /// `source_items`.
    source_item_ids: Vec<AgentHistoryItemId>,
    current_turn_suffix: Vec<AgentHistoryItem>,
    /// The world state of the history before `source_boundary`, where the
    /// suffix the install keeps begins.
    world_state_checkpoint: Option<ConversationItem>,
    source_revision: AgentRevision,
    source_window: AgentContextWindowSnapshot,
    target_window: AgentContextWindowSnapshot,
    target_window_chain: Vec<AgentContextWindowSnapshot>,
    canonical_initial_context: ConversationItem,
    local_trim_prefix_ends: Vec<usize>,
    sampling_retry_policy: AgentSamplingRetryPolicy,
    /// The images of the history it compacts, loaded before it began: its
    /// request, the remote replacement and the next request use them.
    images: LoadedImages,
}

/// What the model returned, as the replacement of the compacted history.
struct CompactionSample {
    replacement_history: Vec<ConversationItem>,
    summary: Option<String>,
    initial_context_digest: [u8; 32],
    reported_usage: Option<hanafubuki_agent_model_api::ModelUsage>,
    estimated_input_tokens: u64,
    estimated_output_tokens: u64,
}
