use super::*;

pub(in crate::service) fn resolve_turn_items(
    turn_id: AgentTurnId,
    history: &[Arc<AgentHistoryItem>],
    input: crate::AgentTurnInput,
) -> Result<Vec<ConversationItem>, AgentError> {
    const MAXIMUM_SELECTED_SKILLS_PER_TURN: usize = 8;
    let crate::AgentTurnInput {
        text,
        images,
        skills,
        turn,
        world_state,
        permission_mode: _,
        origin: _,
        provisional_title: _,
    } = input;
    // The selectable Skills are not re-validated here. `hanafubuki_agent_skills` is their
    // only authority: it decides what a Skill is, what it may contain, and how
    // much of its document one turn carries. A second opinion at this boundary
    // can only ever disagree with the first, and when it does it fails the
    // whole turn over a Skill that loaded correctly.
    validate_world_state_snapshot(&world_state)?;
    let selectable = selectable_skills(&turn);
    let mention_exclusions = turn
        .get::<hanafubuki_agent_contracts::SelectableSkills>()
        .map_or(&[][..], |selectable| {
            selectable.mention_exclusions.as_slice()
        });
    let mut requested = skills;
    let (plain_mentions, linked_skills) = extract_skill_mentions(&text);
    for linked in linked_skills {
        if let Some((id, _)) = selectable.iter().find(|(id, _)| id.as_str() == linked) {
            requested.push(crate::AgentSkillRef { skill: id.clone() });
        }
    }
    for mention in plain_mentions {
        // A `$mention` of a tool or a visible MCP name names that, not a
        // Skill: the kernel's own tools are its knowledge, the others are
        // what the selectable Skills' contributor excluded.
        if crate::AgentCoreTool::ALL
            .iter()
            .any(|tool| tool.name().eq_ignore_ascii_case(mention))
            || mention_exclusions
                .iter()
                .any(|name| name.eq_ignore_ascii_case(mention))
        {
            continue;
        }
        // A `$mention` names a Skill the way a person would — by its name or by
        // its folder — so both resolve, and only an unambiguous match is taken.
        let matches = selectable
            .iter()
            .filter(|(id, skill)| {
                id.folder_name().eq_ignore_ascii_case(mention)
                    || skill.name.eq_ignore_ascii_case(mention)
            })
            .collect::<Vec<_>>();
        if let [(id, _)] = matches.as_slice() {
            requested.push(crate::AgentSkillRef { skill: id.clone() });
        }
    }
    requested.sort_by(|left, right| left.skill.cmp(&right.skill));
    requested.dedup_by(|left, right| left == right);
    if requested.len() > MAXIMUM_SELECTED_SKILLS_PER_TURN {
        return Err(AgentError::LimitExceeded {
            resource: "selected Skills per turn",
            limit: MAXIMUM_SELECTED_SKILLS_PER_TURN,
        });
    }
    let mut items = Vec::with_capacity(requested.len().saturating_add(3));
    items.push(ConversationItem::TurnStart { turn_id });
    let world_state_updates = crate::model::world_state_diff(history, world_state);
    if !world_state_updates.is_empty() {
        items.push(ConversationItem::WorldStateUpdate {
            updates: world_state_updates,
        });
    }
    // A Skill the user referenced explicitly must resolve: without the Skill
    // catalog (its contributor was left out of this admission) the turn is
    // refused rather than run without what was asked for.
    let catalog_missing = turn
        .get::<hanafubuki_agent_contracts::SelectableSkills>()
        .is_none();
    for requested in requested {
        let (id, skill) = selectable
            .iter()
            .find(|(id, _)| *id == requested.skill)
            .ok_or_else(|| {
                if catalog_missing {
                    AgentError::InvalidInput(format!(
                        "the Skill {} cannot be used: the Skill catalog is unavailable",
                        requested.skill
                    ))
                } else {
                    AgentError::StaleGeneration
                }
            })?;
        items.push(ConversationItem::SkillContext {
            skill: id.clone(),
            name: skill.name.clone(),
            content: skill.context.clone(),
        });
    }
    if text.trim().is_empty() && images.is_empty() {
        return Err(AgentError::InvalidInput(
            "user message must contain text or an image".to_string(),
        ));
    }
    items.push(ConversationItem::UserMessage { text, images });
    Ok(items)
}

/// The Skills the turn's admission contributors made selectable, with their
/// parsed ids. An entry whose id is not a Skill id is a contributor's error;
/// it is skipped rather than failing every turn.
fn selectable_skills(
    turn: &hanafubuki_agent_contracts::TurnExtensions,
) -> Vec<(
    hanafubuki_agent_skills::SkillId,
    &hanafubuki_agent_contracts::SelectableSkill,
)> {
    let Some(selectable) = turn.get::<hanafubuki_agent_contracts::SelectableSkills>() else {
        return Vec::new();
    };
    selectable
        .skills
        .iter()
        .filter_map(|skill| {
            match hanafubuki_agent_skills::SkillId::from_untrusted(skill.id.as_str()) {
                Ok(id) => Some((id, skill)),
                Err(error) => {
                    log::warn!("agent selectable Skill skipped: {error}");
                    None
                }
            }
        })
        .collect()
}

