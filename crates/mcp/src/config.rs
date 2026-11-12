use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{McpCredentialKey, McpError, McpServerId};

pub const MAX_MCP_SERVERS: usize = 64;
pub const MAX_MCP_ARGUMENTS: usize = 256;
pub const MAX_MCP_ENVIRONMENT_ENTRIES: usize = 256;
pub const MAX_MCP_HEADERS: usize = 128;
pub const MAX_MCP_FILTERS: usize = 512;
pub const MAX_MCP_FIELD_BYTES: usize = 16 * 1024;
pub const MAX_MCP_TIMEOUT_SECONDS: u64 = 24 * 60 * 60;
pub const MAX_MCP_CONCURRENT_CALLS: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerConfig {
    pub id: McpServerId,
    pub enabled: bool,
    pub required: bool,
    #[serde(default)]
    pub protocol_mode: McpProtocolMode,
    pub transport: McpTransportConfig,
    pub connect_timeout_seconds: u64,
    pub initialize_timeout_seconds: u64,
    pub list_timeout_seconds: u64,
    pub call_timeout_seconds: u64,
    pub shutdown_timeout_seconds: u64,
    pub maximum_concurrent_calls: usize,
    pub tool_maximum_concurrent_calls: BTreeMap<String, usize>,
    pub enabled_tools: BTreeSet<String>,
    pub disabled_tools: BTreeSet<String>,
    pub approval_policy: McpApprovalPolicy,
    pub tool_approval_policies: BTreeMap<String, McpApprovalPolicy>,
    pub exposure_policy: McpExposurePolicy,
    pub tool_exposure_policies: BTreeMap<String, McpExposurePolicy>,
}

/// What a server the user adds starts with. The settings surface fills its
/// form from this; the id, the transport's own fields and the per-tool maps
/// start empty. A server in the settings file takes these for the keys it
/// leaves out (`repair_settings`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerDefaults {
    pub enabled: bool,
    pub required: bool,
    pub protocol_mode: McpProtocolMode,
    pub connect_timeout_seconds: u64,
    pub initialize_timeout_seconds: u64,
    pub list_timeout_seconds: u64,
    pub call_timeout_seconds: u64,
    pub shutdown_timeout_seconds: u64,
    pub maximum_concurrent_calls: usize,
    pub approval_policy: McpApprovalPolicy,
    pub exposure_policy: McpExposurePolicy,
    /// The OAuth policy of a Streamable HTTP transport.
    pub oauth: McpOAuthPolicy,
}

