//! What a request contained, group by group (`/context`): each sampling
//! step's request as it is sent, kept for the session while the Agent runs,
//! and, before one is sent, an estimate of the next.

use hanafubuki_agent_contracts::estimate_tokens;

use super::estimate::{serialized_len, without_media};
use super::*;
use crate::model::{
    AgentContextComposition, AgentContextRequest, AgentContextSection, AgentContextTools,
    AgentReportedPrompt, AgentSentRequest, AgentSessionContext,
};

impl AgentService {
    /// What `session_id`'s `/context` shows: the last request a sampling
    /// step of this run sent, or an estimate of the next (built as a
    /// standalone compaction's check of it is: no step, no contributor
    /// preparing, no tool run and no model called), with the window and
    /// the usage.
    pub async fn session_context(
        &self,
        session_id: AgentSessionId,
    ) -> Result<AgentSessionContext, AgentError> {
        let sent = self
            .session(session_id)?
            .lock()?
            .runtime
            .sent_request
            .clone();
        let request = match sent {
            Some(sent) => AgentContextRequest::Sent((*sent).clone()),
            None => match self.next_request_composition(session_id).await {
                Ok(composition) => AgentContextRequest::Next(composition),
                Err(error) => AgentContextRequest::Unavailable(error.to_string()),
            },
        };
        // The window as `/status` shows it: the session's, else the one its
        // model's facts resolve.
        let context_window_tokens = self
            .session_model_metadata(session_id)?
            .context_window
            .map(|window| window.value);
        let usage = self.session(session_id)?.lock()?.runtime.usage;
        Ok(AgentSessionContext {
            request,
            context_window_tokens,
            usage,
        })
    }

    async fn next_request_composition(
        &self,
        session_id: AgentSessionId,
    ) -> Result<AgentContextComposition, AgentError> {
        let transport = self.prepare_compaction_transport(session_id).await?;
        let images = self.load_history_images(session_id).await?;
        let request = {
            let cell = self.session(session_id)?;
            let record = cell.lock()?;
            step_model_request(
                &record,
                &images,
                &transport.tool_plan,
                &transport.prompt,
                self.inner.limits.maximum_tool_output_bytes,
            )?
        };
        compose_request(&request, &transport.tool_plan)
    }

    /// Keeps `composition` as the session's last sent request; what the
    /// provider reports of it comes with its response.
    pub(super) async fn capture_sent_request(
        &self,
        session_id: AgentSessionId,
        composition: AgentContextComposition,
    ) -> Result<(), AgentError> {
        let cell = self.session(session_id)?;
        let mut record = cell.lock_after_persistence().await?;
        record.runtime.sent_request = Some(Arc::new(AgentSentRequest {
            composition,
            captured_at_ms: now_millis_u64(),
            reported: None,
        }));
        Ok(())
    }
}

/// Records what the provider reported of the session's last sent request.
pub(super) fn record_reported_prompt(
    record: &mut SessionRecord,
    reported: &hanafubuki_agent_model_api::ModelUsage,
) {
    if let Some(sent) = record.runtime.sent_request.as_mut() {
        Arc::make_mut(sent).reported = Some(AgentReportedPrompt {
            input_tokens: reported.input_tokens,
            cache_read_tokens: reported.cache_read_tokens,
            cache_write_tokens: reported.cache_write_tokens,
        });
    }
}

