//! What a role narrows: which tools it allows and denies, which plugins'
//! contributions it turns off, and the permission mode it caps. Every
//! combination here only narrows, so a scope built from any chain of roles
//! never allows what one of them removes.

use std::collections::BTreeSet;
use std::fmt;

use hanafubuki_agent_approval::AgentPermissionMode;

/// The longest tool pattern, in bytes.
pub const MAX_TOOL_PATTERN_BYTES: usize = 256;

/// One entry of a role's `tools.allow` or `tools.deny`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ToolPattern {
    /// A flattened tool name (`namespace__name`), `*` matching any run of
    /// characters.
    Name(String),
    /// Every tool the plugin registers or sources: a row's name, or
    /// `guest:<id>` for an installed plugin's code.
    Plugin(String),
    /// Every tool a provider sources, its id matched as a `*` pattern
    /// (`mcp.github`, `plugin.demo.*`).
    Provider(String),
}

const PLUGIN_PREFIX: &str = "plugin:";
const PROVIDER_PREFIX: &str = "provider:";

impl ToolPattern {
    /// The pattern `text` writes. The error says why it is not one.
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.is_empty() || text.len() > MAX_TOOL_PATTERN_BYTES {
            return Err(format!(
                "a tool pattern holds 1 to {MAX_TOOL_PATTERN_BYTES} bytes"
            ));
        }
        if text.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err("a tool pattern holds no spaces or control characters".to_string());
        }
        let (pattern, rest) = if let Some(rest) = text.strip_prefix(PLUGIN_PREFIX) {
            (Self::Plugin(rest.to_string()), rest)
        } else if let Some(rest) = text.strip_prefix(PROVIDER_PREFIX) {
            (Self::Provider(rest.to_string()), rest)
        } else if let Some((kind, _)) = text.split_once(':') {
            return Err(format!(
                "`{kind}:` is not a kind of tool pattern (a tool name, `plugin:` or `provider:`)"
            ));
        } else {
            (Self::Name(text.to_string()), text)
        };
        if rest.is_empty() {
            return Err(format!("`{text}` names nothing after its kind"));
        }
        Ok(pattern)
    }

    /// Whether `tool` is one this pattern names.
    pub fn matches(&self, tool: &ToolIdentity<'_>) -> bool {
        match self {
            Self::Name(pattern) => glob_matches(pattern, tool.name),
            Self::Plugin(owner) => owner == tool.owner,
            Self::Provider(pattern) => tool
                .provider
                .is_some_and(|provider| glob_matches(pattern, provider)),
        }
    }
}

impl fmt::Display for ToolPattern {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Name(name) => formatter.write_str(name),
            Self::Plugin(owner) => write!(formatter, "{PLUGIN_PREFIX}{owner}"),
            Self::Provider(provider) => write!(formatter, "{PROVIDER_PREFIX}{provider}"),
        }
    }
}

/// What a pattern is matched against: a tool as the plan sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToolIdentity<'a> {
    /// The flattened name the model calls.
    pub name: &'a str,
    /// The plugin that registered or sourced it (`guest:<id>` for a guest).
    pub owner: &'a str,
    /// The provider that sourced it; `None` for a registered tool.
    pub provider: Option<&'a str>,
}

/// Which tools a scope allows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentToolScope {
    /// A tool is allowed when it matches a pattern of every list. Each
    /// role of a chain that sets `tools.allow` adds its list, so the
    /// allowed tools are those every one of them allows; no list allows
    /// every tool, and an empty list allows none.
    pub allow: Vec<BTreeSet<ToolPattern>>,
    /// A tool matching one of these is not allowed, whatever `allow` says.
    pub deny: BTreeSet<ToolPattern>,
}

impl AgentToolScope {
    pub fn allows(&self, tool: &ToolIdentity<'_>) -> bool {
        self.allow
            .iter()
            .all(|list| list.iter().any(|pattern| pattern.matches(tool)))
            && !self.deny.iter().any(|pattern| pattern.matches(tool))
    }
}

/// The narrowing a role resolves to: its tools, the plugins whose
/// contributions it turns off, and its permission-mode cap.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentCapabilityScope {
    /// The role it came from; a name, not a reference.
    pub role: Option<String>,
    pub tools: AgentToolScope,
    /// Row names, or `guest:<id>`.
    pub plugins_off: BTreeSet<String>,
    pub permission_mode_cap: Option<AgentPermissionMode>,
}

impl AgentCapabilityScope {
    /// This scope narrowed by `child`: the allow lists of both, the deny
    /// patterns and plugins off of either, the stricter cap. The result
    /// carries `child`'s role.
    pub fn narrow(&self, child: &Self) -> Self {
        let mut allow = self.tools.allow.clone();
        for list in &child.tools.allow {
            if !allow.contains(list) {
                allow.push(list.clone());
            }
        }
        let permission_mode_cap = match (self.permission_mode_cap, child.permission_mode_cap) {
            (Some(parent), Some(child)) => Some(stricter_permission_mode(parent, child)),
            (parent, child) => parent.or(child),
        };
        Self {
            role: child.role.clone(),
            tools: AgentToolScope {
                allow,
                deny: self.tools.deny.union(&child.tools.deny).cloned().collect(),
            },
            plugins_off: self
                .plugins_off
                .union(&child.plugins_off)
                .cloned()
                .collect(),
            permission_mode_cap,
        }
    }
}

/// The stricter of two permission modes: Manual, then AutoReview, then
/// FullAccess.
pub fn stricter_permission_mode(
    first: AgentPermissionMode,
    second: AgentPermissionMode,
) -> AgentPermissionMode {
    if strictness(second) > strictness(first) {
        second
    } else {
        first
    }
}

fn strictness(mode: AgentPermissionMode) -> u8 {
    match mode {
        AgentPermissionMode::Manual => 2,
        AgentPermissionMode::AutoReview => 1,
        AgentPermissionMode::FullAccess => 0,
    }
}

/// Whether `text` matches `pattern`, whose `*` matches any run of
/// characters, the empty one included.
fn glob_matches(pattern: &str, text: &str) -> bool {
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or_default();
    let Some(mut rest) = text.strip_prefix(first) else {
        return false;
    };
    let mut parts: Vec<&str> = parts.collect();
    let Some(last) = parts.pop() else {
        return rest.is_empty();
    };
    for part in parts {
        match rest.find(part) {
            Some(index) => rest = &rest[index + part.len()..],
            None => return false,
        }
    }
    rest.len() >= last.len() && rest.ends_with(last)
}

#[cfg(test)]
mod tests;