pub(in crate::service) fn validate_world_state_snapshot(
    snapshot: &AgentWorldStateSnapshot,
) -> Result<(), AgentError> {
    // The collector bounds the sections: one per kind (the context registry
    // gives a kind one contributor) and at most
    // `MAXIMUM_AGENT_PLUGIN_WORLD_STATE_SECTIONS` of plugin kinds.
    let mut total = 0usize;
    for section in &snapshot.sections {
        if section.kind == ContextSectionKind::TOOL_CATALOG_OMISSIONS {
            return Err(AgentError::InvalidInput(
                "tool catalog omissions are sampling-step state".to_string(),
            ));
        }
        enforce_size(
            "Agent world-state section bytes",
            section.content.len(),
            MAXIMUM_AGENT_WORLD_STATE_SECTION_BYTES,
        )?;
        if section.content.trim().is_empty()
            || AgentWorldStateSection::new(section.kind.clone(), section.content.clone()).version
                != section.version
        {
            return Err(AgentError::InvalidInput(
                "Agent world-state contains an invalid section".to_string(),
            ));
        }
        total = total.saturating_add(section.content.len());
    }
    enforce_size(
        "Agent world-state total bytes",
        total,
        MAXIMUM_AGENT_WORLD_STATE_TOTAL_BYTES,
    )
}

/// The check on a host context read back from persisted steering. Live
/// steering takes its context from the host, which cut it to the bound.
pub(in crate::service) fn validate_host_context(
    context: &crate::AgentHostContext,
) -> Result<(), AgentError> {
    if context.content.trim().is_empty() {
        return Err(AgentError::InvalidInput(
            "host context must not be empty".to_string(),
        ));
    }
    enforce_size(
        "host context bytes",
        context.content.len(),
        crate::MAXIMUM_HOST_CONTEXT_BYTES,
    )
}

fn extract_skill_mentions(text: &str) -> (Vec<&str>, Vec<&str>) {
    let bytes = text.as_bytes();
    let mut plain = Vec::new();
    let mut packages = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'[' {
            if let Some((package, end)) = linked_skill_package(text, bytes, index) {
                packages.push(package);
                index = end;
                continue;
            }
        }
        if bytes[index] != b'$' {
            index += 1;
            continue;
        }
        let start = index + 1;
        let mut end = start;
        while end < bytes.len()
            && (bytes[end].is_ascii_alphabetic()
                || bytes[end].is_ascii_digit()
                || matches!(bytes[end], b'-' | b'_' | b':'))
            && end.saturating_sub(start) < 64
        {
            end += 1;
        }
        if end > start {
            let name = &text[start..end];
            if !is_common_environment_name(name) {
                plain.push(name);
            }
        }
        index = end.max(index + 1);
    }
    (plain, packages)
}

/// `[$name](skill://<id>)` — the form the model is told to write when it names
/// a Skill it read, so the reference survives a rename of the display name.
fn linked_skill_package<'a>(text: &'a str, bytes: &[u8], start: usize) -> Option<(&'a str, usize)> {
    if bytes.get(start + 1) != Some(&b'$') {
        return None;
    }
    let mut name_end = start + 2;
    while bytes
        .get(name_end)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':'))
    {
        name_end += 1;
    }
    if name_end == start + 2 || bytes.get(name_end) != Some(&b']') {
        return None;
    }
    let mut open = name_end + 1;
    while bytes.get(open).is_some_and(u8::is_ascii_whitespace) {
        open += 1;
    }
    if bytes.get(open) != Some(&b'(') {
        return None;
    }
    let close = bytes[open + 1..].iter().position(|byte| *byte == b')')? + open + 1;
    let path = text[open + 1..close].trim();
    let skill = path.strip_prefix("skill://")?;
    if hanafubuki_agent_skills::SkillId::from_untrusted(skill).is_err() {
        return None;
    }
    Some((skill, close + 1))
}

fn is_common_environment_name(name: &str) -> bool {
    matches!(
        name.to_ascii_uppercase().as_str(),
        "PATH"
            | "HOME"
            | "USER"
            | "SHELL"
            | "PWD"
            | "TMPDIR"
            | "TEMP"
            | "TMP"
            | "LANG"
            | "TERM"
            | "XDG_CONFIG_HOME"
    )
}

