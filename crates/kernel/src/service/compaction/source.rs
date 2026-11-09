use sha2::{Digest, Sha256};

use super::*;

/// What a compaction reads from the session under its lock: the history it
/// replaces, the windows it moves between, and the request that compacts it.
pub(super) struct CompactionSource {
    items: Vec<ConversationItem>,
    item_ids: Vec<AgentHistoryItemId>,
    current_turn_suffix: Vec<AgentHistoryItem>,
    world_state_checkpoint: Option<ConversationItem>,
    pub(super) revision: AgentRevision,
    pub(super) window: AgentContextWindowSnapshot,
    pub(super) target_window: AgentContextWindowSnapshot,
    target_window_chain: Vec<AgentContextWindowSnapshot>,
    canonical_initial_context: ConversationItem,
    request: ModelRequest,
    pub(super) implementation: AgentCompactionImplementation,
    local_trim_prefix_ends: Vec<usize>,
    sampling_retry_policy: AgentSamplingRetryPolicy,
}

impl CompactionSource {
    pub(super) fn into_plan(
        self,
        identity: CompactionIdentity,
        phase: AgentCompactionPhase,
        cancellation: CancellationToken,
        transport: CompactionTransport,
        images: LoadedImages,
    ) -> CompactionPlan {
        CompactionPlan {
            identity,
            phase,
            implementation: self.implementation,
            request: self.request,
            next_tool_plan: transport.tool_plan,
            next_prompt: transport.prompt,
            model: transport.model,
            cancellation,
            source_items: self.items,
            source_item_ids: self.item_ids,
            current_turn_suffix: self.current_turn_suffix,
            world_state_checkpoint: self.world_state_checkpoint,
            source_revision: self.revision,
            source_window: self.window,
            target_window: self.target_window,
            target_window_chain: self.target_window_chain,
            canonical_initial_context: self.canonical_initial_context,
            local_trim_prefix_ends: self.local_trim_prefix_ends,
            sampling_retry_policy: self.sampling_retry_policy,
            images,
        }
    }
}

/// The history before the compaction boundary (a pre-turn or background
/// compaction keeps the current turn out of it), the next context window,
/// and the request that compacts that history with the session's compaction
/// implementation.
pub(super) fn compaction_source(
    record: &SessionRecord,
    phase: AgentCompactionPhase,
    turn_identity: Option<TurnIdentity>,
    images: &LoadedImages,
    transport: &CompactionTransport,
) -> Result<CompactionSource, AgentError> {
    let compaction_boundary = if matches!(
        phase,
        AgentCompactionPhase::PreTurn | AgentCompactionPhase::Background
    ) {
        let identity = turn_identity.ok_or_else(|| {
            AgentError::Protocol("pre-turn compaction is missing its turn identity".to_string())
        })?;
        record
            .conversation.history
            .iter()
            .position(|item| {
                matches!(&item.content, ConversationItem::TurnStart { turn_id } if *turn_id == identity.turn_id)
            })
            .ok_or(AgentError::TurnNotRunning)?
    } else {
        record.conversation.history.len()
    };
    let items = source_items(record, compaction_boundary);
    if items.is_empty() {
        return Err(AgentError::CompactionUnavailable);
    }
    let window = record.runtime.context_window;
    let target_window = window.next()?;
    let mut target_window_chain = match &record.conversation.model_context_boundary {
        AgentModelContextBoundary::Full => vec![window],
        AgentModelContextBoundary::Compacted { checkpoint } => checkpoint.window_chain.clone(),
    };
    target_window_chain.push(target_window);
    let (request, implementation, local_trim_prefix_ends) = compaction_request(
        record,
        &items,
        images,
        transport.model.as_ref(),
        &transport.tool_plan,
    )?;
    Ok(CompactionSource {
        item_ids: record
            .conversation
            .history
            .iter()
            .map(|item| item.item_id)
            .collect(),
        current_turn_suffix: record.conversation.history[compaction_boundary..]
            .iter()
            .map(|item| item.as_ref().clone())
            .collect(),
        world_state_checkpoint: crate::model::full_world_state_checkpoint(
            &record.conversation.history,
        ),
        items,
        revision: AgentRevision::new(record.status.revision),
        window,
        target_window,
        target_window_chain,
        canonical_initial_context: canonical_initial_context(record),
        request,
        implementation,
        local_trim_prefix_ends,
        sampling_retry_policy: record.settings.config.sampling_retry_policy,
    })
}

