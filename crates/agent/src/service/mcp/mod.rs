use crate::{SkillAvailability, SkillAvailabilityState, SkillMcpTransport, SkillToolDependency};
use hanafubuki_agent_config::McpSettingsStore;
use hanafubuki_agent_mcp::{McpManager, McpOAuthManager};
use std::collections::BTreeSet;
use std::sync::Arc;

mod connection;
mod connection_internal;
mod contracts;
mod facade;
mod local_status;
mod oauth;
mod proxy;
mod settings;
mod snapshot;
mod startup;

pub use contracts::{
    McpApplicationSnapshot, McpCredentialPresence, McpCredentialsWriteRequest,
    McpLocalPolicySummary, McpLocalServerStatus, McpLocalStatusSnapshot, McpOAuthCallbackRequest,
    McpOAuthCancelRequest, McpOAuthLoginRequest, McpServerDeleteRequest, McpServerWriteRequest,
    McpToolSnapshot, SkillMcpDependencyStatus,
};
pub(crate) use proxy::mcp_proxy_policy;
pub(crate) use startup::McpLifetime;

#[derive(Clone)]
pub(crate) struct AgentMcpApplication {
    pub(crate) mcp: Arc<McpManager>,
    pub(crate) mcp_settings: McpSettingsStore,
    pub(crate) mcp_oauth: McpOAuthManager,
    pub(crate) mcp_configuration_gate: Arc<tokio::sync::Mutex<()>>,
    pub(crate) blocking: Arc<dyn hanafubuki_agent_foundation::blocking::BlockingOperationPort>,
    /// The proxy policy the MCP HTTP client was last built with.
    pub(crate) proxy: Arc<parking_lot::Mutex<hanafubuki_agent_mcp::McpHttpProxyPolicy>>,
    /// Cancelled at shutdown; every connect observes it.
    pub(crate) lifetime: Arc<McpLifetime>,
}

impl AgentMcpApplication {
    /// The Agent's MCP side: an HTTP client through `proxy`, the manager,
    /// the settings (loaded once, which repairs an incompatible file) and
    /// OAuth, whose tokens keep the platform secure store.
    pub(crate) fn open(
        proxy: &crate::AgentProxy,
        executor: &hanafubuki_agent_process::executor::ExecutorManager,
        workers: hanafubuki_agent_foundation::background_worker::BackgroundWorkerGroup,
        event_sink: Arc<dyn hanafubuki_agent_mcp::McpEventSink>,
        mcp_settings: McpSettingsStore,
        secret_store: Arc<dyn hanafubuki_agent_mcp::McpSecretStore>,
        blocking: Arc<dyn hanafubuki_agent_foundation::blocking::BlockingOperationPort>,
    ) -> Result<Self, String> {
        let proxy = mcp_proxy_policy(proxy);
        let http_client = hanafubuki_agent_mcp::build_mcp_http_client(proxy.clone())
            .map_err(|error| error.to_string())?;
        let mcp = Arc::new(
            McpManager::new_with_event_sink(Arc::new(executor.clone()), workers, event_sink)
                .with_http_client(http_client.clone()),
        );
        mcp_settings.load().map_err(|error| error.to_string())?;
        // User-entered credential values live in the MCP settings file.
        let mcp_oauth = McpOAuthManager::new(secret_store, http_client);
        Ok(Self {
            mcp,
            mcp_settings,
            mcp_oauth,
            mcp_configuration_gate: Arc::new(tokio::sync::Mutex::new(())),
            blocking,
            proxy: Arc::new(parking_lot::Mutex::new(proxy)),
            lifetime: Arc::new(McpLifetime::default()),
        })
    }
}

