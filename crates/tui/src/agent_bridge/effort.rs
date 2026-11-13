//! `/effort` and its keys onto the facade: what is known of the session's
//! model, which the efforts offered are drawn from, and the session's
//! effort set, or stepped a level (Alt+, and Alt+.).

use hanafubuki_agent::{
    AgentHandle, AgentSessionReasoningEffortRequest, AgentSessionRuntimeSnapshot, ModelMetadata,
    ModelReasoningEffort,
};

use super::{AgentBridge, AgentReply, TurnUpdate};
use crate::model_facts::{step_effort, EffortStep};

/// How often a step refused (another step set the effort first) is taken
/// again.
const STEP_ATTEMPTS: usize = 8;

/// What `/effort` and its keys ask of the Agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EffortCall {
    /// What the efforts are offered from.
    List,
    /// The session's effort; `None`: the Provider's default.
    Set(Option<ModelReasoningEffort>),
    /// The effort a level up (`raise`) or down among those offered.
    Step { raise: bool },
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
            EffortCall::Set(effort) => {
                self.spawn(move |agent| async move { set_effort(agent, session, effort).await })
            }
            EffortCall::Step { raise } => self.spawn(move |agent| async move {
                let known = agent
                    .session_model_metadata(session.id)
                    .map_err(|error| error.to_string())?
                    .known_reasoning();
                // Steps sent quicker than they are set (a key held) race:
                // the one refused for a stale revision steps again from the
                // effort the other set.
                let mut attempts = 0;
                loop {
                    let session = agent
                        .snapshot(session.id)
                        .map_err(|error| error.to_string())?;
                    let current = session
                        .reasoning_effort
                        .as_ref()
                        .map(|effort| effort.as_str());
                    let effort = match step_effort(&known, current, raise) {
                        EffortStep::To(effort) => effort,
                        EffortStep::Stays(why) => {
                            return Ok(Some(AgentReply::Update(Box::new(
                                TurnUpdate::EffortUnchanged(why),
                            ))))
                        }
                    };
                    match set_effort(agent.clone(), session, Some(effort)).await {
                        Err(_) if attempts < STEP_ATTEMPTS => attempts += 1,
                        result => return result,
                    }
                }
            }),
        }
        Ok(())
    }
}

async fn set_effort(
    agent: AgentHandle,
    session: AgentSessionRuntimeSnapshot,
    effort: Option<ModelReasoningEffort>,
) -> Result<Option<AgentReply>, String> {
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
}