/// The history before `boundary` as a compaction reads it: projected over
/// the active compression blocks, whose summaries replace what they cover.
fn source_items(record: &SessionRecord, boundary: usize) -> Vec<ConversationItem> {
    let items = &record.conversation.history[..boundary];
    match super::super::compression::project_compaction_source(record, items) {
        Ok(Some(projected)) => projected,
        Ok(None) => history_contents(items),
        Err(message) => {
            log::warn!(
                "Agent context compression failed; the compaction reads the full history: {}",
                bounded_diagnostic(message)
            );
            history_contents(items)
        }
    }
}

/// The model request that compacts `source` with the session's compaction
/// implementation, and the local-trim prefix ends of its input.
fn compaction_request(
    record: &SessionRecord,
    source: &[ConversationItem],
    images: &LoadedImages,
    model: &dyn ModelSession,
    tool_plan: &AgentToolPlan,
) -> Result<(ModelRequest, AgentCompactionImplementation, Vec<usize>), AgentError> {
    let (mut input, local_trim_prefix_ends) = build_compaction_input(source, images)?;
    let implementation =
        AgentCompactionImplementation::from(record.settings.config.compaction_capability);
    if model.compaction_capability() != record.settings.config.compaction_capability {
        return Err(AgentError::Protocol(
            "compaction transport capability changed after session creation".to_string(),
        ));
    }
    let local_summary = implementation == AgentCompactionImplementation::LocalSummary;
    if local_summary {
        if let Some(task_plan) = record.runtime.task_plan.as_ref() {
            input.push(task_plan_model_input(task_plan));
        }
        input.push(ModelInputItem::user_text(SUMMARIZATION_PROMPT));
    } else if implementation == AgentCompactionImplementation::ResponsesCompactV2 {
        input.push(ModelInputItem::CompactionTrigger);
    }
    let request = ModelRequest {
        model: record.settings.config.model.clone(),
        instructions: record.settings.config.instructions.clone(),
        input,
        tools: if local_summary {
            Vec::new()
        } else {
            tool_plan.model_specs()
        },
        tool_choice: hanafubuki_agent_model_api::ModelToolChoice::None,
        parallel_tool_calls: !local_summary && tool_plan.parallel_tool_calls(),
        reasoning: record.settings.config.effective_reasoning(),
        output_format: None,
    };
    Ok((request, implementation, local_trim_prefix_ends))
}

fn build_compaction_input(
    source: &[ConversationItem],
    images: &LoadedImages,
) -> Result<(Vec<ModelInputItem>, Vec<usize>), AgentError> {
    let mut input = Vec::new();
    let mut trim_prefix_ends = Vec::new();
    for item in source {
        if matches!(item, ConversationItem::TurnStart { .. }) && !input.is_empty() {
            trim_prefix_ends.push(input.len());
        }
        if let Some(model) = item.as_model_input_resolved(|image| images.bytes(image))? {
            input.push(model);
        }
    }
    Ok((input, trim_prefix_ends))
}

fn canonical_initial_context(record: &SessionRecord) -> ConversationItem {
    let mut content = String::new();
    content.push_str("Mode: ");
    content.push_str(match record.settings.config.mode {
        crate::AgentSessionMode::Default => "default",
        crate::AgentSessionMode::Plan => "plan",
    });
    content.push('\n');
    if let Some(task_plan) = record.runtime.task_plan.as_ref() {
        content.push_str("\nCurrent task checklist:\n");
        if let Ok(encoded) = serde_json::to_string(task_plan) {
            content.push_str(&encoded);
        }
    }
    let mut hasher = Sha256::new();
    hasher.update(record.settings.config.instructions.as_bytes());
    hasher.update(content.as_bytes());
    ConversationItem::CanonicalInitialContext {
        digest: hasher.finalize().into(),
        content,
    }
}

pub(super) fn canonical_context_digest(item: &ConversationItem) -> [u8; 32] {
    match item {
        ConversationItem::CanonicalInitialContext { digest, .. } => *digest,
        // Every caller passes the plan's item, built by
        // `canonical_initial_context`, which returns only this variant.
        _ => unreachable!("canonical context helper returned the wrong item"),
    }
}
