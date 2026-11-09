// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/hooks/src/schema.rs` (each event's input on a command's stdin),
// `codex-rs/hooks/src/events/*.rs` (how the runs of one event combine) and
// `codex-rs/core/src/tools/hook_names.rs` (the tool names a matcher also
// knows) at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: the matched hooks run one after another,
// and a PreToolUse denial ends the event; the input has no transcript,
// model or permission mode.

use std::path::{Path, PathBuf};
use std::time::Duration;

use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_process::executor::ExecutorManager;
use serde_json::{json, Value};

use crate::command::run_command;
use crate::discovery::{discover, HookCatalog, HookHandler, HookSources};
use crate::event::HookEventName;
use crate::output::{interpret, Effect};
use crate::state::HookStateStore;

/// The source a session start reports.
pub const SESSION_START_SOURCE: &str = "startup";

/// One event, with what its hooks are told.
#[derive(Clone, Debug, PartialEq)]
pub enum HookEvent {
    PreToolUse {
        tool_name: String,
        tool_input: Value,
        tool_use_id: String,
    },
    PostToolUse {
        tool_name: String,
        tool_input: Value,
        tool_response: Value,
        tool_use_id: String,
    },
    UserPromptSubmit {
        prompt: String,
    },
    SessionStart,
    Stop {
        /// Whether a Stop hook already kept this turn going.
        stop_hook_active: bool,
        last_assistant_message: String,
    },
}

impl HookEvent {
    pub fn name(&self) -> HookEventName {
        match self {
            Self::PreToolUse { .. } => HookEventName::PreToolUse,
            Self::PostToolUse { .. } => HookEventName::PostToolUse,
            Self::UserPromptSubmit { .. } => HookEventName::UserPromptSubmit,
            Self::SessionStart => HookEventName::SessionStart,
            Self::Stop { .. } => HookEventName::Stop,
        }
    }

    /// What a group's matcher is matched against.
    pub fn matcher_inputs(&self) -> Vec<&str> {
        match self {
            Self::PreToolUse { tool_name, .. } | Self::PostToolUse { tool_name, .. } => {
                tool_matcher_inputs(tool_name)
            }
            Self::SessionStart => vec![SESSION_START_SOURCE],
            Self::UserPromptSubmit { .. } | Self::Stop { .. } => Vec::new(),
        }
    }
}

/// A tool's own name, then the names hooks written for other agents use
/// for it: what a PreToolUse or PostToolUse matcher is matched against.
pub fn tool_matcher_inputs(tool_name: &str) -> Vec<&str> {
    let aliases: &[&str] = match tool_name {
        "exec_command" => &["Bash"],
        "apply_patch" => &["Write", "Edit"],
        _ => &[],
    };
    std::iter::once(tool_name)
        .chain(aliases.iter().copied())
        .collect()
}

/// Who an event is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HookScope {
    pub session_id: String,
    pub turn_id: Option<String>,
    pub cwd: PathBuf,
}

/// What an event's hooks asked for together.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HookOutcome {
    /// The first denial's reason (PreToolUse).
    pub deny: Option<String>,
    /// Context for the model, in run order.
    pub context: Vec<String>,
    /// Prompts the turn goes on with (Stop).
    pub continue_with: Vec<String>,
    /// A Stop hook ended the turn.
    pub stop: bool,
    /// How each run ended that had no effect it asked for: `(key, why)`.
    pub failures: Vec<(String, String)>,
}

/// The hooks of one Agent: where they are configured, the state of their
/// trust and enablement, and the executor their commands run on.
pub struct HooksEngine {
    sources: HookSources,
    state: HookStateStore,
    executor: ExecutorManager,
}

impl HooksEngine {
    pub fn new(sources: HookSources, state_file: PathBuf, executor: ExecutorManager) -> Self {
        Self {
            sources,
            state: HookStateStore::new(state_file),
            executor,
        }
    }

    /// The hooks configured now, with their trust and enablement. Reads
    /// the files; blocking.
    pub fn discover(&self) -> HookCatalog {
        discover(&self.sources, &self.state.load())
    }

