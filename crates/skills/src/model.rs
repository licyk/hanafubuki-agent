//! The Skill catalog's public vocabulary.
//!
//! Identity is a path, not a digest. A Skill is the directory it lives in, so
//! its identity is the scope it was discovered under plus that directory's path
//! relative to the scope root — stable across edits, readable in a log, and
//! resolvable back to exactly one place on disk. Nothing here carries a content
//! hash, a generation, or a capability token: a Skill that changes on disk is
//! the same Skill, and the catalog simply reads it again.

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Deserializer, Serialize};

/// Files one Skill directory may contain. Generous: real packages carry a
/// licence, a readme, reference documents, scripts, and sample assets. A
/// directory past this is not a Skill package, it is a checkout that happens to
/// hold one.
pub const MAX_SKILL_FILES: usize = 4096;
/// Bytes one file *written through this catalog* may occupy. Discovery does not
/// apply it: what a package already ships is the package's business.
pub const MAX_SKILL_FILE_BYTES: u64 = 4 * 1024 * 1024;

const MAX_ID_BYTES: usize = 200;
const MAX_SEGMENT_BYTES: usize = 96;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillScope {
    /// Bundled with the application and replaced on upgrade.
    System,
    /// The managed user root: installed, authored, and deletable here.
    User,
    /// `.agents/skills` beside the Agent's workspace roots. Read-only.
    Repo,
    /// An enabled installed plugin's Skills. Read-only.
    Plugin,
}

impl SkillScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Repo => "repo",
            Self::Plugin => "plugin",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "system" => Some(Self::System),
            "user" => Some(Self::User),
            "repo" => Some(Self::Repo),
            "plugin" => Some(Self::Plugin),
            _ => None,
        }
    }
}

impl fmt::Display for SkillScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// `"<scope>:<directory path relative to that scope's root>"`, for example
/// `user:pdf-tools` or `repo:web/lint`; a plugin's Skill names its package
/// too, `plugin:<plugin>@<marketplace>:<path>`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct SkillId(String);

impl SkillId {
    pub fn new(scope: SkillScope, relative: &str) -> Result<Self, SkillError> {
        Self::from_untrusted(format!("{scope}:{relative}"))
    }

    pub fn from_untrusted(value: impl Into<String>) -> Result<Self, SkillError> {
        let value = value.into();
        let invalid = || SkillError::InvalidId(value.clone());
        if value.len() > MAX_ID_BYTES {
            return Err(invalid());
        }
        let (scope, relative) = value.split_once(':').ok_or_else(invalid)?;
        let scope = SkillScope::parse(scope).ok_or_else(invalid)?;
        let relative = if scope == SkillScope::Plugin {
            let (package, relative) = relative.split_once(':').ok_or_else(invalid)?;
            if !is_package_id(package) {
                return Err(invalid());
            }
            relative
        } else {
            relative
        };
        if relative.is_empty() {
            return Err(invalid());
        }
        for segment in relative.split('/') {
            let valid = !segment.is_empty()
                && segment.len() <= MAX_SEGMENT_BYTES
                && segment != "."
                && segment != ".."
                && !segment.starts_with('.')
                && !segment.chars().any(|character| {
                    character.is_control()
                        || matches!(character, '\\' | ':' | '<' | '>' | '"' | '|')
                });
            if !valid {
                return Err(invalid());
            }
        }
        Ok(Self(value))
    }

    pub fn scope(&self) -> SkillScope {
        let (scope, _) = self.0.split_once(':').expect("a validated id has a scope");
        SkillScope::parse(scope).expect("a validated id has a known scope")
    }

    /// The Skill directory's path relative to its scope root (a plugin's
    /// Skill root), `/`-separated.
    pub fn relative_path(&self) -> &str {
        let (_, relative) = self.0.split_once(':').expect("a validated id has a scope");
        match self.scope() {
            SkillScope::Plugin => relative
                .split_once(':')
                .map_or(relative, |(_, relative)| relative),
            _ => relative,
        }
    }

    /// The plugin a plugin's Skill comes from, `<plugin>@<marketplace>`.
    pub fn plugin(&self) -> Option<&str> {
        let (_, relative) = self.0.split_once(':').expect("a validated id has a scope");
        (self.scope() == SkillScope::Plugin)
            .then(|| relative.split_once(':').map(|(package, _)| package))
            .flatten()
    }

    /// The last path segment — the Skill's folder name.
    pub fn folder_name(&self) -> &str {
        self.relative_path()
            .rsplit('/')
            .next()
            .expect("a validated id has one segment")
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// `<plugin>@<marketplace>`, as the package store names an installed plugin.
fn is_package_id(value: &str) -> bool {
    value.split_once('@').is_some_and(|(plugin, marketplace)| {
        !plugin.is_empty()
            && !marketplace.is_empty()
            && value.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | '@')
            })
    })
}

