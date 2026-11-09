//! The Agent setting values several Agent crates share: the Agent's
//! configuration persists them, and the approval crate and the model
//! transport re-export the ones in their API. These are values and the rules
//! that bound them, not Agent behaviour. A value two Agent crates persist
//! that neither may depend on the other for (the kernel's session state and
//! the memory store) is defined here too.

use http::{HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};

/// User-visible permission behavior frozen for one Agent turn.
///
/// This selects who resolves an already-classified review request. It never
/// changes a hard forbidden decision into an allowed action.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentPermissionMode {
    #[default]
    Manual,
    AutoReview,
    FullAccess,
}

/// What the operating system lets the commands the model runs do, Codex's
/// `sandbox_mode`: read anything and write nothing, also write in the
/// workspace, or no sandbox. Independent of [`AgentPermissionMode`], which
/// decides who approves a command, not what it can reach.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentSandboxMode {
    ReadOnly,
    WorkspaceWrite,
    FullAccess,
}

impl AgentSandboxMode {
    pub const ALL: &'static [Self] = &[Self::ReadOnly, Self::WorkspaceWrite, Self::FullAccess];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::WorkspaceWrite => "workspace-write",
            Self::FullAccess => "full-access",
        }
    }
}

/// Which Windows sandbox confines the commands, Codex's `[windows]
/// sandbox`: the restricted token any user can have (`unelevated`), or the
/// sandbox users an administrator sets up (`elevated`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentWindowsSandbox {
    Unelevated,
    Elevated,
}

impl AgentWindowsSandbox {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unelevated => "unelevated",
            Self::Elevated => "elevated",
        }
    }
}

/// Whether a session's history may feed memory generation. The kernel
/// persists it with the session and the memory store per source, both in
/// this snake_case form.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentMemoryContributionMode {
    Enabled,
    #[default]
    Disabled,
    /// A tool brought external content into the session; it never
    /// contributes again.
    Polluted,
}

impl AgentMemoryContributionMode {
    pub fn contributes(self) -> bool {
        self == Self::Enabled
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
            Self::Polluted => "polluted",
        }
    }
}

pub const MAX_MODEL_REASONING_EFFORT_BYTES: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ModelReasoningEffort(ModelReasoningEffortValue);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum ModelReasoningEffortValue {
    Minimal,
    Low,
    Medium,
    High,
    XHigh,
    Custom(String),
}

#[allow(non_upper_case_globals)]
impl ModelReasoningEffort {
    pub const Minimal: Self = Self(ModelReasoningEffortValue::Minimal);
    pub const Low: Self = Self(ModelReasoningEffortValue::Low);
    pub const Medium: Self = Self(ModelReasoningEffortValue::Medium);
    pub const High: Self = Self(ModelReasoningEffortValue::High);
    pub const XHigh: Self = Self(ModelReasoningEffortValue::XHigh);

    pub fn parse(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        if value.is_empty() || value.len() > MAX_MODEL_REASONING_EFFORT_BYTES {
            return Err(format!(
                "reasoning effort must contain 1 to {MAX_MODEL_REASONING_EFFORT_BYTES} bytes"
            ));
        }
        if !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(
                "reasoning effort may contain only ASCII letters, digits, '-', '_', or '.'"
                    .to_string(),
            );
        }
        if value == "none" {
            return Err(
                "reasoning effort 'none' is not supported; omit the setting for Provider default"
                    .to_string(),
            );
        }
        Ok(Self(match value.as_str() {
            "minimal" => ModelReasoningEffortValue::Minimal,
            "low" => ModelReasoningEffortValue::Low,
            "medium" => ModelReasoningEffortValue::Medium,
            "high" => ModelReasoningEffortValue::High,
            "xhigh" => ModelReasoningEffortValue::XHigh,
            _ => ModelReasoningEffortValue::Custom(value),
        }))
    }

    pub fn as_str(&self) -> &str {
        match &self.0 {
            ModelReasoningEffortValue::Minimal => "minimal",
            ModelReasoningEffortValue::Low => "low",
            ModelReasoningEffortValue::Medium => "medium",
            ModelReasoningEffortValue::High => "high",
            ModelReasoningEffortValue::XHigh => "xhigh",
            ModelReasoningEffortValue::Custom(value) => value,
        }
    }
}

impl std::fmt::Display for ModelReasoningEffort {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl std::str::FromStr for ModelReasoningEffort {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl Serialize for ModelReasoningEffort {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ModelReasoningEffort {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

/// A model Provider credential or header value: non-empty, without line
/// breaks. `label` names the value in the error.
pub fn validate_model_secret_value(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty() || value.bytes().any(|byte| byte == b'\r' || byte == b'\n') {
        return Err(format!(
            "{label} must be non-empty and contain no line breaks"
        ));
    }
    Ok(())
}

/// A custom model Provider request header, parsed as the transport sends it:
/// a valid header name the transport does not own, and a valid value.
pub fn parse_model_request_header(
    name: &str,
    value: &str,
) -> Result<(HeaderName, HeaderValue), String> {
    let name = HeaderName::from_bytes(name.as_bytes())
        .map_err(|_| "invalid model provider header name".to_string())?;
    if is_transport_owned_header(&name) {
        return Err(format!(
            "model provider header {} is owned by the transport",
            name.as_str()
        ));
    }
    validate_model_secret_value("model provider header", value)?;
    let value = HeaderValue::from_str(value)
        .map_err(|_| "invalid model provider header value".to_string())?;
    Ok((name, value))
}

fn is_transport_owned_header(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "authorization"
            | "proxy-authorization"
            | "x-api-key"
            | "anthropic-version"
            | "host"
            | "content-length"
            | "content-type"
            | "accept"
            | "connection"
            | "transfer-encoding"
    )
}
