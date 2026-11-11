//! Role definitions as the configuration holds them
//! (site/content/docs/roles.en.mdx): the role files of an `agents/`
//! directory and the `[agents.<name>]` entries of a `config.toml`, read per
//! source and tolerantly. Each key is read on its own: one that cannot be
//! used is left out with an issue, and an unknown key (a Codex setting
//! included) is ignored with an issue. A key that narrows (`extends`,
//! `permissionModeCap`, `tools`, `plugins`) and cannot be read makes its
//! role unusable instead, since leaving it out would allow more than the
//! file says. Inheritance and precedence are the facade's.

mod decode;
mod discover;

use std::fmt;
use std::path::{Path, PathBuf};

use hanafubuki_agent_foundation::agent_settings::{AgentPermissionMode, ModelReasoningEffort};
use serde_json::{Map, Value};

use crate::keys::{AGENTS, PROFILES};
use crate::layers::{is_trusted, ConfigLayers};
use crate::PROJECT_CONFIG_DIR;

/// The directory of role files beside a `config.toml`.
pub const ROLE_DIR: &str = "agents";
/// The longest role name, in bytes.
pub const MAX_ROLE_NAME_BYTES: usize = 128;

/// Which configuration a role comes from, lowest precedence first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RoleLayer {
    /// The user's `agents/` and `[agents]` of the user's `config.toml`.
    User,
    /// `[profiles.<name>.agents]` of the profile `-p` selects.
    Profile { name: String },
    /// A trusted project's `.hanafubuki-agent/agents/` and `[agents]`.
    Project,
}

impl fmt::Display for RoleLayer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::User => formatter.write_str("user"),
            Self::Profile { name } => write!(formatter, "profile `{name}`"),
            Self::Project => formatter.write_str("project"),
        }
    }
}

/// Where a definition is written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RoleOrigin {
    /// A role file of an `agents/` directory.
    File { path: PathBuf },
    /// An entry of a `config.toml` (`key` is `agents.<name>` or
    /// `profiles.<profile>.agents.<name>`), with the role file its
    /// `configFile` names.
    Table {
        file: PathBuf,
        key: String,
        config_file: Option<PathBuf>,
    },
}

impl fmt::Display for RoleOrigin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::File { path } => write!(formatter, "file {}", path.display()),
            Self::Table {
                file,
                key,
                config_file,
            } => {
                write!(formatter, "[{key}] in {}", file.display())?;
                match config_file {
                    Some(config_file) => {
                        write!(formatter, " (configFile {})", config_file.display())
                    }
                    None => Ok(()),
                }
            }
        }
    }
}

/// How a role's `instructions` join those of the role it extends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InstructionsMode {
    /// The parent's, a blank line, then the role's.
    #[default]
    Append,
    /// The role's alone.
    Replace,
}

impl InstructionsMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Append => "append",
            Self::Replace => "replace",
        }
    }
}

/// One role as its file or entry writes it: each key `None` (or empty)
/// when it is not set or could not be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoleDefinition {
    pub name: String,
    pub origin: RoleOrigin,
    pub description: Option<String>,
    pub extends: Option<String>,
    pub instructions: Option<String>,
    pub instructions_mode: InstructionsMode,
    pub model: Option<String>,
    pub reasoning_effort: Option<ModelReasoningEffort>,
    pub permission_mode_cap: Option<AgentPermissionMode>,
    pub spawnable: Option<bool>,
    /// `tools.allow` as written; `None` allows what the parent allows.
    pub tools_allow: Option<Vec<String>>,
    pub tools_deny: Vec<String>,
    pub plugins_off: Vec<String>,
    /// Why the role cannot be used, when it cannot.
    pub unusable: Option<String>,
    /// What its reading left out, each completing "<origin>: ".
    pub issues: Vec<String>,
}

impl RoleDefinition {
    /// A definition with nothing set, as a role file without keys holds.
    pub fn empty(name: String, origin: RoleOrigin) -> Self {
        Self {
            name,
            origin,
            description: None,
            extends: None,
            instructions: None,
            instructions_mode: InstructionsMode::Append,
            model: None,
            reasoning_effort: None,
            permission_mode_cap: None,
            spawnable: None,
            tools_allow: None,
            tools_deny: Vec::new(),
            plugins_off: Vec::new(),
            unusable: None,
            issues: Vec::new(),
        }
    }
}

