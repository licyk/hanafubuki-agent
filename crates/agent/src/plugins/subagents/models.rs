//! What the model reads of `spawn_agent`'s `provider`, `model` and
//! `reasoning_effort`: the guidance's Models part, which lists what it may
//! pass, and the error when what it passed is refused, naming the argument
//! with what it can pass instead (the enabled Providers, the known ids
//! closest to the model asked for, the model's levels). Both name a
//! Provider, a model and its levels the same way.

use std::cmp::Reverse;
use std::collections::BTreeSet;

use hanafubuki_agent_contracts::ToolCallValidationError;
use hanafubuki_agent_kernel::{AgentError, KnownReasoning, ModelReasoningEffort};

use super::prompt::one_line;
use crate::session_driver::{ChildModelError, SpawnModelCatalog, SpawnProvider};

/// The most known ids an unknown model's refusal lists.
const MAXIMUM_CLOSEST_MODELS: usize = 20;
/// The most bytes the guidance's Models part takes.
const MAXIMUM_MODELS_GUIDANCE_BYTES: usize = 4096;
/// The most reasoning efforts it shows of a model.
const MAXIMUM_LISTED_EFFORTS: usize = 8;
/// The longest Provider name it shows, in characters.
const MAXIMUM_PROVIDER_NAME_CHARS: usize = 64;
/// The line that ends the Models part when the rest would pass its bound.
const LEFT_OUT: &str =
    "- … the rest of this list is left out; what it leaves out is still accepted";

/// The guidance's Models part: the parent's model, then each Provider of
/// `catalog` with its known models and their reasoning efforts, and what
/// is left out; at most [`MAXIMUM_MODELS_GUIDANCE_BYTES`], ended by
/// [`LEFT_OUT`] where the next line would pass them.
pub(super) fn models_guidance(catalog: &SpawnModelCatalog) -> String {
    let mut text = format!(
        "`spawn_agent`'s `provider`, `model` and `reasoning_effort` choose a sub-agent's Provider, model and reasoning effort; without them it runs its role's, else yours (`{}` of `{}`). The enabled Providers (pass the id) and the models known of them:",
        catalog.model, catalog.provider_id
    );
    let lines = catalog_lines(catalog);
    let whole = lines.iter().map(|line| line.len() + 1).sum::<usize>();
    let budget = if text.len() + whole <= MAXIMUM_MODELS_GUIDANCE_BYTES {
        MAXIMUM_MODELS_GUIDANCE_BYTES
    } else {
        MAXIMUM_MODELS_GUIDANCE_BYTES - LEFT_OUT.len() - 1
    };
    for line in &lines {
        if text.len() + line.len() + 1 > budget {
            text.push('\n');
            text.push_str(LEFT_OUT);
            break;
        }
        text.push('\n');
        text.push_str(line);
    }
    text
}

fn catalog_lines(catalog: &SpawnModelCatalog) -> Vec<String> {
    let mut lines = Vec::new();
    for provider in &catalog.providers {
        lines.push(provider_line(provider, provider.id == catalog.provider_id));
        for (model, reasoning) in &provider.models {
            lines.push(match reasoning {
                KnownReasoning::Levels { efforts, default } if !efforts.is_empty() => {
                    let mut shown = levels(efforts, default.as_ref());
                    let more = shown.len().saturating_sub(MAXIMUM_LISTED_EFFORTS);
                    shown.truncate(MAXIMUM_LISTED_EFFORTS);
                    if more > 0 {
                        shown.push(format!("and {more} more"));
                    }
                    format!("  - `{model}`: reasoning efforts {}", shown.join(", "))
                }
                KnownReasoning::Unsupported { .. } => {
                    format!("  - `{model}`: {NO_REASONING}")
                }
                KnownReasoning::Levels { .. } | KnownReasoning::Unknown => format!("  - `{model}`"),
            });
        }
        if provider.more_models > 0 {
            lines.push(format!("  - … and {} more", provider.more_models));
        }
    }
    if catalog.more_providers > 0 {
        lines.push(format!(
            "- … and {} more enabled Providers",
            catalog.more_providers
        ));
    }
    lines
}

fn provider_line(provider: &SpawnProvider, yours: bool) -> String {
    let mut line = format!("- {}", provider_label(&provider.id, &provider.display_name));
    if yours {
        line.push_str(", yours");
    }
    let has_models = !provider.models.is_empty() || provider.more_models > 0;
    if !provider.listed {
        line.push_str(", with no listing known (any model id is passed to it as given)");
    } else if !has_models {
        line.push_str(", whose listing names no model");
    }
    if has_models {
        line.push(':');
    }
    line
}

/// A Provider as the model reads it: its id, which `provider` takes, and
/// its display name on one line.
fn provider_label(id: &str, display_name: &str) -> String {
    format!(
        "`{id}` ({})",
        one_line(display_name, MAXIMUM_PROVIDER_NAME_CHARS)
    )
}

/// What a model that takes no reasoning effort is said to do.
const NO_REASONING: &str = "takes no reasoning effort";

/// `efforts`, `default` marked.
fn levels(efforts: &[ModelReasoningEffort], default: Option<&ModelReasoningEffort>) -> Vec<String> {
    efforts
        .iter()
        .map(|level| {
            if Some(level) == default {
                format!("{level} (default)")
            } else {
                level.to_string()
            }
        })
        .collect()
}

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
            let provider = format!("Provider {}", provider_label(&provider_id, &provider_name));
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
        .map(|(id, name)| provider_label(id, name))
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
        KnownReasoning::Levels { efforts, default } => format!(
            "`{effort}` is not a reasoning effort `{model}` takes; use one of {}",
            levels(efforts, default.as_ref()).join(", ")
        ),
        KnownReasoning::Unsupported { .. } | KnownReasoning::Unknown => {
            format!("`{model}` {NO_REASONING}; omit `reasoning_effort`")
        }
    }
}
