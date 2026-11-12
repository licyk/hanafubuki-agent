//! `/hooks` onto the facade: the configured hooks, read again after every
//! change, so the browser never shows a list older than its own change;
//! the changes (turning one on or off, trusting some) are the settings
//! catalog's `hooks` actions; and the check at start for hooks that wait to
//! be trusted.

use hanafubuki_agent::{AgentError, AgentHandle, AgentHooks, HookTrust, SettingsActionRequest};
use serde_json::{json, Map, Value};

use super::{AgentBridge, AgentReply, TurnUpdate};

/// What `/hooks` asks of the Agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HooksCall {
    Load,
    /// At start: how many hooks need review; nothing is said when none
    /// does or the hooks cannot be read.
    Check,
    SetEnabled {
        key: String,
        enabled: bool,
    },
    Trust(Vec<String>),
}

/// What the Agent answered `/hooks`.
#[derive(Debug)]
pub(crate) enum HooksReply {
    Loaded(AgentHooks),
    /// How many hooks are untrusted or modified, when any is.
    NeedReview(usize),
    Failed(String),
}

impl AgentBridge {
    pub(super) fn hooks_call(&mut self, call: HooksCall) -> Option<TurnUpdate> {
        self.spawn(move |agent| async move {
            let check = call == HooksCall::Check;
            let written = match call {
                HooksCall::Load | HooksCall::Check => Ok(()),
                HooksCall::SetEnabled { key, enabled } => {
                    let values = json!({ "enabled": enabled });
                    hooks_action(&agent, key, "save", values).await
                }
                HooksCall::Trust(keys) => {
                    let mut trusted = Ok(());
                    for key in keys {
                        trusted = hooks_action(&agent, key, "trust", json!({})).await;
                        if trusted.is_err() {
                            break;
                        }
                    }
                    trusted
                }
            };
            let listed = match written {
                Ok(()) => agent.hooks_list().await,
                Err(error) => Err(AgentError::InvalidInput(error)),
            };
            let reply = match listed {
                Ok(hooks) if check => match needing_review(&hooks) {
                    0 => return Ok(None),
                    count => HooksReply::NeedReview(count),
                },
                Ok(hooks) => HooksReply::Loaded(hooks),
                Err(_) if check => return Ok(None),
                Err(AgentError::FeatureUnavailable { reason, .. }) => {
                    HooksReply::Failed(format!("hooks are unavailable: {reason}"))
                }
                Err(error) => HooksReply::Failed(error.to_string()),
            };
            Ok(Some(AgentReply::Update(Box::new(TurnUpdate::Hooks(reply)))))
        });
        None
    }
}

/// The hooks that do not run until they are trusted (again).
pub(crate) fn needing_review(hooks: &AgentHooks) -> usize {
    hooks
        .hooks
        .iter()
        .filter(|hook| hook.trust != HookTrust::Trusted)
        .count()
}

/// Runs `action` of the settings catalog's `hooks` collection on the hook
/// `key`.
async fn hooks_action(
    agent: &AgentHandle,
    key: String,
    action: &str,
    values: Value,
) -> Result<(), String> {
    let request = SettingsActionRequest {
        collection: "hooks".to_string(),
        record: Some(key),
        action: action.to_string(),
        expected_revision: None,
        values: match values {
            Value::Object(values) => values,
            _ => Map::new(),
        },
    };
    agent.run_settings_action(request).await.map(drop)
}
