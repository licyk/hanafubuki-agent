//! Roles (site/content/docs/roles.en.mdx): named, inheritable definitions
//! of how a session's Agent works, which only narrow it. The configuration
//! reads their definitions per source, and the installed plugins' with
//! their content (`packages/roles.rs`); this module ranks the sources,
//! follows `extends`, merges each chain field by field and keeps where each
//! field came from. A session takes its role from the catalog when it is
//! created or switched (`service/roles.rs`).

mod builtin;
mod reader;
mod resolve;

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::sync::Arc;

use hanafubuki_agent_config::{AgentConfig, RoleDefinition, RoleLayer, RoleOrigin};
use hanafubuki_agent_foundation::agent_settings::ModelReasoningEffort;
use hanafubuki_agent_kernel::AgentCapabilityScope;
use parking_lot::{Mutex, RwLock};

pub use builtin::{DEFAULT_ROLE, EXPLORER_ROLE};
pub(crate) use reader::{AgentRoleCatalogReader, AgentSpawnableRole};

/// The most roles a chain holds: the role itself and the roles it
/// extends, one after the other.
pub const MAX_ROLE_CHAIN: usize = 8;

/// Where a role is defined.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentRoleSource {
    /// Compiled into the Agent.
    BuiltIn,
    /// A role file of the installed plugin `id` (`<plugin>@<marketplace>`),
    /// while it is on.
    Package { id: String, origin: RoleOrigin },
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
            Self::Package { id, origin } => write!(formatter, "plugin `{id}` {origin}"),
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

impl AgentResolvedRole {
    /// What a session takes of the role. `default` narrows nothing: the
    /// empty scope, which a session's record leaves out.
    pub(crate) fn session_role(&self) -> AgentSessionRole {
        if self.name == DEFAULT_ROLE {
            return AgentSessionRole::default();
        }
        AgentSessionRole {
            scope: self.scope.clone(),
            instructions: self.instructions.clone(),
            model: self.model.clone(),
            reasoning_effort: self.reasoning_effort.clone(),
        }
    }
}

/// What a session takes of a role: its scope and instructions, kept by the
/// session, and the model and reasoning effort it selects.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct AgentSessionRole {
    pub(crate) scope: AgentCapabilityScope,
    pub(crate) instructions: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) reasoning_effort: Option<ModelReasoningEffort>,
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

/// The roles of one installed plugin that is on, each named
/// `<plugin>:<role>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentPackageRoles {
    /// `<plugin>@<marketplace>`.
    pub(crate) id: String,
    pub(crate) definitions: Vec<RoleDefinition>,
}

impl AgentRoleCatalog {
    /// The roles a started Agent would resolve now, read without starting
    /// it: those `config` defines and those of the installed plugins under
    /// `plugins_root` that it leaves on (none in `safe_mode`), over the
    /// built-in ones. The plugins are read without opening their store
    /// (nothing written, no lock), so this is safe beside a running Agent;
    /// what reading their roles left out is an issue here too, each naming
    /// its plugin, where a running Agent's catalog leaves it to the plugin.
    /// A configuration that cannot be read leaves the built-in roles, with
    /// an issue.
    pub fn load(config: &AgentConfig, plugins_root: &Path, safe_mode: bool) -> Self {
        let (packages, package_issues) = match config.settings() {
            Ok(settings) => {
                crate::packages::read_installed_roles(plugins_root, &settings, safe_mode)
            }
            Err(error) => (
                Vec::new(),
                vec![format!(
                    "the installed plugins' roles are not read: {error}"
                )],
            ),
        };
        let mut catalog = Self::load_with(config, packages);
        catalog
            .issues
            .extend(package_issues.into_iter().map(|message| AgentRoleIssue {
                role: None,
                message,
            }));
        catalog
    }

    /// The roles of `config` and of `packages`, over the built-in ones.
    pub(crate) fn load_with(config: &AgentConfig, packages: Vec<AgentPackageRoles>) -> Self {
        match config.role_sources() {
            Ok(sources) => resolve::resolve(packages, sources),
            Err(error) => {
                let mut catalog = resolve::resolve(packages, Vec::new());
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

/// The instance's role catalog, rebuilt from the configuration and the
/// installed plugins whenever the settings or the plugins are published.
pub(crate) struct AgentRoles {
    /// Held while one is built, so of two rebuilds the later one's read is
    /// the one kept.
    rebuild: Mutex<()>,
    catalog: RwLock<Arc<AgentRoleCatalog>>,
}

impl AgentRoles {
    pub(crate) fn load(config: &AgentConfig, packages: Vec<AgentPackageRoles>) -> Self {
        Self {
            rebuild: Mutex::new(()),
            catalog: RwLock::new(Arc::new(AgentRoleCatalog::load_with(config, packages))),
        }
    }

    pub(crate) fn current(&self) -> Arc<AgentRoleCatalog> {
        self.catalog.read().clone()
    }

    /// Reads the roles again, with the plugins' roles `packages` gives
    /// (called under the rebuild gate, so the later rebuild reads the later
    /// packages). Blocking: reads the role files.
    pub(crate) fn rebuild(
        &self,
        config: &AgentConfig,
        packages: impl FnOnce() -> Vec<AgentPackageRoles>,
    ) {
        let _rebuild = self.rebuild.lock();
        let catalog = Arc::new(AgentRoleCatalog::load_with(config, packages()));
        *self.catalog.write() = catalog;
    }
}

#[cfg(test)]
mod tests;