impl Default for McpServerDefaults {
    fn default() -> Self {
        Self {
            enabled: true,
            required: false,
            protocol_mode: McpProtocolMode::default(),
            connect_timeout_seconds: 15,
            initialize_timeout_seconds: 30,
            list_timeout_seconds: 30,
            call_timeout_seconds: 120,
            shutdown_timeout_seconds: 10,
            maximum_concurrent_calls: 4,
            approval_policy: McpApprovalPolicy::default(),
            exposure_policy: McpExposurePolicy::default(),
            oauth: McpOAuthPolicy::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpProtocolMode {
    #[default]
    Legacy,
    V20260728,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpTransportConfig {
    Stdio(McpStdioConfig),
    StreamableHttp(McpStreamableHttpConfig),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpStdioConfig {
    pub program: String,
    pub argv: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub public_environment: BTreeMap<String, String>,
    pub credential_environment: BTreeMap<String, McpCredentialKey>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpStreamableHttpConfig {
    pub url: String,
    pub public_headers: BTreeMap<String, String>,
    pub credential_headers: BTreeMap<String, McpCredentialKey>,
    pub oauth: McpOAuthPolicy,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpApprovalPolicy {
    Allow,
    #[default]
    Ask,
    Deny,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpExposurePolicy {
    Direct,
    DirectModelOnly,
    #[default]
    Deferred,
    Hidden,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpOAuthPolicy {
    #[default]
    Disabled,
    Optional,
    Required,
}

impl McpServerConfig {
    pub fn validate(&self) -> Result<(), McpError> {
        // The id decodes as any string, so a file or a request can carry one
        // `McpServerId::parse` refuses.
        McpServerId::parse(self.id.as_str()).map_err(|_| {
            McpError::InvalidConfig(
                "the server id must be 1 to 128 letters, digits, `-`, `_` or `.`".into(),
            )
        })?;
        validate_timeout("connect", self.connect_timeout_seconds)?;
        validate_timeout("initialize", self.initialize_timeout_seconds)?;
        validate_timeout("list", self.list_timeout_seconds)?;
        validate_timeout("call", self.call_timeout_seconds)?;
        validate_timeout("shutdown", self.shutdown_timeout_seconds)?;
        if self.maximum_concurrent_calls == 0
            || self.maximum_concurrent_calls > MAX_MCP_CONCURRENT_CALLS
        {
            return Err(McpError::InvalidConfig(format!(
                "maximum concurrent MCP calls must be between 1 and {MAX_MCP_CONCURRENT_CALLS}"
            )));
        }
        if self.enabled_tools.len() > MAX_MCP_FILTERS || self.disabled_tools.len() > MAX_MCP_FILTERS
        {
            return Err(McpError::InvalidConfig("too many tool filters".into()));
        }
        if self
            .enabled_tools
            .iter()
            .any(|tool| self.disabled_tools.contains(tool))
        {
            return Err(McpError::InvalidConfig(
                "a tool cannot be both enabled and disabled".into(),
            ));
        }
        if self.tool_approval_policies.len() > MAX_MCP_FILTERS
            || self.tool_exposure_policies.len() > MAX_MCP_FILTERS
            || self.tool_maximum_concurrent_calls.len() > MAX_MCP_FILTERS
        {
            return Err(McpError::InvalidConfig(
                "too many per-tool MCP policy overrides".into(),
            ));
        }
        for tool in self
            .tool_approval_policies
            .keys()
            .chain(self.tool_exposure_policies.keys())
            .chain(self.tool_maximum_concurrent_calls.keys())
        {
            validate_nonempty("MCP tool policy name", tool)?;
        }
        if self
            .tool_maximum_concurrent_calls
            .values()
            .any(|value| *value == 0 || *value > MAX_MCP_CONCURRENT_CALLS)
        {
            return Err(McpError::InvalidConfig(format!(
                "per-tool maximum concurrent MCP calls must be between 1 and {MAX_MCP_CONCURRENT_CALLS}"
            )));
        }
        match &self.transport {
            McpTransportConfig::Stdio(config) => validate_stdio(config),
            McpTransportConfig::StreamableHttp(config) => validate_http(config),
        }
    }

    pub fn connect_timeout(&self) -> Duration {
        Duration::from_secs(self.connect_timeout_seconds)
    }

    pub fn initialize_timeout(&self) -> Duration {
        Duration::from_secs(self.initialize_timeout_seconds)
    }

    pub fn list_timeout(&self) -> Duration {
        Duration::from_secs(self.list_timeout_seconds)
    }

    pub fn call_timeout(&self) -> Duration {
        Duration::from_secs(self.call_timeout_seconds)
    }

    pub fn shutdown_timeout(&self) -> Duration {
        Duration::from_secs(self.shutdown_timeout_seconds)
    }

    pub fn approval_policy_for(&self, remote_name: &str) -> McpApprovalPolicy {
        self.tool_approval_policies
            .get(remote_name)
            .copied()
            .unwrap_or(self.approval_policy)
    }

    pub fn exposure_policy_for(&self, remote_name: &str) -> McpExposurePolicy {
        self.tool_exposure_policies
            .get(remote_name)
            .copied()
            .unwrap_or(self.exposure_policy)
    }
}

fn validate_stdio(config: &McpStdioConfig) -> Result<(), McpError> {
    crate::validate_stdio_program_shape(&config.program)?;
    if config.argv.len() > MAX_MCP_ARGUMENTS {
        return Err(McpError::InvalidConfig("too many stdio arguments".into()));
    }
    for argument in &config.argv {
        validate_field("stdio argument", argument)?;
    }
    if config.public_environment.len() > MAX_MCP_ENVIRONMENT_ENTRIES
        || config.credential_environment.len() > MAX_MCP_ENVIRONMENT_ENTRIES
    {
        return Err(McpError::InvalidConfig(
            "too many stdio environment entries".into(),
        ));
    }
    for (name, value) in &config.public_environment {
        validate_environment_name(name)?;
        validate_field("stdio environment value", value)?;
    }
    for name in config.credential_environment.keys() {
        validate_environment_name(name)?;
        if config.public_environment.contains_key(name) {
            return Err(McpError::InvalidConfig(
                "public and credential environment names overlap".into(),
            ));
        }
    }
    Ok(())
}

fn validate_http(config: &McpStreamableHttpConfig) -> Result<(), McpError> {
    let url = url::Url::parse(&config.url)
        .map_err(|_| McpError::InvalidConfig("HTTP URL is invalid".into()))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(McpError::InvalidConfig(
            "HTTP URL must have an HTTP(S) origin without credentials or fragment".into(),
        ));
    }
    if config.public_headers.len() > MAX_MCP_HEADERS
        || config.credential_headers.len() > MAX_MCP_HEADERS
    {
        return Err(McpError::InvalidConfig("too many HTTP headers".into()));
    }
    for (name, value) in &config.public_headers {
        validate_header_name(name)?;
        validate_field("HTTP header value", value)?;
    }
    for name in config.credential_headers.keys() {
        validate_header_name(name)?;
        if config.public_headers.contains_key(name) {
            return Err(McpError::InvalidConfig(
                "public and credential HTTP headers overlap".into(),
            ));
        }
    }
    if config.oauth != McpOAuthPolicy::Disabled
        && config
            .public_headers
            .keys()
            .chain(config.credential_headers.keys())
            .any(|name| name.eq_ignore_ascii_case("authorization"))
    {
        return Err(McpError::InvalidConfig(
            "OAuth cannot be combined with an explicit Authorization header".into(),
        ));
    }
    Ok(())
}

fn validate_timeout(label: &str, seconds: u64) -> Result<(), McpError> {
    if seconds == 0 || seconds > MAX_MCP_TIMEOUT_SECONDS {
        return Err(McpError::InvalidConfig(format!(
            "{label} timeout is outside the allowed range"
        )));
    }
    Ok(())
}

fn validate_nonempty(label: &str, value: &str) -> Result<(), McpError> {
    if value.trim().is_empty() {
        return Err(McpError::InvalidConfig(format!("{label} is empty")));
    }
    validate_field(label, value)
}

fn validate_field(label: &str, value: &str) -> Result<(), McpError> {
    if value.len() > MAX_MCP_FIELD_BYTES || value.contains('\0') {
        return Err(McpError::InvalidConfig(format!("{label} is invalid")));
    }
    Ok(())
}

fn validate_environment_name(name: &str) -> Result<(), McpError> {
    if name.is_empty()
        || name.len() > 256
        || name.contains('=')
        || name.contains('\0')
        || name.bytes().any(|byte| byte == b'\r' || byte == b'\n')
    {
        return Err(McpError::InvalidConfig(
            "environment variable name is invalid".into(),
        ));
    }
    Ok(())
}

fn validate_header_name(name: &str) -> Result<(), McpError> {
    http::HeaderName::from_bytes(name.as_bytes())
        .map(|_| ())
        .map_err(|_| McpError::InvalidConfig("HTTP header name is invalid".into()))
}
