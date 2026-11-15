//! Definition edits only target the user's files; trust is recomputed on discovery.
use super::record::merged;
use super::{SettingsActionOutcome, SettingsActionRequest, ADD, DELETE};
use crate::service::{AgentApplicationService, AgentHook, AgentHookHandler};
use hanafubuki_agent_config::{SettingChoice, SettingKind, SettingSpec};
use hanafubuki_agent_hooks::{HookEventName, HookSource};
use serde_json::{json, Value};

pub(super) fn fields() -> Vec<SettingSpec> {
    let text = |nullable| SettingKind::Text {
        maximum_chars: None,
        secret: false,
        nullable,
    };
    vec![
        SettingSpec::new(
            "event",
            "Event",
            SettingKind::Choice {
                values: HookEventName::ALL
                    .iter()
                    .map(|event| SettingChoice::new(event.name(), event.name()))
                    .collect(),
            },
        ),
        SettingSpec::new("matcher", "Matcher", text(true)),
        SettingSpec::new("command", "Command", text(false)),
        SettingSpec::new(
            "timeout",
            "Timeout (seconds)",
            SettingKind::Integer {
                minimum: Some(1),
                maximum: None,
            },
        ),
        SettingSpec::new("statusMessage", "Status message", text(true)),
    ]
}

pub(super) fn values(hook: &AgentHook) -> Value {
    let AgentHookHandler::Command {
        command,
        timeout_sec,
        status_message,
    } = &hook.handler;
    json!({"event": hook.event.name(), "matcher": hook.matcher, "command": command, "timeout": timeout_sec, "statusMessage": status_message})
}

pub(super) async fn run(
    service: &AgentApplicationService,
    request: &SettingsActionRequest,
    hook: Option<&AgentHook>,
) -> Result<SettingsActionOutcome, String> {
    if hook.is_some_and(|hook| hook.source != HookSource::User) {
        return Err("Only user-owned hook definitions can be edited here".into());
    }
    let enabled = match request.values.get("enabled") {
        Some(value) => value.as_bool().ok_or("enabled must be true or false")?,
        None => hook.is_none_or(|hook| hook.enabled),
    };
    let current = hook
        .map(values)
        .unwrap_or_else(|| json!({"event": "PreToolUse", "command": "", "timeout": 600, "matcher": null, "statusMessage": null}));
    let edits = request
        .values
        .iter()
        .filter(|(key, _)| key.as_str() != "enabled")
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let values: Value = merged(&current, &fields(), &edits)?;
    let previous = hook
        .map(|hook| {
            let mut pieces = hook.key.rsplit(':');
            let handler = pieces
                .next()
                .and_then(|s| s.parse::<usize>().ok())
                .ok_or("Invalid hook identity")?;
            let group = pieces
                .next()
                .and_then(|s| s.parse::<usize>().ok())
                .ok_or("Invalid hook identity")?;
            let file = hook
                .source_path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or("Invalid hook source")?
                .to_string();
            Ok::<_, String>((file, hook.event.name().to_string(), group, handler))
        })
        .transpose()?;
    let replacement = if request.action == DELETE {
        None
    } else {
        let event = values["event"]
            .as_str()
            .ok_or("Select a hook event")?
            .to_string();
        if values["command"]
            .as_str()
            .is_none_or(|command| command.trim().is_empty())
        {
            return Err("Enter a command".into());
        }
        Some((event, {
            let mut handler = json!({"type": "command", "command": values["command"], "timeout": values["timeout"]});
            if !values["statusMessage"].is_null() {
                handler["statusMessage"] = values["statusMessage"].clone();
            }
            let mut group = json!({"hooks": [handler]});
            if !values["matcher"].is_null() {
                group["matcher"] = values["matcher"].clone();
            }
            group
        }))
    };
    let config = service.config.clone();
    hanafubuki_agent_foundation::blocking::run_blocking_operation(
        service.blocking.as_ref(),
        "write user hook definition",
        move || {
            let file = previous
                .as_ref()
                .map(|(file, _, _, _)| file.as_str())
                .unwrap_or("hooks.json");
            config.write_user_hook(
                file,
                previous
                    .as_ref()
                    .map(|(_, event, group, handler)| (event.as_str(), *group, *handler)),
                replacement
                    .as_ref()
                    .map(|(event, group)| (event.as_str(), group.clone())),
            )
        },
    )
    .await??;
    // Listing refreshes the runtime catalog, including definition hashes.
    let catalog = service
        .hooks_list()
        .await
        .map_err(|error| error.to_string())?;
    if request.action != DELETE {
        if let Some(saved) = catalog
            .hooks
            .iter()
            .rev()
            .find(|hook| hook.source == HookSource::User && self::values(hook) == values)
        {
            service
                .set_hook_enabled(saved.key.clone(), enabled)
                .await
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(SettingsActionOutcome {
        record: None,
        message: Some(
            if request.action == DELETE {
                "Hook removed"
            } else if request.action == ADD {
                "Hook added; review and trust it before it runs"
            } else {
                "Hook definition saved; review and trust it before it runs"
            }
            .into(),
        ),
    })
}
