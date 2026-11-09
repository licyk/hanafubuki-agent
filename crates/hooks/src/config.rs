// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/protocol/src/hooks.rs` (the events table, its matcher groups
// and handler kinds) and `codex-rs/config/src/hook_config.rs` (`hooks.json`)
// at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: the table is read tolerantly, group by
// group and handler by handler, so one bad entry is skipped with a
// diagnostic and the others still load; unknown fields are ignored; the
// prompt, agent and MCP handlers and async commands are reported as
// unsupported.

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::event::{unraised_event_reason, HookEventName};

/// One matcher group of an event, at its index in the event's list.
pub(crate) struct GroupConfig {
    pub(crate) index: usize,
    pub(crate) matcher: Option<String>,
    /// Each command handler at its index in the group's `hooks`.
    pub(crate) handlers: Vec<(usize, CommandConfig)>,
}

#[derive(Deserialize)]
pub(crate) struct CommandConfig {
    pub(crate) command: String,
    #[serde(default, rename = "commandWindows", alias = "command_windows")]
    pub(crate) command_windows: Option<String>,
    #[serde(default, rename = "timeout")]
    pub(crate) timeout_sec: Option<u64>,
    #[serde(default, rename = "async")]
    pub(crate) run_async: bool,
    #[serde(default, rename = "statusMessage")]
    pub(crate) status_message: Option<String>,
}

/// The events of one `hooks` table (`hooks.json`'s `hooks`, or
/// `config.toml`'s `[hooks]`), in the Agent's event order. What cannot be
/// used is skipped with a diagnostic naming `source`.
pub(crate) fn decode_events(
    table: &Map<String, Value>,
    source: &str,
    diagnostics: &mut Vec<String>,
) -> Vec<(HookEventName, Vec<GroupConfig>)> {
    for name in table.keys() {
        // Codex keeps its trust state under `[hooks.state]`; the Agent keeps
        // its own in the data directory.
        if name == "state" || HookEventName::parse(name).is_some() {
            continue;
        }
        match unraised_event_reason(name) {
            Some(reason) => diagnostics.push(format!("{source}: {name} hooks never run: {reason}")),
            None => diagnostics.push(format!("{source}: unknown hook event `{name}` is ignored")),
        }
    }
    HookEventName::ALL
        .into_iter()
        .filter_map(|event| {
            let groups = table.get(event.name())?;
            let Some(groups) = groups.as_array() else {
                diagnostics.push(format!(
                    "{source}: {} is not a list of matcher groups and is ignored",
                    event.name()
                ));
                return None;
            };
            let groups = groups
                .iter()
                .enumerate()
                .filter_map(|(index, group)| decode_group(event, index, group, source, diagnostics))
                .collect();
            Some((event, groups))
        })
        .collect()
}

fn decode_group(
    event: HookEventName,
    index: usize,
    group: &Value,
    source: &str,
    diagnostics: &mut Vec<String>,
) -> Option<GroupConfig> {
    let at = format!("{source}: {} group {index}", event.name());
    let Some(group) = group.as_object() else {
        diagnostics.push(format!("{at} is not a table and is ignored"));
        return None;
    };
    let matcher = match group.get("matcher") {
        None | Some(Value::Null) => None,
        Some(Value::String(matcher)) => Some(matcher.clone()),
        Some(_) => {
            diagnostics.push(format!(
                "{at} has a matcher that is not a string and is ignored"
            ));
            return None;
        }
    };
    let handlers = match group.get("hooks") {
        None => Vec::new(),
        Some(Value::Array(handlers)) => handlers
            .iter()
            .enumerate()
            .filter_map(|(handler_index, handler)| {
                decode_handler(handler, &format!("{at} hook {handler_index}"), diagnostics)
                    .map(|handler| (handler_index, handler))
            })
            .collect(),
        Some(_) => {
            diagnostics.push(format!("{at}: `hooks` is not a list and is ignored"));
            return None;
        }
    };
    Some(GroupConfig {
        index,
        matcher,
        handlers,
    })
}

fn decode_handler(
    handler: &Value,
    at: &str,
    diagnostics: &mut Vec<String>,
) -> Option<CommandConfig> {
    let kind = handler.get("type").and_then(Value::as_str);
    match kind {
        Some("command") => {}
        Some(kind @ ("mcp_tool" | "prompt" | "agent")) => {
            diagnostics.push(format!(
                "{at}: `{kind}` hooks are not supported; it never runs"
            ));
            return None;
        }
        Some(kind) => {
            diagnostics.push(format!("{at}: unknown hook type `{kind}` is ignored"));
            return None;
        }
        None => {
            diagnostics.push(format!("{at} has no `type` and is ignored"));
            return None;
        }
    }
    let config = match CommandConfig::deserialize(handler) {
        Ok(config) => config,
        Err(error) => {
            diagnostics.push(format!("{at} cannot be read and is ignored: {error}"));
            return None;
        }
    };
    if config.run_async {
        diagnostics.push(format!(
            "{at}: async command hooks are not supported; it never runs"
        ));
        return None;
    }
    Some(config)
}
