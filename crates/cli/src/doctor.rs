//! `doctor`: whether the standalone Agent can run here. Each check prints
//! one line (`ok`, `FAIL`, or `skip`); any failure fails the command.

use hanafubuki_agent::{
    probe_keyring, AgentConfig, AgentProviderModelCatalogRequest, AgentRuntimeHost,
    ConfigLayerSource, DataDirLock, McpConnectionPhase,
};

use crate::agent::{block_on, Standalone};
use crate::cli::GlobalArgs;
use crate::exit::{Failure, FAILURE, SUCCESS};
use crate::mcp::phase;

pub(crate) fn run(global: &GlobalArgs, online: bool) -> Result<u8, Failure> {
    let mut report = Report::default();
    let standalone = match Standalone::open(global, AgentRuntimeHost::default()) {
        Ok(standalone) => standalone,
        Err(failure) => {
            report.fail("config", &failure.message);
            return Ok(report.code());
        }
    };
    let config = standalone.runtime.config();
    configuration(&mut report, config);
    let default_provider = providers(&mut report, config);
    mcp(&mut report, config);
    match probe_keyring() {
        Ok(()) => report.ok("keyring", hanafubuki_agent::KEYRING_SERVICE),
        Err(error) => report.fail("keyring", &error),
    }
    let data_dir = &standalone.runtime.storage_paths().root;
    let free = match DataDirLock::acquire(data_dir) {
        Ok(_lock) => {
            report.ok("data", &format!("{} (not in use)", data_dir.display()));
            true
        }
        Err(error) => {
            report.fail("data", &error);
            false
        }
    };
    if !online {
        report.skip(
            "online",
            "the Provider and the MCP servers are reached with --online",
        );
    } else if !free {
        report.skip("online", "the data directory is in use");
    } else {
        reach(&mut report, &standalone, default_provider);
    }
    standalone.shutdown();
    Ok(report.code())
}

fn configuration(report: &mut Report, config: &AgentConfig) {
    match config.layer_sources() {
        Ok(sources) => {
            for source in sources {
                match source {
                    ConfigLayerSource::User { path, present } => report.ok(
                        "config",
                        &format!(
                            "{}{}",
                            path.display(),
                            if present { "" } else { " (none; the defaults)" }
                        ),
                    ),
                    ConfigLayerSource::Profile { name } => {
                        report.ok("config", &format!("profile `{name}`"))
                    }
                    ConfigLayerSource::Project { path, trusted } => report.ok(
                        "config",
                        &format!(
                            "{}{}",
                            path.display(),
                            if trusted {
                                ""
                            } else {
                                " (not read: the project is not trusted)"
                            }
                        ),
                    ),
                    ConfigLayerSource::Overrides => report.ok("config", "-c overrides"),
                }
            }
        }
        Err(error) => report.fail("config", &error),
    }
}

/// The default Provider's id, when it can run a turn.
fn providers(report: &mut Report, config: &AgentConfig) -> Option<String> {
    let registry = match config.providers() {
        Ok(registry) => registry,
        Err(error) => {
            report.fail("providers", &error);
            return None;
        }
    };
    let default = registry
        .default_provider_id
        .as_deref()
        .and_then(|id| registry.providers.iter().find(|provider| provider.id == id));
    match default {
        None => {
            report.fail(
                "providers",
                &format!(
                    "{} configured, none the default (hanafubuki-agent providers default ID)",
                    registry.providers.len()
                ),
            );
            None
        }
        Some(_) if registry.model_policy.model.is_empty() => {
            report.fail(
                "providers",
                "no model is set (hanafubuki-agent providers default ID --model MODEL)",
            );
            None
        }
        Some(provider) => {
            report.ok(
                "providers",
                &format!(
                    "{} configured; the default {} ({}) runs {}",
                    registry.providers.len(),
                    provider.id,
                    provider.base_url,
                    registry.model_policy.model
                ),
            );
            Some(provider.id.clone())
        }
    }
}

fn mcp(report: &mut Report, config: &AgentConfig) {
    match config.mcp().load() {
        Ok(settings) => {
            let enabled = settings
                .servers
                .iter()
                .filter(|server| server.enabled)
                .count();
            report.ok(
                "mcp",
                &format!("{} configured, {enabled} enabled", settings.servers.len()),
            );
        }
        Err(error) => report.fail("mcp", &error.to_string()),
    }
}

/// Starts the Agent, lists the default Provider's models and connects the
/// enabled MCP servers.
fn reach(report: &mut Report, standalone: &Standalone, default_provider: Option<String>) {
    let agent = match standalone.start() {
        Ok(agent) => agent,
        Err(failure) => {
            report.fail("online", &failure.message);
            return;
        }
    };
    if let Some(provider) = default_provider {
        let listed = block_on(&agent, move |agent| async move {
            agent
                .list_provider_models(AgentProviderModelCatalogRequest {
                    provider_ids: Some(vec![provider]),
                })
                .await
        });
        match listed.map(|results| results.into_iter().next()) {
            Ok(Some(result)) => match result.error {
                None => report.ok(
                    "provider",
                    &format!(
                        "{} answers ({} models)",
                        result.provider_id,
                        result.models.len()
                    ),
                ),
                Some(error) => report.fail("provider", &format!("{}: {error}", result.provider_id)),
            },
            Ok(None) => report.fail("provider", "the default Provider was not reached"),
            Err(failure) => report.fail("provider", &failure.message),
        }
    }
    let servers = standalone.runtime.config().mcp().load();
    for server in servers.map(|settings| settings.servers).unwrap_or_default() {
        if !server.enabled {
            continue;
        }
        let id = server.id.clone();
        let connected = block_on(&agent, move |agent| async move {
            agent.mcp_connect(server.id).await
        });
        let status = connected.map(|snapshot| {
            snapshot
                .servers
                .into_iter()
                .find(|status| status.server_id == id)
        });
        match status {
            Ok(Some(status)) if status.phase == McpConnectionPhase::Ready => report.ok(
                "mcp",
                &format!("{} connected ({} tools)", id.as_str(), status.tool_count),
            ),
            Ok(Some(status)) => report.fail(
                "mcp",
                &format!(
                    "{}: {}{}",
                    id.as_str(),
                    phase(status.phase),
                    status
                        .last_error
                        .map(|error| format!(": {error}"))
                        .unwrap_or_default()
                ),
            ),
            Ok(None) => report.fail("mcp", &format!("{}: no status", id.as_str())),
            Err(failure) => report.fail("mcp", &format!("{}: {}", id.as_str(), failure.message)),
        }
    }
}

#[derive(Default)]
struct Report {
    failed: bool,
}

impl Report {
    fn ok(&mut self, check: &str, detail: &str) {
        println!("ok    {check:<10}{detail}");
    }

    fn fail(&mut self, check: &str, detail: &str) {
        self.failed = true;
        println!("FAIL  {check:<10}{detail}");
    }

    fn skip(&mut self, check: &str, detail: &str) {
        println!("skip  {check:<10}{detail}");
    }

    fn code(&self) -> u8 {
        if self.failed {
            FAILURE
        } else {
            SUCCESS
        }
    }
}
