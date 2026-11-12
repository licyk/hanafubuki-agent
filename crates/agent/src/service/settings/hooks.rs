//! The hooks of the user's and a trusted project's configuration and of
//! the installed plugins: each one turned on or off, and trusted as it is
//! defined now. Its definition (event, matcher, command, timeout, status
//! message, where it is written) is its label and summary: it is written
//! in its file, never here. Read through the `hooks` row, so while it is
//! not running the collection says why and lists none.

use std::collections::BTreeMap;

use hanafubuki_agent_config::{SettingKind, SettingSpec, SettingValue, SettingsGroup};

use super::{
    name_of, SettingsAction, SettingsActionOutcome, SettingsActionRequest, SettingsCollection,
    SettingsRecord, SAVE,
};
use crate::service::{AgentApplicationService, AgentHook, AgentHookHandler};
use hanafubuki_agent_hooks::HookSource;

pub(super) const ID: &str = "hooks";
const TRUST: &str = "trust";
const ENABLED: &str = "enabled";

fn fields() -> Vec<SettingSpec> {
    vec![SettingSpec::new(ENABLED, "Enabled", SettingKind::Boolean)]
}

pub(super) async fn collection(service: &AgentApplicationService) -> SettingsCollection {
    let collection = SettingsCollection::new(ID, "Hooks", SettingsGroup::Tools, fields())
        .with_actions(vec![
            SettingsAction::record(SAVE, "Save"),
            SettingsAction::record(TRUST, "Trust").confirmed(),
        ]);
    let hooks = match service.hooks_list().await {
        Ok(hooks) => hooks,
        Err(error) => return collection.unavailable(error),
    };
    let records = hooks
        .hooks
        .iter()
        .map(|hook| {
            let event = name_of(&hook.event);
            SettingsRecord {
                id: hook.key.clone(),
                label: match &hook.matcher {
                    Some(matcher) => format!("{event} · {matcher}"),
                    None => event,
                },
                summary: definition(hook),
                state: Some(name_of(&hook.trust)),
                issue: None,
                read_only: None,
                values: BTreeMap::from([(
                    ENABLED.to_string(),
                    SettingValue::Boolean(hook.enabled),
                )]),
                secrets_set: Vec::new(),
            }
        })
        .collect();
    SettingsCollection {
        records,
        issues: hooks.diagnostics,
        ..collection
    }
}

pub(super) async fn run(
    service: &AgentApplicationService,
    request: SettingsActionRequest,
) -> Result<SettingsActionOutcome, String> {
    let key = request.record_id()?.to_string();
    match request.action.as_str() {
        SAVE => {
            for (field, value) in &request.values {
                let enabled = match (field.as_str(), value.as_bool()) {
                    (ENABLED, Some(enabled)) => enabled,
                    (ENABLED, None) => return Err("`enabled` is true or false".to_string()),
                    _ => return Err(format!("`{field}` is not a field of this record")),
                };
                service
                    .set_hook_enabled(key.clone(), enabled)
                    .await
                    .map_err(|error| error.to_string())?;
            }
            Ok(SettingsActionOutcome::default())
        }
        TRUST => {
            service
                .trust_hooks(vec![key])
                .await
                .map_err(|error| error.to_string())?;
            Ok(SettingsActionOutcome::default())
        }
        _ => request.unknown_action(),
    }
}

/// What the hook runs, how long it may take, what it shows while it runs
/// and where it is written.
fn definition(hook: &AgentHook) -> String {
    let AgentHookHandler::Command {
        command,
        timeout_sec,
        status_message,
    } = &hook.handler;
    let mut parts = vec![command.clone(), format!("times out after {timeout_sec}s")];
    if let Some(message) = status_message {
        parts.push(format!("shows \"{message}\""));
    }
    parts.push(match &hook.source {
        HookSource::Plugin { id, .. } => format!("plugin {id}"),
        HookSource::User | HookSource::Project => {
            format!("{} {}", name_of(&hook.source), hook.source_path.display())
        }
    });
    parts.join(" · ")
}
