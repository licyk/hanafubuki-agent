// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/hooks/src/engine/output_parser.rs` and the `parse_completed`
// of `codex-rs/hooks/src/events/{pre_tool_use,post_tool_use,stop,
// user_prompt_submit,session_start}.rs` (what a hook's exit code and output
// mean for each event) at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: the output is read tolerantly, field by
// field (an unknown field is ignored, a field of the wrong type is absent);
// a hook cannot rewrite a tool's input or block a prompt, and says so as
// its failure.

use serde_json::Value;

use crate::command::CommandRun;
use crate::event::HookEventName;

/// What one hook's run asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Effect {
    None,
    /// PreToolUse: the call does not run.
    Deny(String),
    /// Context for the model.
    Context(String),
    /// Stop: the turn goes on with this prompt.
    Continue(String),
    /// Stop: the turn ends without the remaining hooks.
    Stop,
}

/// An effect, or why the run had none.
pub(crate) type Interpreted = Result<Effect, String>;

pub(crate) fn interpret(event: HookEventName, run: &CommandRun) -> Interpreted {
    if let Some(error) = &run.error {
        return Err(error.clone());
    }
    match run.exit_code {
        Some(0) => interpret_stdout(event, &run.stdout),
        Some(2) => {
            let Some(reason) = trimmed(&run.stderr) else {
                return Err(format!(
                    "the {} hook exited with code 2 but wrote no reason to stderr",
                    event.name()
                ));
            };
            match event {
                HookEventName::PreToolUse => Ok(Effect::Deny(reason)),
                HookEventName::PostToolUse => Ok(Effect::Context(reason)),
                HookEventName::Stop => Ok(Effect::Continue(reason)),
                HookEventName::UserPromptSubmit | HookEventName::SessionStart => Err(format!(
                    "a {} hook cannot block (exit code 2): {reason}",
                    event.name()
                )),
            }
        }
        Some(code) => Err(format!("the hook exited with code {code}")),
        None => Err("the hook exited without a status code".to_string()),
    }
}

fn interpret_stdout(event: HookEventName, stdout: &str) -> Interpreted {
    let Some(text) = trimmed(stdout) else {
        return Ok(Effect::None);
    };
    let output = match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(output)) => output,
        _ if text.starts_with('{') || text.starts_with('[') => {
            return Err(format!("the {} hook printed invalid JSON", event.name()))
        }
        // Plain text is context where an event takes context from stdout.
        _ => {
            return Ok(match event {
                HookEventName::UserPromptSubmit | HookEventName::SessionStart => {
                    Effect::Context(text)
                }
                _ => Effect::None,
            })
        }
    };
    let field = |name: &str| output.get(name).and_then(Value::as_str).and_then(trimmed);
    let specific = |name: &str| {
        output
            .get("hookSpecificOutput")
            .and_then(|specific| specific.get(name))
            .and_then(Value::as_str)
            .and_then(trimmed)
    };
    let blocks = field("decision").as_deref() == Some("block");
    let goes_on = output.get("continue").and_then(Value::as_bool) != Some(false);
    match event {
        HookEventName::PreToolUse => {
            let denies = specific("permissionDecision").as_deref() == Some("deny");
            let reason = if denies {
                specific("permissionDecisionReason")
            } else if blocks {
                field("reason")
            } else {
                return Ok(Effect::None);
            };
            reason
                .map(Effect::Deny)
                .ok_or_else(|| "the PreToolUse hook denied the call without a reason".to_string())
        }
        HookEventName::PostToolUse => {
            let context = [
                specific("additionalContext"),
                field("reason").filter(|_| blocks),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
            Ok(if context.is_empty() {
                Effect::None
            } else {
                Effect::Context(context.join("\n\n"))
            })
        }
        HookEventName::Stop if !goes_on => Ok(Effect::Stop),
        HookEventName::Stop if blocks => field("reason")
            .map(Effect::Continue)
            .ok_or_else(|| "the Stop hook blocked without a reason".to_string()),
        HookEventName::Stop => Ok(Effect::None),
        HookEventName::UserPromptSubmit | HookEventName::SessionStart => {
            if blocks || !goes_on {
                return Err(format!("a {} hook cannot block the turn", event.name()));
            }
            Ok(specific("additionalContext").map_or(Effect::None, Effect::Context))
        }
    }
}

fn trimmed(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

#[cfg(test)]
mod tests;