impl fmt::Display for SkillId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for SkillId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::from_untrusted(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillResourceKind {
    /// Prose the model reads: `references/**` and any other Markdown.
    Reference,
    /// `scripts/**`, runnable through the approved script tool.
    Script,
    /// Everything else the package ships.
    Asset,
}

/// One file inside a Skill, addressed by its package-relative path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillResourceDescriptor {
    pub path: String,
    pub kind: SkillResourceKind,
    pub size: u64,
}

/// Presentation metadata from an optional `agents/openai.yaml`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInterface {
    pub display_name: Option<String>,
    pub short_description: Option<String>,
    pub icon_small: Option<String>,
    pub icon_large: Option<String>,
    pub brand_color: Option<String>,
    pub default_prompt: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillMcpTransport {
    Stdio,
    StreamableHttp,
    Sse,
}

/// An MCP server a Skill says it needs. Declarative only: the MCP authority
/// decides whether it is actually reachable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillToolDependency {
    pub server_id: String,
    pub description: String,
    pub transport: Option<SkillMcpTransport>,
    pub url: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillAvailabilityState {
    Ready,
    Missing,
    InstalledDisabled,
    NeedsAuthentication,
    IncompatibleTransport,
    InvalidConfiguration,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillAvailability {
    pub state: SkillAvailabilityState,
    pub reason: Option<String>,
}

impl Default for SkillAvailability {
    fn default() -> Self {
        Self {
            state: SkillAvailabilityState::Ready,
            reason: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillCatalogEntry {
    pub id: SkillId,
    pub scope: SkillScope,
    /// Frontmatter `name`, defaulting to the folder name.
    pub name: String,
    pub description: String,
    pub short_description: Option<String>,
    pub license: Option<String>,
    pub version: Option<String>,
    pub allow_implicit_invocation: bool,
    pub interface: SkillInterface,
    pub dependencies: Vec<SkillToolDependency>,
    /// Native text of the Skill directory, for people and logs to read.
    pub directory: String,
    /// Whether this Skill may be edited and deleted through the catalog.
    pub editable: bool,
    pub enabled: bool,
    pub availability: SkillAvailability,
    pub resources: Vec<SkillResourceDescriptor>,
}

impl SkillCatalogEntry {
    /// What to show as the Skill's title.
    pub fn display_name(&self) -> &str {
        self.interface.display_name.as_deref().unwrap_or(&self.name)
    }

    /// The shortest accurate description available.
    pub fn summary(&self) -> &str {
        self.interface
            .short_description
            .as_deref()
            .or(self.short_description.as_deref())
            .unwrap_or(&self.description)
    }
}

/// Whether a [`SkillLoadIssue`] cost the user a Skill.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillIssueSeverity {
    /// A directory that was not loaded: a Skill that failed to load, or a
    /// directory under a root (or the root) that could not be read.
    Error,
    /// Everything was loaded that could be: a Skill that loaded with a field
    /// ignored or shortened, a Skill shadowed by a nearer one with the same
    /// id, or a watcher that stopped refreshing the catalog.
    Warning,
}

/// Something wrong under a Skill root. Reported beside the catalog rather
/// than failing it, so one broken package never hides the rest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillLoadIssue {
    pub severity: SkillIssueSeverity,
    /// Native text of the offending directory or file.
    pub path: String,
    pub message: String,
}

impl SkillLoadIssue {
    pub fn error(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: SkillIssueSeverity::Error,
            path: path.into(),
            message: message.into(),
        }
    }

    pub fn warning(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: SkillIssueSeverity::Warning,
            path: path.into(),
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillCatalogSnapshot {
    pub revision: u64,
    pub entries: Vec<SkillCatalogEntry>,
    pub issues: Vec<SkillLoadIssue>,
}

impl SkillCatalogSnapshot {
    pub fn entry(&self, id: &SkillId) -> Option<&SkillCatalogEntry> {
        self.entries.iter().find(|entry| &entry.id == id)
    }
}

/// A Skill and its complete `SKILL.md`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRead {
    pub entry: SkillCatalogEntry,
    /// The whole document, frontmatter included.
    pub instructions: String,
}

/// One text file inside a Skill, read for authoring.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillFileRead {
    pub id: SkillId,
    pub path: String,
    pub content: String,
}

/// One resource read for the model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillResourceRead {
    pub id: SkillId,
    pub path: String,
    pub kind: SkillResourceKind,
    pub content: String,
}

