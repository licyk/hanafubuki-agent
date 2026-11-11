//! The role catalog of the instance (site/content/docs/roles.en.mdx), and
//! the roles sessions are given.

use std::sync::Arc;

use hanafubuki_agent_foundation::agent_settings::ModelReasoningEffort;
use hanafubuki_agent_kernel::{
    AgentCapabilityScope, AgentError, AgentSessionId, AgentSessionRuntimeSnapshot,
};

use super::AgentApplicationService;
use crate::roles::{AgentRoleCatalog, DEFAULT_ROLE};
use crate::{AgentSessionModelSelectionRequest, AgentSessionReasoningEffortRequest};

/// What a session takes of a role: its scope and instructions, kept by the
/// session, and the model and reasoning effort it selects.
pub(super) struct AgentSessionRole {
    pub(super) scope: AgentCapabilityScope,
    pub(super) instructions: Option<String>,
    pub(super) model: Option<String>,
    pub(super) reasoning_effort: Option<ModelReasoningEffort>,
}

impl AgentApplicationService {
    /// The roles as the last settings or package publication left them:
    /// read when the Agent is built and again by every
    /// [`Self::reload_packages`], which each settings write runs. A session
    /// takes its role from it when it is created or its role is set.
    pub fn roles(&self) -> Arc<AgentRoleCatalog> {
        self.roles.current()
    }

    /// Gives `session_id` the role `role` of the current catalog: its scope
    /// and instructions, then its model and reasoning effort, if it sets
    /// them, through the model selection. `default` clears the scope. Only
    /// while no turn runs in the session; the scope is committed durably
    /// before the model is selected. Returns the session's snapshot.
    pub async fn set_session_role(
        &self,
        session_id: AgentSessionId,
        role: &str,
    ) -> Result<AgentSessionRuntimeSnapshot, String> {
        let role = self
            .session_role(Some(role))
            .map_err(|error| error.to_string())?;
        let service = self.clone();
        let (scope, instructions) = (role.scope, role.instructions);
        let mut snapshot = hanafubuki_agent_foundation::blocking::run_blocking_operation(
            self.blocking.as_ref(),
            "agent session role worker",
            move || {
                service.core.activate_session(session_id)?;
                service
                    .core
                    .set_session_role(session_id, scope, instructions)
            },
        )
        .await?
        .map_err(|error| error.to_string())?;
        if let Some(model) = role.model.filter(|model| *model != snapshot.model) {
            snapshot = self
                .set_session_model(AgentSessionModelSelectionRequest {
                    session_id,
                    generation: snapshot.generation,
                    revision: snapshot.revision,
                    provider_id: snapshot.provider_id.clone(),
                    model,
                })
                .await?;
        }
        if let Some(effort) = role.reasoning_effort {
            snapshot = self
                .set_session_reasoning_effort(AgentSessionReasoningEffortRequest {
                    session_id,
                    generation: snapshot.generation,
                    revision: snapshot.revision,
                    effort: Some(effort),
                })
                .await?;
        }
        Ok(snapshot)
    }

    /// What a session takes of the role `name` (`default` without one). A
    /// role no source defines, or that is not usable, is invalid input
    /// naming the usable ones.
    pub(super) fn session_role(&self, name: Option<&str>) -> Result<AgentSessionRole, AgentError> {
        let name = name.unwrap_or(DEFAULT_ROLE);
        let catalog = self.roles();
        let Some(role) = catalog.role(name) else {
            let usable = catalog
                .roles()
                .map(|role| role.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let why = match catalog.unusable_role(name) {
                Some(unusable) => format!("the role `{name}` is not usable: {}", unusable.reason),
                None => format!("no role is named `{name}`"),
            };
            return Err(AgentError::InvalidInput(format!(
                "{why}; the usable roles are {usable}"
            )));
        };
        // `default` narrows nothing: the empty scope, which a session's
        // record leaves out.
        if role.name == DEFAULT_ROLE {
            return Ok(AgentSessionRole {
                scope: AgentCapabilityScope::default(),
                instructions: None,
                model: None,
                reasoning_effort: None,
            });
        }
        Ok(AgentSessionRole {
            scope: role.scope.clone(),
            instructions: role.instructions.clone(),
            model: role.model.clone(),
            reasoning_effort: role.reasoning_effort.clone(),
        })
    }

    /// Reads the roles again on the blocking pool; a failure keeps the
    /// catalog as it was.
    pub(super) async fn reload_roles(&self) {
        let roles = self.roles.clone();
        let config = self.config.clone();
        let rebuilt = hanafubuki_agent_foundation::blocking::run_blocking_operation(
            self.blocking.as_ref(),
            "agent role catalog rebuild",
            move || roles.rebuild(&config),
        )
        .await;
        if let Err(error) = rebuilt {
            log::warn!("the role catalog was not read again: {error}");
        }
    }
}