/// The roles one source defines, in its order: its `[agents]` entries by
/// name, then its role files by path. Two of the same name are both here;
/// the first keeps the name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoleSource {
    pub layer: RoleLayer,
    pub definitions: Vec<RoleDefinition>,
    /// What could not be read as a role at all.
    pub issues: Vec<String>,
}

/// The role files an installed plugin's `agents` component holds, by path,
/// each named as in an `agents/` directory (the plugin's name is the
/// caller's to add), and what could not be read as a role at all.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoleFiles {
    pub definitions: Vec<RoleDefinition>,
    pub issues: Vec<String>,
}

/// The role files of `paths`: each a directory, whose `.toml` files (in
/// its subdirectories too) are read, or one role file. Read as the user's
/// role files are, tolerantly; nothing is ever written beside them.
pub fn read_role_files(paths: &[PathBuf]) -> RoleFiles {
    discover::read_paths(paths)
}

/// A name a role can have: 1 to [`MAX_ROLE_NAME_BYTES`] bytes without
/// spaces, control characters or `:`, which separates an installed
/// plugin's name from its role's (`<plugin>:<role>`).
pub fn is_role_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_ROLE_NAME_BYTES
        && !name
            .chars()
            .any(|c| c == ':' || c.is_whitespace() || c.is_control())
}

/// Every source the configuration has, lowest first: the user's (beside
/// `user_file`, whose document is `user`), the profile's and the trusted
/// project's of `layers`.
pub(crate) fn read_role_sources(
    user_file: &Path,
    user: &Map<String, Value>,
    layers: Option<&ConfigLayers>,
) -> Vec<RoleSource> {
    let folder = user_file.parent().unwrap_or(Path::new(""));
    let mut sources = vec![read_source(
        RoleLayer::User,
        user_file,
        user.get(AGENTS),
        folder,
    )];
    let Some(layers) = layers else {
        return sources;
    };
    if let Some(name) = &layers.profile {
        let agents = user
            .get(PROFILES)
            .and_then(|profiles| profiles.get(name))
            .and_then(|profile| profile.get(AGENTS));
        let mut source = RoleSource {
            layer: RoleLayer::Profile { name: name.clone() },
            definitions: Vec::new(),
            issues: Vec::new(),
        };
        let key = format!("{PROFILES}.{name}.{AGENTS}");
        discover::read_entries(user_file, &key, agents, folder, &mut source);
        sources.push(source);
    }
    if let Some(project) = layers.project.as_deref() {
        if is_trusted(user, project) {
            sources.push(read_project(project));
        } else if project.join(PROJECT_CONFIG_DIR).join(ROLE_DIR).is_dir() {
            log::info!(
                "the project's roles in {} are not read: the project is not trusted",
                project.display()
            );
        }
    }
    sources
}

/// A trusted project's source. Its `config.toml` is read here as it is,
/// never repaired: it is the project's, not the user's.
fn read_project(project: &Path) -> RoleSource {
    let folder = project.join(PROJECT_CONFIG_DIR);
    let file = ConfigLayers::project_file(project);
    let (document, issue) = match discover::read_document(&file) {
        Ok(document) => (document, None),
        Err(reason) => (Map::new(), Some(reason)),
    };
    let mut source = read_source(RoleLayer::Project, &file, document.get(AGENTS), &folder);
    source.issues.extend(issue);
    source
}

/// The source of `layer` in `folder`: the `[agents]` entries of its
/// `config.toml` (`file`), then the role files of its `agents/`.
fn read_source(layer: RoleLayer, file: &Path, agents: Option<&Value>, folder: &Path) -> RoleSource {
    let mut source = RoleSource {
        layer,
        definitions: Vec::new(),
        issues: Vec::new(),
    };
    let declared = discover::read_entries(file, AGENTS, agents, folder, &mut source);
    discover::read_directory(&folder.join(ROLE_DIR), &declared, &mut source);
    source
}

#[cfg(test)]
mod tests;
