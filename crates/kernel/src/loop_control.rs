use std::collections::BTreeSet;

use hanafubuki_agent_model_api::ModelToolCall;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{AgentMessageId, AgentMessagePhase};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentFollowUpReason {
    ToolCalls,
    ProviderContinuation,
    PendingUserInput,
    HookContinuation,
    ContextRollover,
    RetryRecovery,
    /// Wake events were delivered at the step.
    BackgroundEvents,
}

/// Codex-compatible result of driving one model response stream.
///
/// Tool execution details remain private to the sampling driver. The outer
/// turn loop only needs to know whether another sample is required and which
/// durable assistant message is the latest terminal candidate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SamplingRequestResult {
    pub needs_follow_up: bool,
    pub last_agent_message_id: Option<AgentMessageId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentProgressDigest(pub [u8; 32]);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentNoProgressState {
    pub last_digest: Option<AgentProgressDigest>,
    pub consecutive_stalled_steps: u32,
}

impl AgentNoProgressState {
    pub fn observe_follow_up(
        &mut self,
        digest: AgentProgressDigest,
        maximum_consecutive_stalled_steps: u32,
    ) -> Result<(), AgentNoProgressError> {
        if self.last_digest == Some(digest) {
            self.consecutive_stalled_steps = self
                .consecutive_stalled_steps
                .checked_add(1)
                .ok_or(AgentNoProgressError::CounterOverflow)?;
        } else {
            self.last_digest = Some(digest);
            self.consecutive_stalled_steps = 0;
        }
        if self.consecutive_stalled_steps >= maximum_consecutive_stalled_steps {
            return Err(AgentNoProgressError::Exhausted {
                consecutive_steps: self.consecutive_stalled_steps,
                limit: maximum_consecutive_stalled_steps,
            });
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentNoProgressError {
    CounterOverflow,
    Exhausted { consecutive_steps: u32, limit: u32 },
}

pub fn sampling_progress_digest(
    assistant_messages: &[(Option<AgentMessagePhase>, String)],
    tool_calls: &[ModelToolCall],
    reasons: &BTreeSet<AgentFollowUpReason>,
) -> AgentProgressDigest {
    let mut hasher = Sha256::new();
    for (phase, message) in assistant_messages {
        hasher.update([match phase {
            Some(AgentMessagePhase::Commentary) => 1,
            Some(AgentMessagePhase::FinalAnswer) => 2,
            None => 0,
        }]);
        hasher.update(message.as_bytes());
    }
    for call in tool_calls {
        hasher.update(call.name.as_bytes());
        hasher.update(call.arguments.to_string().as_bytes());
    }
    for reason in reasons {
        hasher.update([*reason as u8]);
    }
    AgentProgressDigest(hasher.finalize().into())
}

#[cfg(test)]
mod tests;
