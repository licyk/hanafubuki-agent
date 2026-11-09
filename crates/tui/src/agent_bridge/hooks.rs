//! `/hooks` onto the facade's hook calls: the configured hooks, read again
//! after every change (turning one on or off, trusting some), so the
//! browser never shows a list older than its own change; and the check at
//! start for hooks that wait to be trusted.

use hanafubuki_agent::{AgentError, AgentHooks, HookTrust};

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
                    agent.set_hook_enabled(key, enabled).await
                }
                HooksCall::Trust(keys) => agent.trust_hooks(keys).await,
            };
            let listed = match written {
                Ok(()) => agent.hooks_list().await,
                Err(error) => Err(error),
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
