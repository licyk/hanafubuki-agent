use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const ABSOLUTE_MAX_CUMULATIVE_MODEL_INPUT_TOKENS: u64 = 100_000_000;
pub const ABSOLUTE_MAX_CUMULATIVE_MODEL_OUTPUT_TOKENS: u64 = 10_000_000;
pub const ABSOLUTE_MAX_TOOL_INVOCATIONS: u64 = 10_000;
pub const ABSOLUTE_MAX_TOOL_OUTPUT_BYTES: u64 = 256 * 1024 * 1024;
pub const ABSOLUTE_MAX_HOOK_FRAGMENT_BYTES: u64 = 16 * 1024 * 1024;
pub const ABSOLUTE_MAX_PENDING_INPUT_ITEMS: usize = 1_024;
pub const ABSOLUTE_MAX_IN_FLIGHT_TOOLS: usize = 64;
pub const ABSOLUTE_MAX_CONSECUTIVE_STALLED_STEPS: u32 = 32;
pub const ABSOLUTE_MAX_IDLE_TIMEOUT_SECONDS: u64 = 24 * 60 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTurnBudgetPolicy {
    pub maximum_cumulative_model_input_tokens: u64,
    pub maximum_cumulative_model_output_tokens: u64,
    pub maximum_tool_invocations: u64,
    pub maximum_tool_output_bytes: u64,
    pub maximum_hook_fragment_bytes: u64,
    pub maximum_pending_input_items: usize,
    pub maximum_in_flight_tools: usize,
    pub maximum_consecutive_stalled_steps: u32,
    pub model_stream_idle_timeout_seconds: u64,
    pub tool_idle_timeout_seconds: u64,
    pub approval_timeout_seconds: u64,
}

impl Default for AgentTurnBudgetPolicy {
    fn default() -> Self {
        Self {
            maximum_cumulative_model_input_tokens: 8_000_000,
            maximum_cumulative_model_output_tokens: 1_000_000,
            maximum_tool_invocations: 256,
            maximum_tool_output_bytes: 16 * 1024 * 1024,
            maximum_hook_fragment_bytes: 1024 * 1024,
            maximum_pending_input_items: 64,
            maximum_in_flight_tools: 8,
            maximum_consecutive_stalled_steps: 4,
            model_stream_idle_timeout_seconds: 120,
            tool_idle_timeout_seconds: 10 * 60,
            approval_timeout_seconds: 15 * 60,
        }
    }
}

impl AgentTurnBudgetPolicy {
    pub fn validate(self) -> Result<Self, AgentBudgetError> {
        validate_nonzero_ceiling(
            "maximum cumulative model input tokens",
            self.maximum_cumulative_model_input_tokens,
            ABSOLUTE_MAX_CUMULATIVE_MODEL_INPUT_TOKENS,
        )?;
        validate_nonzero_ceiling(
            "maximum cumulative model output tokens",
            self.maximum_cumulative_model_output_tokens,
            ABSOLUTE_MAX_CUMULATIVE_MODEL_OUTPUT_TOKENS,
        )?;
        validate_nonzero_ceiling(
            "maximum tool invocations",
            self.maximum_tool_invocations,
            ABSOLUTE_MAX_TOOL_INVOCATIONS,
        )?;
        validate_nonzero_ceiling(
            "maximum tool output bytes",
            self.maximum_tool_output_bytes,
            ABSOLUTE_MAX_TOOL_OUTPUT_BYTES,
        )?;
        validate_nonzero_ceiling(
            "maximum hook fragment bytes",
            self.maximum_hook_fragment_bytes,
            ABSOLUTE_MAX_HOOK_FRAGMENT_BYTES,
        )?;
        validate_nonzero_ceiling(
            "maximum pending input items",
            self.maximum_pending_input_items as u64,
            ABSOLUTE_MAX_PENDING_INPUT_ITEMS as u64,
        )?;
        validate_nonzero_ceiling(
            "maximum in-flight tools",
            self.maximum_in_flight_tools as u64,
            ABSOLUTE_MAX_IN_FLIGHT_TOOLS as u64,
        )?;
        validate_nonzero_ceiling(
            "maximum consecutive stalled steps",
            u64::from(self.maximum_consecutive_stalled_steps),
            u64::from(ABSOLUTE_MAX_CONSECUTIVE_STALLED_STEPS),
        )?;
        validate_nonzero_ceiling(
            "model stream idle timeout seconds",
            self.model_stream_idle_timeout_seconds,
            ABSOLUTE_MAX_IDLE_TIMEOUT_SECONDS,
        )?;
        validate_nonzero_ceiling(
            "tool idle timeout seconds",
            self.tool_idle_timeout_seconds,
            ABSOLUTE_MAX_IDLE_TIMEOUT_SECONDS,
        )?;
        validate_nonzero_ceiling(
            "approval timeout seconds",
            self.approval_timeout_seconds,
            ABSOLUTE_MAX_IDLE_TIMEOUT_SECONDS,
        )?;
        Ok(self)
    }

