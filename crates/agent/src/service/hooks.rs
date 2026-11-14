//! The hook calls: the user's and a trusted project's hooks, their trust
//! and enablement, served by the `hooks` plugin row
//! (`hanafubuki-agent-plugin-hooks`) through the feature helper.

use std::path::PathBuf;
use std::sync::Arc;

use hanafubuki_agent_config::{AgentConfig, ConfigLayerSource};
use hanafubuki_agent_hooks::{
    HookCatalog, HookEntry, HookEventName, HookHandler, HookSource, HookSources, HookTrust,
};
use hanafubuki_agent_kernel::AgentError;
use hanafubuki_agent_plugin_hooks::{HooksService, WorkingDirectory, HOOKS_PLUGIN};
use serde::Serialize;

use super::AgentApplicationService;

/// The hooks configured now, and what in their configuration could not be
/// used.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentHooks {
    pub hooks: Vec<AgentHook>,
    pub diagnostics: Vec<String>,
}

/// One configured hook. Only an enabled hook whose trust is `trusted`
/// runs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentHook {
    /// What `set_hook_enabled` and `trust_hooks` name it by.
    pub key: String,
    pub event: HookEventName,
    pub matcher: Option<String>,
    pub handler: AgentHookHandler,
    pub source: HookSource,
    /// The `hooks.json` or `config.toml` it is configured in.
    pub source_path: PathBuf,
    pub trust: HookTrust,
    pub enabled: bool,
    /// `sha256:<hex>` of its definition, which trusting it records.
    pub current_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum AgentHookHandler {
    Command {
        command: String,
        timeout_sec: u64,
        status_message: Option<String>,
    },
}

impl AgentApplicationService {
    /// Every configured hook, read again now; `FeatureUnavailable` while
    /// the `hooks` plugin is not running.
    pub async fn hooks_list(&self) -> Result<AgentHooks, AgentError> {
        self.feature(HOOKS_PLUGIN, |hooks: Arc<HooksService>| async move {
            let catalog = hooks.list().await.map_err(AgentError::InvalidInput)?;
            Ok(agent_hooks(&catalog))
        })
        .await
    }

    /// Turns the hook `key` on or off; a hook that is off never runs.
    pub async fn set_hook_enabled(&self, key: String, enabled: bool) -> Result<(), AgentError> {
        self.feature(HOOKS_PLUGIN, |hooks: Arc<HooksService>| async move {
            hooks
                .set_enabled(key, enabled)
                .await
                .map_err(AgentError::InvalidInput)
        })
        .await
    }

    /// Trusts each hook of `keys` as it is defined now; a later change to
    /// its definition takes the trust back.
    pub async fn trust_hooks(&self, keys: Vec<String>) -> Result<(), AgentError> {
        self.feature(HOOKS_PLUGIN, |hooks: Arc<HooksService>| async move {
            hooks.trust(keys).await.map_err(AgentError::InvalidInput)
        })
        .await
    }
}

/// Where hooks are configured: the user's configuration directory, and the
/// project's when its configuration is read (a trusted project with a
/// `.hanafubuki-agent/config.toml`).
pub(super) fn hook_sources(config: &AgentConfig) -> Result<HookSources, String> {
    let mut user = None;
    let mut project = None;
    for source in config.layer_sources()? {
        match source {
            ConfigLayerSource::User { path, .. } => user = path.parent().map(PathBuf::from),
            ConfigLayerSource::Project {
                path,
                trusted: true,
            } => project = path.parent().map(PathBuf::from),
            _ => {}
        }
    }
    Ok(HookSources {
        user: user.ok_or("the configuration has no directory")?,
        project,
        plugins: Vec::new(),
    })
}

/// The first workspace root, as it is when a hook runs.
pub(super) fn working_directory(
    driver: crate::session_driver::AgentSessionDriver,
) -> WorkingDirectory {
    Arc::new(move |session_id| {
        driver
            .workspace_roots(session_id)
            .map(|roots| roots.primary().clone())
            .map_err(|error| error.to_string())
    })
}

fn agent_hooks(catalog: &HookCatalog) -> AgentHooks {
    AgentHooks {
        hooks: catalog.hooks.iter().map(agent_hook).collect(),
        diagnostics: catalog.diagnostics.clone(),
    }
}

pub(super) fn agent_hook(hook: &HookEntry) -> AgentHook {
    let HookHandler::Command {
        command,
        timeout_sec,
        status_message,
    } = &hook.handler;
    AgentHook {
        key: hook.key.clone(),
        event: hook.event,
        matcher: hook.matcher.clone(),
        handler: AgentHookHandler::Command {
            command: command.clone(),
            timeout_sec: *timeout_sec,
            status_message: status_message.clone(),
        },
        source: hook.source.clone(),
        source_path: hook.source_path.clone(),
        trust: hook.trust,
        enabled: hook.enabled,
        current_hash: hook.current_hash.clone(),
    }
}
