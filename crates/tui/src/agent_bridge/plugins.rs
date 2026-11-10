//! `/plugins` onto the facade's calls on the installed plugins, their
//! marketplaces and the built-in rows: each change, then everything read
//! again, so the browser never shows a list older than its own change.

use hanafubuki_agent::{
    AgentHandle, AgentMarketplace, AgentPackageInstall, AgentPackageSource, AgentPackageStatus,
    AgentPluginStatus, MarketplaceAddRequest, PackagePermissions,
};

use super::{AgentBridge, AgentReply, TurnUpdate};

/// What `/plugins` asks of the Agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PluginsCall {
    Load,
    Install(AgentPackageSource),
    Uninstall {
        id: String,
        purge_data: bool,
    },
    SetEnabled {
        id: String,
        enabled: bool,
    },
    Update(String),
    Retry(String),
    TrustHooks(Vec<String>),
    /// What a code package's guest asks for, as its page showed it.
    GrantPermissions {
        id: String,
        permissions: PackagePermissions,
    },
    /// A code package's settings, an object, or `null` to remove them.
    SetSettings {
        id: String,
        settings: serde_json::Value,
    },
    AddMarketplace(String),
    UpgradeMarketplace(String),
    RemoveMarketplace(String),
    /// A built-in row on or off (`disabledPlugins`).
    SetBuiltinEnabled {
        name: String,
        enabled: bool,
    },
    RetryBuiltin(String),
}

/// Everything `/plugins` shows.
#[derive(Debug, Clone, Default)]
pub(crate) struct PluginsState {
    pub(crate) packages: Vec<AgentPackageStatus>,
    pub(crate) marketplaces: Vec<AgentMarketplace>,
    pub(crate) builtins: Vec<AgentPluginStatus>,
    /// The Agent runs in safe mode: no installed plugin is loaded.
    pub(crate) safe_mode: bool,
}

/// What a call did, for the browser to say.
#[derive(Debug)]
pub(crate) enum PluginsDone {
    Notice(String),
    /// An install or an update, with the hooks it brought to trust.
    Installed(Box<AgentPackageInstall>),
}

/// What the Agent answered `/plugins`.
#[derive(Debug)]
pub(crate) enum PluginsReply {
    Loaded {
        state: Box<PluginsState>,
        done: Option<PluginsDone>,
    },
    /// The call failed; the state as read after it, when it could be.
    Failed {
        error: String,
        state: Option<Box<PluginsState>>,
    },
}

impl AgentBridge {
    pub(super) fn plugins_call(&mut self, call: PluginsCall) -> Option<TurnUpdate> {
        let safe_mode = self.slot.status().safe_mode;
        self.spawn(move |agent| async move {
            let done = run(&agent, call).await;
            let state = load(&agent, safe_mode).await;
            let reply = match (done, state) {
                (Ok(done), Ok(state)) => PluginsReply::Loaded {
                    state: Box::new(state),
                    done,
                },
                (Err(error), state) => PluginsReply::Failed {
                    error,
                    state: state.ok().map(Box::new),
                },
                (Ok(_), Err(error)) => PluginsReply::Failed { error, state: None },
            };
            Ok(Some(AgentReply::Update(Box::new(TurnUpdate::Plugins(
                reply,
            )))))
        });
        None
    }
}

async fn run(agent: &AgentHandle, call: PluginsCall) -> Result<Option<PluginsDone>, String> {
    let notice = |text: String| Ok(Some(PluginsDone::Notice(text)));
    match call {
        PluginsCall::Load => Ok(None),
        PluginsCall::Install(source) => {
            let install = agent.install_package(source).await?;
            Ok(Some(PluginsDone::Installed(Box::new(install))))
        }
        PluginsCall::Update(id) => {
            let install = agent.update_package(&id).await?;
            Ok(Some(PluginsDone::Installed(Box::new(install))))
        }
        PluginsCall::Uninstall { id, purge_data } => {
            let removed = agent.uninstall_package(&id, purge_data).await?;
            match removed.purged_data {
                Some(_) => notice(format!("Uninstalled {id} and deleted its data.")),
                None => notice(format!("Uninstalled {id}.")),
            }
        }
        PluginsCall::SetEnabled { id, enabled } => {
            agent.set_package_enabled(&id, enabled).await?;
            notice(format!("{id} is {}.", on_or_off(enabled)))
        }
        PluginsCall::Retry(id) => {
            let package = agent.retry_package(&id).await?;
            match package.issues.len() {
                0 => notice(format!("{id} was loaded again.")),
                count => notice(format!("{id} was loaded again, with {count} issue(s).")),
            }
        }
        PluginsCall::TrustHooks(keys) => {
            let count = keys.len();
            agent
                .trust_hooks(keys)
                .await
                .map_err(|error| error.to_string())?;
            notice(format!("Trusted {count} hook(s)."))
        }
        PluginsCall::GrantPermissions { id, permissions } => {
            agent.grant_package_permissions(&id, permissions).await?;
            notice(format!("Granted {id} its permissions."))
        }
        PluginsCall::SetSettings { id, settings } => {
            agent.set_package_settings(&id, settings).await?;
            notice(format!("Saved the settings of {id}."))
        }
        PluginsCall::AddMarketplace(source) => {
            let added = agent
                .add_marketplace(MarketplaceAddRequest {
                    source,
                    ..MarketplaceAddRequest::default()
                })
                .await?;
            if added.already_added {
                notice(format!("Marketplace {} is already added.", added.name))
            } else {
                notice(format!("Added marketplace {}.", added.name))
            }
        }
        PluginsCall::UpgradeMarketplace(name) => {
            let upgrades = agent.upgrade_marketplace(Some(name.clone())).await?;
            if upgrades.iter().any(|upgrade| upgrade.upgraded) {
                notice(format!("Upgraded marketplace {name}."))
            } else {
                notice(format!("Marketplace {name} is up to date."))
            }
        }
        PluginsCall::RemoveMarketplace(name) => {
            agent.remove_marketplace(&name).await?;
            notice(format!("Removed marketplace {name}."))
        }
        PluginsCall::SetBuiltinEnabled { name, enabled } => {
            agent.set_plugin_enabled(name.clone(), enabled).await?;
            notice(format!(
                "The built-in plugin {name} is {}.",
                on_or_off(enabled)
            ))
        }
        PluginsCall::RetryBuiltin(name) => {
            agent.retry_plugin(&name).await?;
            notice(format!("The built-in plugin {name} was started again."))
        }
    }
}

async fn load(agent: &AgentHandle, safe_mode: bool) -> Result<PluginsState, String> {
    Ok(PluginsState {
        packages: agent.packages().await?,
        marketplaces: agent.marketplaces().await?,
        builtins: agent.plugin_status(),
        safe_mode,
    })
}

fn on_or_off(enabled: bool) -> &'static str {
    if enabled {
        "on"
    } else {
        "off"
    }
}
