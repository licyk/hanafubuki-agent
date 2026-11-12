//! `/plugins` onto the facade: each change, then everything read again, so
//! the browser never shows a list older than its own change. The changes
//! are the settings catalog's (the `packages`, `marketplaces`, `plugins`
//! and `hooks` collections' actions, and the options' writes by key), but
//! two the catalog does not hold: the grant of what a code plugin asks for,
//! a consent given for what the browser showed, and the raw JSON settings
//! of a code plugin whose manifest does not describe them.

use hanafubuki_agent::{
    package_setting_key, AgentHandle, AgentMarketplace, AgentPackageStatus, AgentPluginStatus,
    PackagePermissions, SettingChange, SettingWriteOp, SettingsActionOutcome,
    SettingsActionRequest, SettingsWrite,
};
use serde_json::{json, Map, Value};

use super::{AgentBridge, AgentReply, TurnUpdate};

/// What `/plugins` asks of the Agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PluginsCall {
    Load,
    /// What to install, as the catalog names it: `PLUGIN@MARKETPLACE` of
    /// an added marketplace, or a directory.
    Install(String),
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
    /// A code package's settings, an object, or `null` to remove them,
    /// for a package whose manifest does not describe them.
    SetSettings {
        id: String,
        settings: serde_json::Value,
    },
    /// The settings a code package's options form changed, each `null` to
    /// return to its default.
    SetPackageOptions {
        id: String,
        options: serde_json::Map<String, serde_json::Value>,
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
    /// The options a built-in plugin's form changed
    /// (`[pluginOptions.<name>]`), each `null` to return to its default.
    SetBuiltinOptions {
        name: String,
        options: serde_json::Map<String, serde_json::Value>,
    },
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
    /// An install or an update: the package, and the hooks it brought to
    /// trust (key and command).
    Installed {
        package: Box<AgentPackageStatus>,
        hooks: Vec<(String, String)>,
    },
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
            let outcome = action(agent, "packages", Some(source), "add", json!({})).await?;
            installed(agent, outcome.record.unwrap_or_default()).await
        }
        PluginsCall::Update(id) => {
            action(agent, "packages", Some(id.clone()), "update", json!({})).await?;
            installed(agent, id).await
        }
        PluginsCall::Uninstall { id, purge_data } => {
            let verb = if purge_data { "purge" } else { "delete" };
            action(agent, "packages", Some(id.clone()), verb, json!({})).await?;
            if purge_data {
                notice(format!("Uninstalled {id} and deleted its data."))
            } else {
                notice(format!("Uninstalled {id}."))
            }
        }
        PluginsCall::SetEnabled { id, enabled } => {
            let values = json!({ "enabled": enabled });
            action(agent, "packages", Some(id.clone()), "save", values).await?;
            notice(format!("{id} is {}.", on_or_off(enabled)))
        }
        PluginsCall::Retry(id) => {
            let outcome = action(agent, "packages", Some(id.clone()), "retry", json!({})).await?;
            notice(format!(
                "{id} was {}.",
                outcome.message.as_deref().unwrap_or("loaded again")
            ))
        }
        PluginsCall::TrustHooks(keys) => {
            let count = keys.len();
            for key in keys {
                action(agent, "hooks", Some(key), "trust", json!({})).await?;
            }
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
        PluginsCall::SetPackageOptions { id, options } => {
            let changes = changes(options, |key| package_setting_key(&id, key));
            agent
                .write_setting_values(SettingsWrite { changes })
                .await?;
            notice(format!("Saved the settings of {id}."))
        }
        PluginsCall::AddMarketplace(source) => {
            let values = json!({ "source": source });
            let outcome = action(agent, "marketplaces", None, "add", values).await?;
            let name = outcome.record.unwrap_or_default();
            if outcome.message.is_some() {
                notice(format!("Marketplace {name} is already added."))
            } else {
                notice(format!("Added marketplace {name}."))
            }
        }
        PluginsCall::UpgradeMarketplace(name) => {
            let outcome = action(
                agent,
                "marketplaces",
                Some(name.clone()),
                "upgrade",
                json!({}),
            )
            .await?;
            if outcome.message.as_deref() == Some("upgraded") {
                notice(format!("Upgraded marketplace {name}."))
            } else {
                notice(format!("Marketplace {name} is up to date."))
            }
        }
        PluginsCall::RemoveMarketplace(name) => {
            action(
                agent,
                "marketplaces",
                Some(name.clone()),
                "delete",
                json!({}),
            )
            .await?;
            notice(format!("Removed marketplace {name}."))
        }
        PluginsCall::SetBuiltinEnabled { name, enabled } => {
            let values = json!({ "enabled": enabled });
            action(agent, "plugins", Some(name.clone()), "save", values).await?;
            notice(format!(
                "The built-in plugin {name} is {}.",
                on_or_off(enabled)
            ))
        }
        PluginsCall::SetBuiltinOptions { name, options } => {
            let changes = changes(options, |key| format!("pluginOptions.{name}.{key}"));
            agent
                .write_setting_values(SettingsWrite { changes })
                .await?;
            notice(format!("Saved the options of the built-in plugin {name}."))
        }
        PluginsCall::RetryBuiltin(name) => {
            action(agent, "plugins", Some(name.clone()), "retry", json!({})).await?;
            notice(format!("The built-in plugin {name} was started again."))
        }
    }
}

/// Runs `action` of the catalog's collection `collection`, on `record`.
async fn action(
    agent: &AgentHandle,
    collection: &str,
    record: Option<String>,
    action: &str,
    values: Value,
) -> Result<SettingsActionOutcome, String> {
    let request = SettingsActionRequest {
        collection: collection.to_string(),
        record,
        action: action.to_string(),
        expected_revision: None,
        values: match values {
            Value::Object(values) => values,
            _ => Map::new(),
        },
    };
    agent.run_settings_action(request).await
}

/// The options a form changed as a write by key: a value set, `null` back
/// to its default.
fn changes(options: Map<String, Value>, key: impl Fn(&str) -> String) -> Vec<SettingChange> {
    options
        .into_iter()
        .map(|(option, value)| SettingChange {
            key: key(&option),
            op: match value {
                Value::Null => SettingWriteOp::Reset,
                value => SettingWriteOp::Set(value),
            },
        })
        .collect()
}

/// The package `id` installed or updated, with its hooks that wait to be
/// trusted and what each runs (its key, when the hooks cannot be read).
async fn installed(agent: &AgentHandle, id: String) -> Result<Option<PluginsDone>, String> {
    let package = agent
        .package(&id)
        .await?
        .ok_or_else(|| format!("plugin `{id}` is not installed"))?;
    let listed = agent.hooks_list().await.map(|hooks| hooks.hooks);
    let hooks = package
        .hooks_to_trust
        .iter()
        .map(|key| {
            let command = listed
                .iter()
                .flatten()
                .find(|hook| hook.key == *key)
                .map(|hook| {
                    let hanafubuki_agent::AgentHookHandler::Command { command, .. } = &hook.handler;
                    command.clone()
                })
                .unwrap_or_else(|| key.clone());
            (key.clone(), command)
        })
        .collect();
    Ok(Some(PluginsDone::Installed {
        package: Box::new(package),
        hooks,
    }))
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
