use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentReasoningPhase {
    #[default]
    NotObserved,
    Thinking,
    Completed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentReasoningSnapshot {
    pub sampling_step: u32,
    pub phase: AgentReasoningPhase,
    pub active_since: Option<u128>,
    pub step_completed_millis: u128,
    pub turn_completed_millis: u128,
    pub last_completed_at: Option<u128>,
}

/// Durable Agent-internal reasoning lifecycle checkpoint.
///
/// Provider item identities and reasoning content deliberately do not cross
/// this boundary. The active count only preserves overlapping item timing.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentReasoningCheckpoint {
    snapshot: AgentReasoningSnapshot,
    active_reasoning_item_count: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AgentReasoningTransitionError {
    ActiveItemsAtStepBoundary,
    InvalidSamplingStep { current: u32, next: u32 },
    ItemNotActive,
    ClockMovedBackwards,
    ArithmeticOverflow,
    InvalidCheckpoint,
}

impl fmt::Display for AgentReasoningTransitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ActiveItemsAtStepBoundary => {
                formatter.write_str("reasoning items remain active at the sampling step boundary")
            }
            Self::InvalidSamplingStep { current, next } => write!(
                formatter,
                "reasoning sampling step must advance exactly once (current={current}, next={next})"
            ),
            Self::ItemNotActive => formatter.write_str("reasoning item is not active"),
            Self::ClockMovedBackwards => formatter.write_str("reasoning clock moved backwards"),
            Self::ArithmeticOverflow => {
                formatter.write_str("reasoning timing arithmetic overflowed")
            }
            Self::InvalidCheckpoint => {
                formatter.write_str("reasoning checkpoint violates lifecycle invariants")
            }
        }
    }
}

impl AgentReasoningCheckpoint {
    pub fn snapshot(self) -> AgentReasoningSnapshot {
        self.snapshot
    }

    pub fn is_active(self) -> bool {
        self.active_reasoning_item_count > 0
    }

    pub(crate) fn begin_step(
        &mut self,
        sampling_step: u32,
    ) -> Result<(), AgentReasoningTransitionError> {
        self.validate()?;
        if self.active_reasoning_item_count != 0 {
            return Err(AgentReasoningTransitionError::ActiveItemsAtStepBoundary);
        }
        let expected = self
            .snapshot
            .sampling_step
            .checked_add(1)
            .ok_or(AgentReasoningTransitionError::ArithmeticOverflow)?;
        if sampling_step != expected {
            return Err(AgentReasoningTransitionError::InvalidSamplingStep {
                current: self.snapshot.sampling_step,
                next: sampling_step,
            });
        }
        self.snapshot = AgentReasoningSnapshot {
            sampling_step,
            turn_completed_millis: self.snapshot.turn_completed_millis,
            ..AgentReasoningSnapshot::default()
        };
        self.validate()
    }

    pub(crate) fn item_started(
        &mut self,
        now: u128,
    ) -> Result<bool, AgentReasoningTransitionError> {
        self.validate()?;
        if self.snapshot.sampling_step == 0 {
            return Err(AgentReasoningTransitionError::InvalidSamplingStep {
                current: 0,
                next: 0,
            });
        }
        let first_active = self.active_reasoning_item_count == 0;
        self.active_reasoning_item_count = self
            .active_reasoning_item_count
            .checked_add(1)
            .ok_or(AgentReasoningTransitionError::ArithmeticOverflow)?;
        if first_active {
            self.snapshot.phase = AgentReasoningPhase::Thinking;
            self.snapshot.active_since = Some(now);
        }
        self.validate()?;
        Ok(first_active)
    }

    pub(crate) fn item_completed(
        &mut self,
        now: u128,
    ) -> Result<bool, AgentReasoningTransitionError> {
        let mut next = *self;
        next.validate()?;
        if next.active_reasoning_item_count == 0 {
            return Err(AgentReasoningTransitionError::ItemNotActive);
        }
        next.active_reasoning_item_count -= 1;
        if next.active_reasoning_item_count != 0 {
            next.validate()?;
            *self = next;
            return Ok(false);
        }
        next.finish_active_interval(now)?;
        next.validate()?;
        *self = next;
        Ok(true)
    }

    pub(crate) fn terminal_close(
        &mut self,
        now: u128,
    ) -> Result<bool, AgentReasoningTransitionError> {
        let mut next = *self;
        next.validate()?;
        if next.active_reasoning_item_count == 0 {
            return Ok(false);
        }
        next.active_reasoning_item_count = 0;
        next.finish_active_interval(now)?;
        next.validate()?;
        *self = next;
        Ok(true)
    }

    pub(crate) fn recovery_abort(&mut self) -> Result<bool, AgentReasoningTransitionError> {
        self.validate()?;
        if self.active_reasoning_item_count == 0 {
            return Ok(false);
        }
        self.active_reasoning_item_count = 0;
        self.snapshot.active_since = None;
        self.snapshot.phase = if self.snapshot.last_completed_at.is_some() {
            AgentReasoningPhase::Completed
        } else {
            AgentReasoningPhase::NotObserved
        };
        self.validate()?;
        Ok(true)
    }

    fn finish_active_interval(&mut self, now: u128) -> Result<(), AgentReasoningTransitionError> {
        let active_since = self
            .snapshot
            .active_since
            .take()
            .ok_or(AgentReasoningTransitionError::InvalidCheckpoint)?;
        let elapsed = now
            .checked_sub(active_since)
            .ok_or(AgentReasoningTransitionError::ClockMovedBackwards)?;
        self.snapshot.step_completed_millis = self
            .snapshot
            .step_completed_millis
            .checked_add(elapsed)
            .ok_or(AgentReasoningTransitionError::ArithmeticOverflow)?;
        self.snapshot.turn_completed_millis = self
            .snapshot
            .turn_completed_millis
            .checked_add(elapsed)
            .ok_or(AgentReasoningTransitionError::ArithmeticOverflow)?;
        self.snapshot.phase = AgentReasoningPhase::Completed;
        self.snapshot.last_completed_at = Some(now);
        Ok(())
    }

    pub(crate) fn validate(&self) -> Result<(), AgentReasoningTransitionError> {
        if self.snapshot.step_completed_millis > self.snapshot.turn_completed_millis {
            return Err(AgentReasoningTransitionError::InvalidCheckpoint);
        }
        match self.snapshot.phase {
            AgentReasoningPhase::NotObserved
                if self.active_reasoning_item_count == 0
                    && self.snapshot.active_since.is_none()
                    && self.snapshot.step_completed_millis == 0
                    && self.snapshot.last_completed_at.is_none() => {}
            AgentReasoningPhase::Thinking
                if self.active_reasoning_item_count > 0 && self.snapshot.active_since.is_some() => {
            }
            AgentReasoningPhase::Completed
                if self.active_reasoning_item_count == 0
                    && self.snapshot.active_since.is_none()
                    && self.snapshot.last_completed_at.is_some() => {}
            _ => return Err(AgentReasoningTransitionError::InvalidCheckpoint),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