/// Bytes of a packaged asset. Not serializable: assets cross into image and
/// attachment handling, never into catalog or frontend data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillAssetRead {
    pub id: SkillId,
    pub path: String,
    pub bytes: Vec<u8>,
}

/// What a candidate source directory would install as.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillSourceInspection {
    pub name: String,
    pub description: String,
    pub folder_name: String,
    pub resources: Vec<SkillResourceDescriptor>,
    /// Whether the managed user root already holds this folder name.
    pub conflicts: bool,
}

/// Where the System and user Skills are discovered and where the managed ones
/// live. The repository roots follow the Agent's workspace and are set on the
/// service as it changes (`SkillService::set_repo_roots`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillRoots {
    /// Bundled Skills, replaced from the embedded copy on every start.
    pub system: PathBuf,
    /// The managed root: installs, authoring, and deletion happen here.
    pub user: PathBuf,
    /// Production adapters install the embedded System Skills. Test harnesses
    /// disable that to supply isolated fixtures.
    pub install_bundled_system_skills: bool,
    pub state_file: PathBuf,
}

/// An enabled installed plugin's Skill root: its `skills/` (or the
/// directories its manifest names) or the Skills its commands became.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginSkillRoot {
    /// `<plugin>@<marketplace>`, which the Skills' ids carry.
    pub plugin: String,
    /// The manifest's name, which prefixes the Skills' names
    /// (`<namespace>:<name>`), as Codex names a plugin's Skills.
    pub namespace: String,
    pub path: PathBuf,
    /// An Agent Plugins package's Skills are its root's direct children;
    /// other roots are searched in depth.
    pub direct_children_only: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SkillCatalogListRequest {
    pub force_reload: bool,
}

/// Install a Skill by copying a directory that contains `SKILL.md` into the
/// managed root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillInstallRequest {
    pub source_directory: PathBuf,
    /// Replace an existing managed Skill of the same folder name.
    pub overwrite: bool,
}

/// Author a new Skill directly in the managed root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillCreateRequest {
    pub name: String,
    pub description: String,
}

/// Copy an existing Skill of any scope into the managed root under a new name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillDuplicateRequest {
    pub source: SkillId,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillWriteFileRequest {
    pub id: SkillId,
    pub path: String,
    pub content: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillDeleteFileRequest {
    pub id: SkillId,
    pub path: String,
}

/// The enabled Skills captured for one user turn, with their documents.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillTurnCatalog {
    pub catalog_revision: u64,
    /// The Skills the turn may use.
    pub entries: Vec<SkillTurnEntry>,
    /// Enabled Skills the turn may not use, with why: listed so the model can
    /// tell the user what is missing instead of guessing.
    pub unavailable: Vec<SkillTurnUnavailable>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillTurnEntry {
    pub id: SkillId,
    pub name: String,
    pub description: String,
    pub allow_implicit_invocation: bool,
    pub dependencies: Vec<SkillToolDependency>,
    /// Native text of the Skill directory, where its scripts and assets run
    /// from.
    pub directory: String,
    /// The bounded complete `SKILL.md`, frontmatter included.
    pub document: String,
    pub resources: Vec<SkillResourceDescriptor>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillTurnUnavailable {
    pub id: SkillId,
    pub name: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SkillError {
    InvalidId(String),
    InvalidName(String),
    InvalidPath(String),
    InvalidManifest(String),
    LimitExceeded(String),
    NotFound(String),
    ResourceNotFound(String),
    AlreadyExists(String),
    ReadOnly(String),
    Disabled(String),
    UnsupportedResource(String),
    Io(String),
}

impl fmt::Display for SkillError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId(value) => write!(formatter, "invalid Skill id: {value}"),
            Self::InvalidName(message) => write!(formatter, "invalid Skill name: {message}"),
            Self::InvalidPath(message) => write!(formatter, "invalid Skill path: {message}"),
            Self::InvalidManifest(message) => write!(formatter, "invalid SKILL.md: {message}"),
            Self::LimitExceeded(message) => write!(formatter, "Skill limit exceeded: {message}"),
            Self::NotFound(id) => write!(formatter, "Skill not found: {id}"),
            Self::ResourceNotFound(path) => write!(formatter, "Skill resource not found: {path}"),
            Self::AlreadyExists(name) => write!(formatter, "a Skill named {name} already exists"),
            Self::ReadOnly(id) => write!(formatter, "Skill {id} is not editable"),
            Self::Disabled(id) => write!(formatter, "Skill {id} is turned off"),
            Self::UnsupportedResource(path) => {
                write!(formatter, "Skill resource {path} is not bounded text")
            }
            Self::Io(message) => write!(formatter, "Skill I/O failed: {message}"),
        }
    }
}

impl std::error::Error for SkillError {}

#[cfg(test)]
mod tests;