fn digest_hex(digest: [u8; 32]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn safe_mcp_transport_summary(config: &hanafubuki_agent_mcp::McpServerConfig) -> (String, String) {
    match &config.transport {
        hanafubuki_agent_mcp::McpTransportConfig::Stdio(stdio) => {
            ("stdio".to_string(), stdio.program.clone())
        }
        hanafubuki_agent_mcp::McpTransportConfig::StreamableHttp(http) => {
            let target = http.url.split_once('?').map_or_else(
                || http.url.clone(),
                |(base, _)| format!("{base}?[redacted]"),
            );
            ("streamable_http".to_string(), target)
        }
    }
}

fn dependency_transport_matches(
    dependency: &SkillToolDependency,
    server: &hanafubuki_agent_mcp::McpServerConfig,
) -> bool {
    match (dependency.transport, &server.transport) {
        (
            None | Some(SkillMcpTransport::Stdio),
            hanafubuki_agent_mcp::McpTransportConfig::Stdio(_),
        ) => dependency.url.is_none(),
        (
            None | Some(SkillMcpTransport::StreamableHttp),
            hanafubuki_agent_mcp::McpTransportConfig::StreamableHttp(http),
        ) => dependency.url.as_deref().is_none_or(|expected| {
            let Ok(expected) = url::Url::parse(expected) else {
                return false;
            };
            let Ok(actual) = url::Url::parse(&http.url) else {
                return false;
            };
            expected == actual
        }),
        // SSE is parsed so malformed manifests are diagnosed precisely, but
        // hanafubuki-agent-mcp intentionally has no SSE runtime transport.
        (Some(SkillMcpTransport::Sse), _) => false,
        _ => false,
    }
}

pub(crate) fn dependency_availability(statuses: &[SkillMcpDependencyStatus]) -> SkillAvailability {
    let unavailable = statuses
        .iter()
        .find(|status| status.state != SkillAvailabilityState::Ready);
    unavailable.map_or(
        SkillAvailability {
            state: SkillAvailabilityState::Ready,
            reason: None,
        },
        |status| SkillAvailability {
            state: status.state,
            reason: status.reason.clone(),
        },
    )
}

fn bounded_tool_description(mut description: String) -> String {
    if description.len() > 512 {
        let boundary = description
            .char_indices()
            .take_while(|(index, _)| *index <= 512)
            .map(|(index, _)| index)
            .last()
            .unwrap_or(0);
        description.truncate(boundary);
    }
    description
}

fn credential_presence(
    settings: &hanafubuki_agent_mcp::McpSettings,
    key: hanafubuki_agent_mcp::McpCredentialKey,
) -> McpCredentialPresence {
    let configured = settings.credentials.get(&key);
    McpCredentialPresence {
        key,
        present: configured.is_some(),
        generation: configured
            .map(|credential| credential.generation)
            .unwrap_or(0),
    }
}

fn credential_references(
    server: &hanafubuki_agent_mcp::McpServerConfig,
) -> BTreeSet<hanafubuki_agent_mcp::McpCredentialKey> {
    match &server.transport {
        hanafubuki_agent_mcp::McpTransportConfig::Stdio(stdio) => {
            stdio.credential_environment.values().cloned().collect()
        }
        hanafubuki_agent_mcp::McpTransportConfig::StreamableHttp(http) => {
            http.credential_headers.values().cloned().collect()
        }
    }
}

/// Store or clear one credential value, advancing its generation either way.
///
/// The generation is what live connections compare, so a cleared credential
/// must still advance it; otherwise a server would keep using the value it
/// resolved before the deletion.
fn write_credential(
    settings: &mut hanafubuki_agent_mcp::McpSettings,
    key: hanafubuki_agent_mcp::McpCredentialKey,
    value: Option<String>,
) -> Result<(), hanafubuki_agent_mcp::McpError> {
    let generation = settings
        .credentials
        .get(&key)
        .map(|credential| credential.generation)
        .unwrap_or(0)
        .checked_add(1)
        .ok_or(hanafubuki_agent_mcp::McpError::LimitExceeded)?;
    match value {
        Some(value) => {
            settings.credentials.insert(
                key,
                hanafubuki_agent_mcp::McpCredentialSettings { value, generation },
            );
        }
        None => {
            settings.credentials.remove(&key);
        }
    }
    Ok(())
}

fn require_revision(
    settings: &hanafubuki_agent_mcp::McpSettings,
    expected: u64,
) -> Result<(), String> {
    if settings.revision == expected {
        Ok(())
    } else {
        Err(hanafubuki_agent_mcp::McpError::StaleGeneration.to_string())
    }
}

impl AgentMcpApplication {
    async fn load_settings(&self) -> Result<hanafubuki_agent_mcp::McpSettings, String> {
        let repository = self.mcp_settings.clone();
        hanafubuki_agent_foundation::blocking::run_blocking_operation(
            self.blocking.as_ref(),
            "MCP settings load",
            move || repository.load(),
        )
        .await?
        .map_err(|error| error.to_string())
    }

    async fn update_settings<F>(
        &self,
        expected_revision: u64,
        update: F,
    ) -> Result<hanafubuki_agent_mcp::McpSettings, String>
    where
        F: FnOnce(
                &mut hanafubuki_agent_mcp::McpSettings,
            ) -> Result<(), hanafubuki_agent_mcp::McpError>
            + Send
            + 'static,
    {
        let repository = self.mcp_settings.clone();
        hanafubuki_agent_foundation::blocking::run_blocking_operation(
            self.blocking.as_ref(),
            "MCP settings update",
            move || repository.update(expected_revision, update),
        )
        .await?
        .map_err(|error| error.to_string())
    }
}