pub(in crate::service) fn assert_valid_limits(limits: AgentHardLimits) {
    assert!(limits.maximum_sampling_steps > 0);
    assert!(limits.maximum_concurrent_tool_calls > 0);
    assert!(limits.maximum_tool_argument_bytes > 0);
    assert!(
        limits.maximum_tool_output_bytes >= crate::tool_output::MINIMUM_CANONICAL_TOOL_OUTPUT_BYTES
    );
    assert!(limits.maximum_model_output_bytes > 0);
    assert!(limits.maximum_compaction_summary_bytes > 0);
    assert!(limits.maximum_compaction_summary_tokens > 0);
    assert!(limits.maximum_history_items > 0);
    assert!(limits.maximum_history_bytes > 0);
    assert!(limits.event_buffer_capacity > 0);
    assert!(!limits.approval_timeout.is_zero());
    assert!(!limits.turn_deadline.is_zero());
    assert!(!limits.plugin_call_deadline.is_zero());
}

/// Repairs the fields of a session config a rule may have tightened since it
/// was written, or a producer set where it does not apply, so a session is
/// never refused over one: an out-of-range value is clamped, an inapplicable
/// one is dropped, each with a warning. It is the one place the values only
/// the Responses protocol carries (reasoning effort, remote compaction) are
/// narrowed for a session on another protocol: the shared model policy keeps
/// them, the application passes them through, and every session the kernel
/// creates, reconfigures or recovers is narrowed here. Identity (provider id,
/// model) and size bounds are not repaired; [`validate_session_config`]
/// still checks them after this.
pub(in crate::service) fn normalize_session_config(config: &mut AgentSessionConfig) {
    let mut repaired = Vec::new();
    let neutral = config.model.is_empty();
    let protocol = config.provider_protocol;
    if config.reasoning_effort.is_some() && (neutral || !protocol.supports_reasoning_effort()) {
        config.reasoning_effort = None;
        repaired.push("reasoning effort (supported only by the Responses protocol)");
    }
    if config.compaction_capability
        != hanafubuki_agent_model_api::ModelCompactionCapability::LocalSummary
        && (neutral || !protocol.supports_remote_compaction())
    {
        config.compaction_capability =
            hanafubuki_agent_model_api::ModelCompactionCapability::LocalSummary;
        repaired.push(if neutral {
            "compaction capability (an unconfigured session summarizes locally)"
        } else {
            "compaction capability (remote compaction is supported only by the Responses protocol)"
        });
    }
    if config.context_window_tokens == Some(0) {
        config.context_window_tokens = None;
        repaired.push("context window tokens (0 is unknown)");
    }
    let threshold = config.auto_compact_threshold_percent.clamp(1, 100);
    if threshold != config.auto_compact_threshold_percent {
        config.auto_compact_threshold_percent = threshold;
        repaired.push("auto compact threshold percent (clamped to 1..=100)");
    }
    let retry = config.sampling_retry_policy;
    let initial_backoff_millis = retry.initial_backoff_millis.clamp(1, 60_000);
    let normalized_retry = crate::AgentSamplingRetryPolicy {
        maximum_retries: retry.maximum_retries.min(10),
        initial_backoff_millis,
        maximum_backoff_millis: retry
            .maximum_backoff_millis
            .clamp(initial_backoff_millis, 60_000),
    };
    if normalized_retry != retry {
        config.sampling_retry_policy = normalized_retry;
        repaired.push("response stream retry policy (clamped to its bounds)");
    }
    for field in repaired {
        log::warn!("Agent session config repaired: {field}");
    }
}

pub(in crate::service) fn validate_session_config(
    config: &AgentSessionConfig,
    limits: AgentHardLimits,
) -> Result<(), AgentError> {
    if config.provider_id.is_empty()
        || config.provider_id.len() > 64
        || !config
            .provider_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(AgentError::InvalidInput(
            "Provider id must contain 1 to 64 bounded ASCII characters".to_string(),
        ));
    }
    if config.model.len() > 256 || (!config.model.is_empty() && config.model.trim().is_empty()) {
        return Err(AgentError::InvalidInput(
            "model must be empty or contain 1 to 256 bytes".to_string(),
        ));
    }
    // Reasoning effort and compaction were narrowed by
    // `normalize_session_config`, which every caller runs first.
    if config.model.is_empty() && !config.instructions.is_empty() {
        return Err(AgentError::InvalidInput(
            "an unconfigured Agent session must use the neutral model policy".to_string(),
        ));
    }
    enforce_size(
        "instructions bytes",
        config.instructions.len(),
        limits.maximum_history_bytes,
    )?;
    config.sampling_retry_policy.validate()?;
    if config.context_window_tokens == Some(0) {
        return Err(AgentError::InvalidInput(
            "context window tokens must be positive".to_string(),
        ));
    }
    if !(1..=100).contains(&config.auto_compact_threshold_percent) {
        return Err(AgentError::InvalidInput(
            "auto compact threshold percent must be between 1 and 100".to_string(),
        ));
    }
    Ok(())
}
