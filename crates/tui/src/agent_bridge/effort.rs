//! `/effort` onto the facade: what is known of the session's model, which
//! the efforts offered are drawn from, and the session's effort set.

use hanafubuki_agent::{AgentSessionReasoningEffortRequest, ModelMetadata, ModelReasoningEffort};

use super::{AgentBridge, AgentReply, TurnUpdate};

/// What `/effort` asks of the Agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EffortCall {
    /// What the efforts are offered from.
    List,
    /// The session's effort; `None`: the Provider's default.
    Set(Option<ModelReasoningEffort>),
}

/// What `/effort` offers: the session's model, its effort, and what is
/// known of the model, which the efforts are drawn from.
#[derive(Debug)]
pub(crate) struct EffortChoices {
    pub(crate) model: String,
    pub(crate) current: Option<ModelReasoningEffort>,
    pub(crate) metadata: ModelMetadata,
}

impl AgentBridge {
    pub(super) fn effort_call(&mut self, call: EffortCall) -> Result<(), String> {
        let session = self.current()?;
        match call {
            EffortCall::List => self.spawn(move |agent| async move {
                let metadata = agent
                    .session_model_metadata(session.id)
                    .map_err(|error| error.to_string())?;
                Ok(Some(AgentReply::Update(Box::new(
                    TurnUpdate::EffortChoices(EffortChoices {
                        model: session.model,
                        current: session.reasoning_effort,
                        metadata,
                    }),
                ))))
            }),
            EffortCall::Set(effort) => self.spawn(move |agent| async move {
                let session = agent
                    .set_session_reasoning_effort(AgentSessionReasoningEffortRequest {
                        session_id: session.id,
                        generation: session.generation,
                        revision: session.revision,
                        effort,
                    })
                    .await
                    .map_err(|error| format!("the reasoning effort was not changed: {error}"))?;
                Ok(Some(AgentReply::Update(Box::new(
                    TurnUpdate::EffortChanged(session.reasoning_effort),
                ))))
            }),
        }
        Ok(())
    }
}