    pub fn model_stream_idle_timeout(self) -> Duration {
        Duration::from_secs(self.model_stream_idle_timeout_seconds)
    }

    pub fn tool_idle_timeout(self) -> Duration {
        Duration::from_secs(self.tool_idle_timeout_seconds)
    }

    pub fn approval_timeout(self) -> Duration {
        Duration::from_secs(self.approval_timeout_seconds)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTurnBudgetLedger {
    pub sampling_admissions: u64,
    pub cumulative_model_input_tokens: u64,
    pub cumulative_model_output_tokens: u64,
    pub tool_invocations: u64,
    pub tool_output_bytes: u64,
    pub hook_fragment_bytes: u64,
    pub pending_input_items: usize,
    pub compactions: u64,
}

impl AgentTurnBudgetLedger {
    pub fn record_model_usage(
        &mut self,
        input_tokens: u64,
        output_tokens: u64,
        policy: AgentTurnBudgetPolicy,
    ) -> Result<(), AgentBudgetError> {
        let sampling_admissions = checked_add("sampling admissions", self.sampling_admissions, 1)?;
        let cumulative_model_input_tokens = checked_add(
            "cumulative model input tokens",
            self.cumulative_model_input_tokens,
            input_tokens,
        )?;
        let cumulative_model_output_tokens = checked_add(
            "cumulative model output tokens",
            self.cumulative_model_output_tokens,
            output_tokens,
        )?;
        enforce(
            "cumulative model input tokens",
            cumulative_model_input_tokens,
            policy.maximum_cumulative_model_input_tokens,
        )?;
        enforce(
            "cumulative model output tokens",
            cumulative_model_output_tokens,
            policy.maximum_cumulative_model_output_tokens,
        )?;
        self.sampling_admissions = sampling_admissions;
        self.cumulative_model_input_tokens = cumulative_model_input_tokens;
        self.cumulative_model_output_tokens = cumulative_model_output_tokens;
        Ok(())
    }

    pub fn record_tool_invocations(
        &mut self,
        count: usize,
        policy: AgentTurnBudgetPolicy,
    ) -> Result<(), AgentBudgetError> {
        let count = u64::try_from(count).map_err(|_| AgentBudgetError::Overflow {
            resource: "tool invocations",
        })?;
        let next = checked_add("tool invocations", self.tool_invocations, count)?;
        enforce("tool invocations", next, policy.maximum_tool_invocations)?;
        self.tool_invocations = next;
        Ok(())
    }

    pub fn record_tool_output_bytes(
        &mut self,
        bytes: usize,
        policy: AgentTurnBudgetPolicy,
    ) -> Result<(), AgentBudgetError> {
        let bytes = u64::try_from(bytes).map_err(|_| AgentBudgetError::Overflow {
            resource: "tool output bytes",
        })?;
        let next = checked_add("tool output bytes", self.tool_output_bytes, bytes)?;
        enforce("tool output bytes", next, policy.maximum_tool_output_bytes)?;
        self.tool_output_bytes = next;
        Ok(())
    }

    pub fn record_hook_fragment_bytes(
        &mut self,
        bytes: usize,
        policy: AgentTurnBudgetPolicy,
    ) -> Result<(), AgentBudgetError> {
        let bytes = u64::try_from(bytes).map_err(|_| AgentBudgetError::Overflow {
            resource: "hook fragment bytes",
        })?;
        let next = checked_add("hook fragment bytes", self.hook_fragment_bytes, bytes)?;
        enforce(
            "hook fragment bytes",
            next,
            policy.maximum_hook_fragment_bytes,
        )?;
        self.hook_fragment_bytes = next;
        Ok(())
    }

    pub fn admit_pending_input_items(
        &mut self,
        count: usize,
        policy: AgentTurnBudgetPolicy,
    ) -> Result<(), AgentBudgetError> {
        let next =
            self.pending_input_items
                .checked_add(count)
                .ok_or(AgentBudgetError::Overflow {
                    resource: "pending input items",
                })?;
        if next > policy.maximum_pending_input_items {
            return Err(AgentBudgetError::Exhausted {
                resource: "pending input items",
                limit: policy.maximum_pending_input_items as u64,
            });
        }
        self.pending_input_items = next;
        Ok(())
    }

    pub fn consume_pending_input_items(&mut self, count: usize) -> Result<(), AgentBudgetError> {
        self.pending_input_items =
            self.pending_input_items
                .checked_sub(count)
                .ok_or(AgentBudgetError::Underflow {
                    resource: "pending input items",
                })?;
        Ok(())
    }

    pub fn record_compaction(&mut self) -> Result<(), AgentBudgetError> {
        self.compactions = checked_add("compactions", self.compactions, 1)?;
        Ok(())
    }

    pub fn reset_cumulative_model_usage(&mut self) {
        self.cumulative_model_input_tokens = 0;
        self.cumulative_model_output_tokens = 0;
    }

    pub fn proactive_model_usage_limit_reached(
        &self,
        policy: AgentTurnBudgetPolicy,
        threshold_percent: u8,
    ) -> bool {
        let input_limit = percentage_of(
            policy.maximum_cumulative_model_input_tokens,
            threshold_percent,
        );
        let output_limit = percentage_of(
            policy.maximum_cumulative_model_output_tokens,
            threshold_percent,
        );
        self.cumulative_model_input_tokens >= input_limit
            || self.cumulative_model_output_tokens >= output_limit
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentBudgetError {
    InvalidPolicy {
        resource: &'static str,
        value: u64,
        ceiling: u64,
    },
    Exhausted {
        resource: &'static str,
        limit: u64,
    },
    Overflow {
        resource: &'static str,
    },
    Underflow {
        resource: &'static str,
    },
}

impl fmt::Display for AgentBudgetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPolicy {
                resource,
                value,
                ceiling,
            } => write!(
                formatter,
                "{resource} must be between 1 and {ceiling}, received {value}"
            ),
            Self::Exhausted { resource, limit } => {
                write!(
                    formatter,
                    "{resource} exceeded the configured limit {limit}"
                )
            }
            Self::Overflow { resource } => write!(formatter, "{resource} overflowed"),
            Self::Underflow { resource } => write!(formatter, "{resource} underflowed"),
        }
    }
}

impl std::error::Error for AgentBudgetError {}

fn validate_nonzero_ceiling(
    resource: &'static str,
    value: u64,
    ceiling: u64,
) -> Result<(), AgentBudgetError> {
    if value == 0 || value > ceiling {
        return Err(AgentBudgetError::InvalidPolicy {
            resource,
            value,
            ceiling,
        });
    }
    Ok(())
}

fn checked_add(resource: &'static str, current: u64, amount: u64) -> Result<u64, AgentBudgetError> {
    current
        .checked_add(amount)
        .ok_or(AgentBudgetError::Overflow { resource })
}

fn enforce(resource: &'static str, value: u64, limit: u64) -> Result<(), AgentBudgetError> {
    if value > limit {
        return Err(AgentBudgetError::Exhausted { resource, limit });
    }
    Ok(())
}

fn percentage_of(limit: u64, percent: u8) -> u64 {
    u64::try_from((u128::from(limit) * u128::from(percent)) / 100).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests;