/// `request`'s groups, its tools attributed by `tool_plan` (the plan it was
/// built with, whose specs are its tools in order). Each group is the
/// estimate of its own serialized part; what no group holds is the rest.
pub(in crate::service) fn compose_request(
    request: &ModelRequest,
    tool_plan: &AgentToolPlan,
) -> Result<AgentContextComposition, AgentError> {
    let (stripped, images) = without_media(request)?;
    let mut composition = AgentContextComposition {
        instructions_tokens: estimate_tokens(serialized_len(&stripped.instructions)?),
        images: images.count,
        image_tokens: images.tokens,
        ..AgentContextComposition::default()
    };
    // Each group is summed in bytes, and its estimate rounds once.
    let mut tools: Vec<(Option<&crate::AgentToolProviderId>, u64, usize)> = Vec::new();
    for (provider_id, specification) in tool_plan.model_spec_providers() {
        let bytes = serialized_len(specification)?;
        match tools.iter_mut().find(|(known, ..)| *known == provider_id) {
            Some((_, count, total)) => {
                *count += 1;
                *total = total.saturating_add(bytes);
            }
            None => tools.push((provider_id, 1, bytes)),
        }
    }
    composition.tools = tools
        .into_iter()
        .map(|(provider_id, count, bytes)| AgentContextTools {
            provider_id: provider_id.map(|id| id.as_str().to_string()),
            count,
            tokens: estimate_tokens(bytes),
        })
        .collect();
    let mut input_bytes = InputBytes::default();
    for item in &stripped.input {
        input_bytes.add(item, serialized_len(item)?);
    }
    input_bytes.estimate_into(&mut composition);
    let total_bytes_tokens = estimate_tokens(serialized_len(&stripped)?);
    composition.other_tokens = total_bytes_tokens.saturating_sub(grouped_text_tokens(&composition));
    composition.total_tokens =
        total_bytes_tokens
            .checked_add(images.tokens)
            .ok_or(AgentError::UsageOverflow {
                field: "estimated model request tokens",
            })?;
    Ok(composition)
}

/// The serialized bytes of a request's input, by group.
#[derive(Default)]
struct InputBytes {
    sections: Vec<(String, usize)>,
    user_messages: usize,
    assistant_messages: usize,
    reasoning: usize,
    tool_calls: usize,
    continuation: usize,
    compaction_summaries: usize,
}

impl InputBytes {
    fn add(&mut self, item: &ModelInputItem, bytes: usize) {
        let group = match item {
            // What the person ran counts with the calls and their outputs.
            ModelInputItem::ContextFragment { kind, .. }
                if kind == crate::model::USER_SHELL_COMMAND_FRAGMENT =>
            {
                &mut self.tool_calls
            }
            ModelInputItem::ContextFragment { kind, .. } => {
                match self.sections.iter_mut().find(|(known, _)| known == kind) {
                    Some((_, total)) => *total = total.saturating_add(bytes),
                    None => self.sections.push((kind.clone(), bytes)),
                }
                return;
            }
            ModelInputItem::UserMessage { .. } => &mut self.user_messages,
            ModelInputItem::AssistantMessage { .. } => &mut self.assistant_messages,
            ModelInputItem::Reasoning { .. } => &mut self.reasoning,
            ModelInputItem::ToolCall { .. }
            | ModelInputItem::CustomToolCall { .. }
            | ModelInputItem::ToolOutput { .. }
            | ModelInputItem::CustomToolOutput { .. }
            | ModelInputItem::ToolSearchCall { .. }
            | ModelInputItem::ToolSearchOutput { .. } => &mut self.tool_calls,
            ModelInputItem::ContinuationContext { .. } => &mut self.continuation,
            ModelInputItem::CompactionSummary { .. }
            | ModelInputItem::CompactionTrigger
            | ModelInputItem::Compaction { .. } => &mut self.compaction_summaries,
        };
        *group = group.saturating_add(bytes);
    }

    fn estimate_into(self, composition: &mut AgentContextComposition) {
        composition.sections = self
            .sections
            .into_iter()
            .map(|(kind, bytes)| AgentContextSection {
                kind,
                tokens: estimate_tokens(bytes),
            })
            .collect();
        composition.user_messages_tokens = estimate_tokens(self.user_messages);
        composition.assistant_messages_tokens = estimate_tokens(self.assistant_messages);
        composition.reasoning_tokens = estimate_tokens(self.reasoning);
        composition.tool_calls_tokens = estimate_tokens(self.tool_calls);
        composition.continuation_tokens = estimate_tokens(self.continuation);
        composition.compaction_summaries_tokens = estimate_tokens(self.compaction_summaries);
    }
}

/// The groups' estimates of the request's text, every group but the images
/// and the rest.
fn grouped_text_tokens(composition: &AgentContextComposition) -> u64 {
    [
        composition.instructions_tokens,
        composition.user_messages_tokens,
        composition.assistant_messages_tokens,
        composition.reasoning_tokens,
        composition.tool_calls_tokens,
        composition.continuation_tokens,
        composition.compaction_summaries_tokens,
    ]
    .into_iter()
    .chain(composition.tools.iter().map(|tools| tools.tokens))
    .chain(composition.sections.iter().map(|section| section.tokens))
    .fold(0u64, u64::saturating_add)
}
