//! Roles (site/content/docs/roles.en.mdx): named, inheritable definitions
//! of how a session's Agent works, which only narrow it. The configuration
//! reads their definitions per source; this module ranks the sources,
//! follows `extends`, merges each chain field by field and keeps where each
//! field came from. A session takes its role from the catalog when it is
//! created or switched (`service/roles.rs`).

mod builtin;
mod resolve;

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use hanafubuki_agent_config::{AgentConfig, RoleLayer, RoleOrigin};
use hanafubuki_agent_foundation::agent_settings::ModelReasoningEffort;
use hanafubuki_agent_kernel::AgentCapabilityScope;
use parking_lot::{Mutex, RwLock};

pub use builtin::{DEFAULT_ROLE, EXPLORER_ROLE};

/// The most roles a chain holds: the role itself and the roles it
/// extends, one after the other.
pub const MAX_ROLE_CHAIN: usize = 8;

/// Where a role is defined.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentRoleSource {
    /// Compiled into the Agent.
    BuiltIn,
    /// A role file or an `[agents]` entry of the user, the profile or a
    /// trusted project.
    Configured {
        layer: RoleLayer,
        origin: RoleOrigin,
    },
}

impl fmt::Display for AgentRoleSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BuiltIn => formatter.write_str("built-in"),
            Self::Configured { layer, origin } => write!(formatter, "{layer} {origin}"),
        }
    }
}

/// One role of a chain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentRoleLink {
    pub name: String,
    pub source: AgentRoleSource,
}

/// A usable role, its chain merged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentResolvedRole {
    pub name: String,
    pub description: String,
    pub source: AgentRoleSource,
    /// The role, then each role it extends, nearest first.
    pub chain: Vec<AgentRoleLink>,
    /// The chain's instructions, merged.
    pub instructions: Option<String>,
    pub model: Option<String>,
    pub reasoning_effort: Option<ModelReasoningEffort>,
    pub spawnable: bool,
    /// What the chain narrows; its `role` is this role's name.
    pub scope: AgentCapabilityScope,
    /// For each field set by the chain (by its key, `tools.allow`), the
    /// roles of the chain whose definition decides it, nearest first.
    pub provenance: BTreeMap<&'static str, Vec<String>>,
}

/// A role no session can have, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentUnusableRole {
    pub name: String,
    pub source: AgentRoleSource,
    pub reason: String,
}

/// What reading or resolving the roles left out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentRoleIssue {
    /// The role it is about, when it is about one.
    pub role: Option<String>,
    pub message: String,
}

impl fmt::Display for AgentRoleIssue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.role {
            Some(role) => write!(formatter, "role `{role}`: {}", self.message),
            None => formatter.write_str(&self.message),
        }
    }
}

/// Every role, resolved: the usable ones, the unusable ones with why, and
/// the issues.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentRoleCatalog {
    roles: BTreeMap<String, AgentResolvedRole>,
    unusable: BTreeMap<String, AgentUnusableRole>,
    issues: Vec<AgentRoleIssue>,
}

impl AgentRoleCatalog {
    /// The roles `config` defines now, over the built-in ones. A
    /// configuration that cannot be read leaves the built-in roles, with
    /// an issue.
    pub fn load(config: &AgentConfig) -> Self {
        match config.role_sources() {
            Ok(sources) => resolve::resolve(sources),
            Err(error) => {
                let mut catalog = resolve::resolve(Vec::new());
                catalog.issues.push(AgentRoleIssue {
                    role: None,
                    message: format!("the roles of the configuration are not read: {error}"),
                });
                catalog
            }
        }
    }

    /// The usable role `name`.
    pub fn role(&self, name: &str) -> Option<&AgentResolvedRole> {
        self.roles.get(name)
    }

    /// The usable roles, by name.
    pub fn roles(&self) -> impl Iterator<Item = &AgentResolvedRole> {
        self.roles.values()
    }

    /// The unusable role `name`.
    pub fn unusable_role(&self, name: &str) -> Option<&AgentUnusableRole> {
        self.unusable.get(name)
    }

    /// The unusable roles, by name.
    pub fn unusable(&self) -> impl Iterator<Item = &AgentUnusableRole> {
        self.unusable.values()
    }

    /// What reading and resolving the roles left out, unusable roles
    /// included, in source order.
    pub fn issues(&self) -> &[AgentRoleIssue] {
        &self.issues
    }
}

/// The instance's role catalog, rebuilt from the configuration whenever
/// the settings or the installed plugins are published.
pub(crate) struct AgentRoles {
    /// Held while one is built, so of two rebuilds the later one's read is
    /// the one kept.
    rebuild: Mutex<()>,
    catalog: RwLock<Arc<AgentRoleCatalog>>,
}

impl AgentRoles {
    pub(crate) fn load(config: &AgentConfig) -> Self {
        Self {
            rebuild: Mutex::new(()),
            catalog: RwLock::new(Arc::new(AgentRoleCatalog::load(config))),
        }
    }

    pub(crate) fn current(&self) -> Arc<AgentRoleCatalog> {
        self.catalog.read().clone()
    }

    /// Reads the roles again. Blocking: reads the role files.
    pub(crate) fn rebuild(&self, config: &AgentConfig) {
        let _rebuild = self.rebuild.lock();
        let catalog = Arc::new(AgentRoleCatalog::load(config));
        *self.catalog.write() = catalog;
    }
}

#[cfg(test)]
mod tests;
