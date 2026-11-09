// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/hooks/src/engine/discovery.rs` (the sources in order, the hook
// keys, the timeouts, the hash of a normalized definition and the trust
// and enablement it decides) and `codex-rs/config/src/fingerprint.rs` (the
// hash) at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: the sources are the user's and a trusted
// project's configuration directories, each with `hooks.json` and the
// `[hooks]` table of `config.toml`; there are no managed, plugin or
// environment hooks; the state is the Agent's own file; a file that cannot
// be read is a diagnostic, never a failure.

use std::fs;
use std::path::{Path, PathBuf};

use hanafubuki_agent_foundation::paths::ResolvedPath;
use serde::Serialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::config::{decode_events, CommandConfig, GroupConfig};
use crate::event::HookEventName;
use crate::matcher::HookMatcher;
use crate::state::{HookState, HookStates};

const DEFAULT_TIMEOUT_SEC: u64 = 600;
const HOOKS_JSON: &str = "hooks.json";
const CONFIG_TOML: &str = "config.toml";

/// Where hooks are configured: each directory's `hooks.json` and the
/// `[hooks]` table of its `config.toml`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HookSources {
    /// The user's configuration directory.
    pub user: PathBuf,
    /// A trusted project's configuration directory (`.hanafubuki-agent`).
    pub project: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HookSource {
    User,
    Project,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HookTrust {
    /// The user trusted this definition.
    Trusted,
    /// The user never trusted it.
    Untrusted,
    /// The user trusted an earlier definition under this key.
    Modified,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HookHandler {
    Command {
        command: String,
        timeout_sec: u64,
        status_message: Option<String>,
    },
}

/// One configured hook handler.
#[derive(Clone, Debug)]
pub struct HookEntry {
    /// `<source file>:<event label>:<group index>:<handler index>`.
    pub key: String,
    pub event: HookEventName,
    pub matcher: Option<String>,
    pub handler: HookHandler,
    pub source: HookSource,
    pub source_path: PathBuf,
    /// `sha256:<hex>` of the normalized definition.
    pub current_hash: String,
    pub trust: HookTrust,
    pub enabled: bool,
    pub(crate) compiled: Option<HookMatcher>,
}

impl HookEntry {
    /// Only an enabled hook whose definition the user trusted runs.
    pub fn runnable(&self) -> bool {
        self.enabled && self.trust == HookTrust::Trusted
    }

    pub(crate) fn matches(&self, inputs: &[&str]) -> bool {
        self.compiled
            .as_ref()
            .is_none_or(|matcher| matcher.matches(inputs))
    }
}

/// The hooks configured now, and what could not be used.
#[derive(Clone, Debug, Default)]
pub struct HookCatalog {
    pub hooks: Vec<HookEntry>,
    pub diagnostics: Vec<String>,
}

impl HookCatalog {
    /// The hooks of `event` that run for `inputs` (a tool's names, or a
    /// session start's source), in configuration order.
    pub fn runnable<'a>(
        &'a self,
        event: HookEventName,
        inputs: &'a [&'a str],
    ) -> impl Iterator<Item = &'a HookEntry> + 'a {
        self.hooks
            .iter()
            .filter(move |hook| hook.event == event && hook.runnable() && hook.matches(inputs))
    }
}

/// The hooks of `sources`, the user's first, each directory's `hooks.json`
/// before its `config.toml`.
pub(crate) fn discover(sources: &HookSources, states: &HookStates) -> HookCatalog {
    let mut catalog = HookCatalog::default();
    let directories = std::iter::once((sources.user.as_path(), HookSource::User)).chain(
        sources
            .project
            .as_deref()
            .map(|dir| (dir, HookSource::Project)),
    );
    for (directory, source) in directories {
        let tables = [
            read_hooks_json(&directory.join(HOOKS_JSON), &mut catalog.diagnostics),
            read_config_toml(&directory.join(CONFIG_TOML), &mut catalog.diagnostics),
        ];
        if tables.iter().all(Option::is_some) {
            catalog.diagnostics.push(format!(
                "hooks are configured in both {HOOKS_JSON} and {CONFIG_TOML} of {}; prefer one",
                directory.display()
            ));
        }
        for (path, table) in tables.into_iter().flatten() {
            append(&mut catalog, source, &path, &table, states);
        }
    }
    catalog
}