    /// Turns the hook `key` on or off.
    pub fn set_enabled(&self, key: &str, enabled: bool) -> Result<(), String> {
        self.known(key)?;
        self.state.update(|states| {
            states.entry(key.to_string()).or_default().enabled = (!enabled).then_some(false);
        })
    }

    /// Trusts each hook of `keys` as it is defined now; a later change to
    /// its definition takes the trust back.
    pub fn trust(&self, keys: &[String]) -> Result<(), String> {
        let catalog = self.discover();
        let hashes = keys
            .iter()
            .map(|key| {
                catalog
                    .hooks
                    .iter()
                    .find(|hook| &hook.key == key)
                    .map(|hook| (key.clone(), hook.current_hash.clone()))
                    .ok_or_else(|| format!("no hook is configured as `{key}`"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.state.update(|states| {
            for (key, hash) in hashes {
                states.entry(key).or_default().trusted_hash = Some(hash);
            }
        })
    }

    /// Runs `event`'s hooks of `catalog` that run for it, one after
    /// another, until one denies a call or stops the turn. Blocking.
    pub fn run(
        &self,
        catalog: &HookCatalog,
        event: &HookEvent,
        scope: &HookScope,
        cancellation: &CancellationToken,
    ) -> HookOutcome {
        let inputs = event.matcher_inputs();
        let input = command_input(event, scope).to_string();
        let mut outcome = HookOutcome::default();
        for hook in catalog.runnable(event.name(), &inputs) {
            if cancellation.is_cancelled() {
                break;
            }
            let HookHandler::Command {
                command,
                timeout_sec,
                ..
            } = &hook.handler;
            let run = run_command(
                &self.executor,
                command,
                Duration::from_secs(*timeout_sec),
                &input,
                &scope.cwd,
                cancellation,
            );
            match interpret(hook.event, &run) {
                Ok(Effect::None) => {}
                Ok(Effect::Deny(reason)) => {
                    outcome.deny = Some(reason);
                    break;
                }
                Ok(Effect::Context(context)) => outcome.context.push(context),
                Ok(Effect::Continue(prompt)) => outcome.continue_with.push(prompt),
                Ok(Effect::Stop) => {
                    outcome.stop = true;
                    break;
                }
                Err(why) => {
                    log::warn!("hook {} had no effect: {why}", hook.key);
                    outcome.failures.push((hook.key.clone(), why));
                }
            }
        }
        outcome
    }

    fn known(&self, key: &str) -> Result<(), String> {
        if self.discover().hooks.iter().any(|hook| hook.key == key) {
            Ok(())
        } else {
            Err(format!("no hook is configured as `{key}`"))
        }
    }
}

/// The JSON a hook's command reads on its stdin.
fn command_input(event: &HookEvent, scope: &HookScope) -> Value {
    let mut input = json!({
        "session_id": scope.session_id,
        "turn_id": scope.turn_id,
        "transcript_path": null,
        "cwd": native(&scope.cwd),
        "hook_event_name": event.name().name(),
    });
    let fields = match event {
        HookEvent::PreToolUse {
            tool_name,
            tool_input,
            tool_use_id,
        } => json!({
            "tool_name": tool_name,
            "tool_input": tool_input,
            "tool_use_id": tool_use_id,
        }),
        HookEvent::PostToolUse {
            tool_name,
            tool_input,
            tool_response,
            tool_use_id,
        } => json!({
            "tool_name": tool_name,
            "tool_input": tool_input,
            "tool_response": tool_response,
            "tool_use_id": tool_use_id,
        }),
        HookEvent::UserPromptSubmit { prompt } => json!({ "prompt": prompt }),
        HookEvent::SessionStart => json!({ "source": SESSION_START_SOURCE }),
        HookEvent::Stop {
            stop_hook_active,
            last_assistant_message,
        } => json!({
            "stop_hook_active": stop_hook_active,
            "last_assistant_message": last_assistant_message,
        }),
    };
    if let (Value::Object(input), Value::Object(fields)) = (&mut input, fields) {
        input.extend(fields);
    }
    input
}

fn native(path: &Path) -> String {
    path.display().to_string()
}
