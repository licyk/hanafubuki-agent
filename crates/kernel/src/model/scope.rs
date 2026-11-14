//! What a role narrows: which tools it allows and denies, which plugins'
//! contributions it turns off, and the permission mode it caps. Every
//! combination here only narrows, so a scope built from any chain of roles
//! never allows what one of them removes. A session keeps its scope in its
//! configuration (site/content/docs/plugin-rules.en.mdx, "Capability
//! scope").

use std::collections::BTreeSet;
use std::fmt;

use hanafubuki_agent_approval::AgentPermissionMode;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

/// The longest tool pattern, in bytes.
pub const MAX_TOOL_PATTERN_BYTES: usize = 256;

/// The most tool patterns a scope holds, its allow lists' and its deny
/// patterns together.
pub const MAX_SCOPE_TOOL_PATTERNS: usize = 128;

/// The rows that are the Agent itself: a scope never turns them off.
pub const CORE_PLUGINS: [&str; 3] = ["agent-host-services", "agent-kernel", "agent-wake"];

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

impl Serialize for ToolPattern {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
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
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentToolScope {
    /// A tool is allowed when it matches a pattern of every list. Each
    /// role of a chain that sets `tools.allow` adds its list, so the
    /// allowed tools are those every one of them allows; no list allows
    /// every tool, and an empty list allows none.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub allow: Vec<BTreeSet<ToolPattern>>,
    /// A tool matching one of these is not allowed, whatever `allow` says.
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub deny: BTreeSet<ToolPattern>,
}

impl AgentToolScope {
    /// Whether it allows every tool.
    pub fn is_unrestricted(&self) -> bool {
        self.allow.is_empty() && self.deny.is_empty()
    }

    pub fn allows(&self, tool: &ToolIdentity<'_>) -> bool {
        self.allow
            .iter()
            .all(|list| list.iter().any(|pattern| pattern.matches(tool)))
            && !self.deny.iter().any(|pattern| pattern.matches(tool))
    }
}

/// The narrowing a role resolves to: its tools, the plugins whose
/// contributions it turns off, and its permission-mode cap. The default
/// scope narrows nothing and is the `default` role's.
///
/// It is persisted with the session's configuration and read tolerantly
/// (see its `Deserialize`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilityScope {
    /// The role it came from; a name, not a reference: a role edited or
    /// deleted later never changes a scope made from it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "AgentToolScope::is_unrestricted")]
    pub tools: AgentToolScope,
    /// Row names, or `guest:<id>`.
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub plugins_off: BTreeSet<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_mode_cap: Option<AgentPermissionMode>,
}

/// What a session's snapshot shows of its scope: counts, and the plugins
/// it turns off.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilityScopeSummary {
    /// How many allow lists a tool must match; 0 allows every tool.
    pub allow_lists: usize,
    pub allow_patterns: usize,
    pub deny_patterns: usize,
    pub plugins_off: Vec<String>,
    pub permission_mode_cap: Option<AgentPermissionMode>,
}

impl AgentCapabilityScope {
    /// Whether it narrows nothing and names no role: the `default` role's.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// `mode`, tightened to the cap.
    pub fn capped_permission_mode(&self, mode: AgentPermissionMode) -> AgentPermissionMode {
        self.permission_mode_cap
            .map_or(mode, |cap| stricter_permission_mode(mode, cap))
    }

    /// Whether the plugin `owner`'s contributions are off.
    pub fn turns_off(&self, owner: &str) -> bool {
        self.plugins_off.contains(owner)
    }

    /// The summary a snapshot shows; `None` for the empty scope.
    pub fn summary(&self) -> Option<AgentCapabilityScopeSummary> {
        (!self.is_empty()).then(|| AgentCapabilityScopeSummary {
            allow_lists: self.tools.allow.len(),
            allow_patterns: self.tools.allow.iter().map(BTreeSet::len).sum(),
            deny_patterns: self.tools.deny.len(),
            plugins_off: self.plugins_off.iter().cloned().collect(),
            permission_mode_cap: self.permission_mode_cap,
        })
    }

    /// Puts the scope in its one form: allow lists sorted and each once,
    /// the core rows never off. A tool scope past the bounds (more than
    /// [`MAX_SCOPE_TOOL_PATTERNS`] patterns, or one longer than
    /// [`MAX_TOOL_PATTERN_BYTES`]) becomes one that allows no tool, since
    /// dropping any of its patterns could widen it. Returns what it
    /// repaired.
    pub fn normalize(&mut self) -> Vec<&'static str> {
        let mut repaired = Vec::new();
        self.tools.allow.sort();
        self.tools.allow.dedup();
        let patterns = self.tools.allow.iter().flatten().chain(&self.tools.deny);
        let count = patterns.clone().count();
        if count > MAX_SCOPE_TOOL_PATTERNS
            || patterns
                .clone()
                .any(|pattern| pattern.to_string().len() > MAX_TOOL_PATTERN_BYTES)
        {
            self.tools = AgentToolScope {
                allow: vec![BTreeSet::new()],
                deny: BTreeSet::new(),
            };
            repaired.push("capability scope tools (past their bounds: no tool is allowed)");
        }
        let plugins = self.plugins_off.len();
        self.plugins_off
            .retain(|plugin| !CORE_PLUGINS.contains(&plugin.as_str()));
        if self.plugins_off.len() != plugins {
            repaired.push("capability scope plugins off (the core rows are never off)");
        }
        repaired
    }

    /// This scope narrowed by `child`: the allow lists of both, the deny
    /// patterns and plugins off of either, the stricter cap. The result
    /// carries `child`'s role, else this scope's.
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
            role: child.role.clone().or_else(|| self.role.clone()),
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

/// Reads a persisted scope field by field, so a field that cannot be read
/// is left at its default rather than failing the session. What is read
/// never widens what was written, except where the data itself is lost:
/// an allow list that is not a list allows nothing, and a pattern this
/// build cannot read is dropped from its list (in an allow list, it then
/// allows nothing more; a deny pattern denies what it no longer names).
impl<'de> Deserialize<'de> for AgentCapabilityScope {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let tools = value.get("tools");
        let strings = |list: Option<&Value>| -> Vec<String> {
            list.and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default()
        };
        let patterns = |list: Option<&Value>| -> BTreeSet<ToolPattern> {
            strings(list)
                .iter()
                .filter_map(|text| match ToolPattern::parse(text) {
                    Ok(pattern) => Some(pattern),
                    Err(error) => {
                        log::warn!("Agent session scope pattern `{text}` is left out: {error}");
                        None
                    }
                })
                .collect()
        };
        let allow = tools
            .and_then(|tools| tools.get("allow"))
            .and_then(Value::as_array)
            .map(|lists| lists.iter().map(|list| patterns(Some(list))).collect())
            .unwrap_or_default();
        Ok(Self {
            role: value
                .get("role")
                .and_then(Value::as_str)
                .map(str::to_string),
            tools: AgentToolScope {
                allow,
                deny: patterns(tools.and_then(|tools| tools.get("deny"))),
            },
            plugins_off: strings(value.get("pluginsOff")).into_iter().collect(),
            permission_mode_cap: value
                .get("permissionModeCap")
                .and_then(|cap| serde_json::from_value(cap.clone()).ok()),
        })
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
