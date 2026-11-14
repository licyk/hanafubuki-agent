//! What the model reads when `spawn_agent`'s `provider`, `model` or
//! `reasoning_effort` is refused: an error naming the argument, with what
//! it can pass instead (the enabled Providers, the known ids closest to
//! the model asked for, the model's levels).

use std::cmp::Reverse;
use std::collections::BTreeSet;

use hanafubuki_agent_contracts::ToolCallValidationError;
use hanafubuki_agent_kernel::{AgentError, KnownReasoning, ModelReasoningEffort};

use crate::session_driver::ChildModelError;

/// The most known ids an unknown model's refusal lists.
const MAXIMUM_CLOSEST_MODELS: usize = 20;

/// The refusal the model reads for `error`.
pub(super) fn refusal(error: ChildModelError) -> AgentError {
    let (field, reason) = match error {
        ChildModelError::UnknownProvider { requested, enabled } => {
            ("provider", unknown_provider(&requested, &enabled))
        }
        ChildModelError::UnusableProvider(reason) => ("provider", reason),
        ChildModelError::UnknownModel {
            model,
            explicit,
            provider_id,
            provider_name,
            known,
        } => {
            let provider = format!("Provider `{provider_id}` ({provider_name})");
            let reason = if explicit {
                format!("`{model}` is not a model {provider} lists")
            } else {
                format!(
                    "the model this agent would inherit, `{model}`, is not one {provider} lists; pass a `model`"
                )
            };
            ("model", format!("{reason}; {}", closest(&known, &model)))
        }
        ChildModelError::UnknownEffort {
            effort,
            model,
            known,
        } => ("reasoning_effort", unknown_effort(&effort, &model, &known)),
        ChildModelError::Failed(error) => return AgentError::Persistence(error),
    };
    AgentError::InvalidInput(
        ToolCallValidationError::InvalidArgumentAt {
            path: field.to_string(),
            reason,
        }
        .to_string(),
    )
}

fn unknown_provider(requested: &str, enabled: &[(String, String)]) -> String {
    if enabled.is_empty() {
        return format!("`{requested}` is not an enabled Provider, and none is enabled");
    }
    let enabled = enabled
        .iter()
        .map(|(id, name)| format!("`{id}` ({name})"))
        .collect::<Vec<_>>();
    format!(
        "`{requested}` is not an enabled Provider; use one of {}",
        enabled.join(", ")
    )
}

/// Up to [`MAXIMUM_CLOSEST_MODELS`] of `known`, those sharing the longest
/// start with `model` first, then by id, and how many more there are.
fn closest(known: &BTreeSet<String>, model: &str) -> String {
    let mut ids = known.iter().collect::<Vec<_>>();
    // Stable, so ids sharing as long a start stay in id order.
    ids.sort_by_key(|id| Reverse(shared_start(id, model)));
    let listed = ids
        .iter()
        .take(MAXIMUM_CLOSEST_MODELS)
        .map(|id| format!("`{id}`"))
        .collect::<Vec<_>>();
    let more = ids.len().saturating_sub(MAXIMUM_CLOSEST_MODELS);
    if more == 0 {
        format!("closest: {}", listed.join(", "))
    } else {
        format!("closest: {} (and {more} more)", listed.join(", "))
    }
}

fn shared_start(id: &str, model: &str) -> usize {
    id.chars()
        .zip(model.chars())
        .take_while(|(left, right)| left == right)
        .count()
}

fn unknown_effort(effort: &ModelReasoningEffort, model: &str, known: &KnownReasoning) -> String {
    match known {
        KnownReasoning::Levels { efforts, default } => {
            let levels = efforts
                .iter()
                .map(|level| {
                    if Some(level) == default.as_ref() {
                        format!("{level} (default)")
                    } else {
                        level.to_string()
                    }
                })
                .collect::<Vec<_>>();
            format!(
                "`{effort}` is not a reasoning effort `{model}` takes; use one of {}",
                levels.join(", ")
            )
        }
        KnownReasoning::Unsupported { .. } | KnownReasoning::Unknown => {
            format!("`{model}` takes no reasoning effort; omit `reasoning_effort`")
        }
    }
}
