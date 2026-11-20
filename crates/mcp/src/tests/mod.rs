use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use hanafubuki_agent_foundation::background_worker::BackgroundWorkerGroup;
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_process::executor::manager::executor_registry_counts;
use hanafubuki_agent_process::executor::ExecutorManager;
use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{
    CallToolResult, ContentBlock as Content, ProtocolVersion, ResourceContents, ServerCapabilities,
    ServerConfig, Tool, ToolAnnotations,
};
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use rmcp::{tool, tool_handler, tool_router, ServerHandler};
use serde_json::{json, Map};
use tempfile::tempdir;
use tokio_util::sync::CancellationToken as TokioCancellationToken;

use super::*;

/// The environment variable naming the Python the stdio fixtures run on.
const PYTHON_ENVIRONMENT: &str = "HANAFUBUKI_AGENT_TEST_PYTHON";

/// The Python a stdio fixture runs on: what [`PYTHON_ENVIRONMENT`] names,
/// else `python` on Windows and `python3` elsewhere.
fn python_program() -> &'static str {
    static PYTHON: OnceLock<String> = OnceLock::new();
    PYTHON.get_or_init(|| {
        std::env::var(PYTHON_ENVIRONMENT)
            .ok()
            .map(|program| program.trim().to_string())
            .filter(|program| !program.is_empty())
            .unwrap_or_else(|| match cfg!(windows) {
                true => "python".to_string(),
                false => "python3".to_string(),
            })
    })
}

/// Whether that Python runs here. A fixture that needs one returns when it
/// does not, saying so once, rather than failing: a machine without an
/// interpreter says nothing about the code under test.
///
/// It is asked the way a fixture's server is: with the few variables a
/// server inherits, not this process's whole environment. A shim that only
/// finds its interpreter through a variable of the test process fails here,
/// as it would once spawned.
fn python_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        let program = python_program();
        let mut probe = std::process::Command::new(program);
        probe
            .args(["-c", "import sys"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .env_clear();
        for name in [
            "PATH",
            "SYSTEMROOT",
            "SYSTEMDRIVE",
            "USERPROFILE",
            "TEMP",
            "TMP",
        ] {
            if let Some(value) = std::env::var_os(name) {
                probe.env(name, value);
            }
        }
        let runs = probe.status().is_ok_and(|status| status.success());
        if !runs {
            eprintln!(
                "warning: the stdio MCP fixtures are skipped: `{program}` is not a Python that \
                 runs here; name one with {PYTHON_ENVIRONMENT}"
            );
        }
        runs
    })
}

fn stdio_config() -> McpServerConfig {
    McpServerConfig {
        id: McpServerId::parse("local").unwrap(),
        enabled: true,
        required: false,
        protocol_mode: McpProtocolMode::Legacy,
        transport: McpTransportConfig::Stdio(McpStdioConfig {
            environment_variables: Vec::new(),
            program: "fake-mcp".into(),
            argv: vec!["--stdio".into()],
            cwd: None,
            public_environment: BTreeMap::from([("LANG".into(), "C".into())]),
            credential_environment: BTreeMap::from([(
                "TOKEN".into(),
                McpCredentialKey::parse("mcp-local-token").unwrap(),
            )]),
        }),
        connect_timeout_seconds: 10,
        initialize_timeout_seconds: 10,
        list_timeout_seconds: 10,
        call_timeout_seconds: 30,
        shutdown_timeout_seconds: 10,
        maximum_concurrent_calls: 1,
        tool_maximum_concurrent_calls: BTreeMap::new(),
        enabled_tools: BTreeSet::new(),
        disabled_tools: BTreeSet::new(),
        approval_policy: McpApprovalPolicy::Ask,
        tool_approval_policies: BTreeMap::new(),
        exposure_policy: McpExposurePolicy::Deferred,
        tool_exposure_policies: BTreeMap::new(),
    }
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct HttpEchoRequest {
    value: String,
}

#[derive(Debug, Clone)]
struct HttpFixtureServer {
    tool_router: ToolRouter<Self>,
    legacy_only: bool,
}

impl HttpFixtureServer {
    fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
            legacy_only: false,
        }
    }

    fn legacy() -> Self {
        Self {
            tool_router: Self::tool_router(),
            legacy_only: true,
        }
    }
}

#[tool_router]
impl HttpFixtureServer {
    #[tool(description = "Echo a value over Streamable HTTP")]
    fn echo(&self, Parameters(request): Parameters<HttpEchoRequest>) -> String {
        request.value
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for HttpFixtureServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        if self.legacy_only {
            Cow::Owned(vec![ProtocolVersion::V_2025_06_18])
        } else {
            Cow::Borrowed(ProtocolVersion::KNOWN_VERSIONS)
        }
    }

    fn discover(
        &self,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> impl std::future::Future<Output = Result<rmcp::model::DiscoverResult, rmcp::model::ErrorData>>
           + '_ {
        let result = if self.legacy_only {
            Err(rmcp::model::ErrorData::method_not_found::<
                rmcp::model::DiscoverRequestMethod,
            >())
        } else {
            Ok(rmcp::model::DiscoverResult::from_server_info(
                self.supported_protocol_versions().into_owned(),
                self.get_info(),
            ))
        };
        std::future::ready(result)
    }
}

#[derive(Debug, Clone)]
struct OversizedHttpFixtureServer {
    tool_router: ToolRouter<Self>,
}

impl OversizedHttpFixtureServer {
    fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_router]
impl OversizedHttpFixtureServer {
    #[tool(description = "Return one intentionally oversized SSE message")]
    fn oversized(&self) -> String {
        "s".repeat(crate::client_adapter::MAXIMUM_MCP_MESSAGE_BYTES + 1)
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for OversizedHttpFixtureServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }
}

mod client_adapter;
mod config;
mod credentials;
mod discovery;
mod http;
mod manager_lifecycle;
mod output;
mod producer_quirks;
mod reconnect;
mod stdio;
mod stdio_environment;
