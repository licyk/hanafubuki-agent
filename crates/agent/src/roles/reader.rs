//! The narrow port through which the `subagents` plugin reads the roles:
//! the spawnable ones and what a child takes of one, from the catalog as
//! it is at each call. The application constructs it over its catalog; the
//! plugin never holds a catalog of its own.

use std::sync::Arc;

use super::{AgentRoles, AgentSessionRole};

/// A role `spawn_agent` may give a new agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentSpawnableRole {
    pub(crate) name: String,
    pub(crate) description: String,
}

#[derive(Clone)]
pub(crate) struct AgentRoleCatalogReader {
    roles: Arc<AgentRoles>,
}

impl AgentRoleCatalogReader {
    pub(crate) fn new(roles: Arc<AgentRoles>) -> Self {
        Self { roles }
    }

    /// The spawnable roles, by name.
    pub(crate) fn spawnable(&self) -> Vec<AgentSpawnableRole> {
        self.roles
            .current()
            .roles()
            .filter(|role| role.spawnable)
            .map(|role| AgentSpawnableRole {
                name: role.name.clone(),
                description: role.description.clone(),
            })
            .collect()
    }

    /// What a child spawned as `name` takes of it; the spawnable roles'
    /// names, by name, when no usable role `name` is spawnable.
    pub(crate) fn spawn_role(&self, name: &str) -> Result<AgentSessionRole, Vec<String>> {
        let catalog = self.roles.current();
        match catalog.role(name) {
            Some(role) if role.spawnable => Ok(role.session_role()),
            _ => Err(catalog
                .roles()
                .filter(|role| role.spawnable)
                .map(|role| role.name.clone())
                .collect()),
        }
    }
}
