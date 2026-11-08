//! The memory policy of one sampling step: the `memory` plugin's, when it
//! prepared one for the step, or answers shown as they are and no pollution.

use std::sync::Arc;

use hanafubuki_agent_contracts::{
    AgentMemoryStep, AgentMemoryStepPolicy, AgentProcessedMemoryAnswer,
};

#[derive(Clone, Default)]
pub(crate) struct StepMemoryPolicy(Option<Arc<dyn AgentMemoryStepPolicy>>);

impl StepMemoryPolicy {
    /// The policy among a step's prepared values.
    pub(crate) fn of_step(prepared: &hanafubuki_agent_contracts::TurnExtensions) -> Self {
        Self(
            prepared
                .get::<AgentMemoryStep>()
                .map(|step| step.policy.clone()),
        )
    }

    pub(crate) fn excludes_external_context(&self) -> Result<bool, String> {
        self.0
            .as_ref()
            .map_or(Ok(false), |policy| policy.excludes_external_context())
    }

    pub(crate) fn visible_stream_prefix_len(&self, text: &str) -> Result<usize, String> {
        self.0.as_ref().map_or(Ok(text.len()), |policy| {
            policy.visible_stream_prefix_len(text)
        })
    }

    pub(crate) fn process_final_answer(
        &self,
        text: &str,
    ) -> Result<AgentProcessedMemoryAnswer, String> {
        match &self.0 {
            Some(policy) => policy.process_final_answer(text),
            None => Ok(AgentProcessedMemoryAnswer {
                visible_text: text.to_string(),
                citations: Vec::new(),
            }),
        }
    }
}