fn append(
    catalog: &mut HookCatalog,
    source: HookSource,
    path: &ResolvedPath,
    table: &Map<String, Value>,
    states: &HookStates,
) {
    let source_text = path.native_text();
    for (event, groups) in decode_events(table, source_text, &mut catalog.diagnostics) {
        for group in groups {
            let GroupConfig {
                index,
                matcher,
                handlers,
            } = group;
            let matcher = matcher.filter(|_| event.has_matcher());
            let compiled = match matcher.as_deref().map(HookMatcher::new).transpose() {
                Ok(compiled) => compiled,
                Err(error) => {
                    catalog.diagnostics.push(format!(
                        "{source_text}: {} group {index} has an invalid matcher and is ignored: {error}",
                        event.name()
                    ));
                    continue;
                }
            };
            for (handler_index, handler) in handlers {
                let Some(handler) = normalize(handler) else {
                    catalog.diagnostics.push(format!(
                        "{source_text}: {} group {index} hook {handler_index} has an empty command and is ignored",
                        event.name()
                    ));
                    continue;
                };
                let key = format!("{source_text}:{}:{index}:{handler_index}", event.label());
                let current_hash = hook_hash(event, matcher.as_deref(), &handler);
                let state = states.get(&key).cloned().unwrap_or_default();
                catalog.hooks.push(HookEntry {
                    trust: trust(&state, &current_hash),
                    enabled: state.enabled != Some(false),
                    key,
                    event,
                    matcher: matcher.clone(),
                    handler,
                    source,
                    source_path: path.native().to_path_buf(),
                    current_hash,
                    compiled: compiled.clone(),
                });
            }
        }
    }
}

fn normalize(handler: CommandConfig) -> Option<HookHandler> {
    let command = if cfg!(windows) {
        handler.command_windows.unwrap_or(handler.command)
    } else {
        handler.command
    };
    if command.trim().is_empty() {
        return None;
    }
    Some(HookHandler::Command {
        command,
        timeout_sec: handler.timeout_sec.unwrap_or(DEFAULT_TIMEOUT_SEC).max(1),
        status_message: handler.status_message,
    })
}

fn trust(state: &HookState, current_hash: &str) -> HookTrust {
    match state.trusted_hash.as_deref() {
        Some(trusted) if trusted == current_hash => HookTrust::Trusted,
        Some(_) => HookTrust::Modified,
        None => HookTrust::Untrusted,
    }
}

/// The hash of a hook's normalized definition (its event, matcher and
/// handler), so the same hook spelled differently, or moved between
/// `hooks.json` and `config.toml`, keeps its trust, and any change to
/// what it runs loses it.
fn hook_hash(event: HookEventName, matcher: Option<&str>, handler: &HookHandler) -> String {
    let HookHandler::Command {
        command,
        timeout_sec,
        status_message,
    } = handler;
    let identity = json!({
        "event_name": event.label(),
        "matcher": matcher,
        "hooks": [{
            "type": "command",
            "command": command,
            "timeout": timeout_sec,
            "statusMessage": status_message,
        }],
    });
    let canonical = serde_json::to_vec(&sorted(identity)).unwrap_or_default();
    let digest = Sha256::digest(canonical);
    let hex = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("sha256:{hex}")
}

/// `value` with every object's keys in order and its nulls left out,
/// whatever order the map keeps (`serde_json/preserve_order` may be on
/// anywhere in the build).
fn sorted(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries = map
                .into_iter()
                .filter(|(_, value)| !value.is_null())
                .collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key, sorted(value)))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sorted).collect()),
        other => other,
    }
}

fn read_hooks_json(
    path: &Path,
    diagnostics: &mut Vec<String>,
) -> Option<(ResolvedPath, Map<String, Value>)> {
    let (path, text) = read(path, diagnostics)?;
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(mut file)) => match file.remove("hooks") {
            Some(Value::Object(hooks)) if !hooks.is_empty() => Some((path, hooks)),
            None | Some(Value::Object(_)) => None,
            Some(_) => {
                diagnostics.push(format!(
                    "{}: `hooks` is not a table and is ignored",
                    path.native_text()
                ));
                None
            }
        },
        Ok(_) => {
            diagnostics.push(format!(
                "{} is not a JSON object and is ignored",
                path.native_text()
            ));
            None
        }
        Err(error) => {
            diagnostics.push(format!(
                "{} cannot be read and is ignored: {error}",
                path.native_text()
            ));
            None
        }
    }
}

fn read_config_toml(
    path: &Path,
    diagnostics: &mut Vec<String>,
) -> Option<(ResolvedPath, Map<String, Value>)> {
    let (path, text) = read(path, diagnostics)?;
    // A file the configuration cannot read is reported by the configuration.
    let mut table = toml::from_str::<toml::Table>(&text).ok()?;
    match serde_json::to_value(table.remove("hooks")?) {
        Ok(Value::Object(hooks)) => {
            let events = hooks.keys().any(|key| key != "state");
            events.then_some((path, hooks))
        }
        _ => {
            diagnostics.push(format!(
                "{}: [hooks] is not a table and is ignored",
                path.native_text()
            ));
            None
        }
    }
}

fn read(path: &Path, diagnostics: &mut Vec<String>) -> Option<(ResolvedPath, String)> {
    if !path.is_file() {
        return None;
    }
    let resolved = match ResolvedPath::resolve(path) {
        Ok(resolved) => resolved,
        Err(error) => {
            diagnostics.push(format!("{} cannot be resolved: {error}", path.display()));
            return None;
        }
    };
    match fs::read_to_string(resolved.system()) {
        Ok(text) => Some((resolved, text)),
        Err(error) => {
            diagnostics.push(format!(
                "{} cannot be read: {error}",
                resolved.native_text()
            ));
            None
        }
    }
}
