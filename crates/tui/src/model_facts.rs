// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! How the interface names what is known of a model: what a model list
//! shows of it (its fact columns, and the detail line of the one selected),
//! and the reasoning efforts offered for it, which Alt+, and Alt+. step
//! through. The sources' names are model-api's
//! (`ModelMetadataSource::label`).

use hanafubuki_agent::{
    reasoning_effort_choices, KnownReasoning, ModelMetadata, ModelReasoningEffort, SettingChoice,
};

use crate::bottom_pane::RowFacts;
use crate::history_cell::format_tokens_compact;

/// The names over [`fact_columns`], as wide.
pub(crate) const FACTS_HEADER: &str = "Context  Thinking  Vision";

/// What a model list says of a model: its window (`200K`, or `?` when not
/// known), and whether it thinks and reads images (`yes`, `no` or `?`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FactCells {
    pub(crate) window: String,
    pub(crate) thinking: &'static str,
    pub(crate) vision: &'static str,
}

pub(crate) fn fact_cells(metadata: &ModelMetadata) -> FactCells {
    FactCells {
        window: metadata.context_window.as_ref().map_or_else(
            || "?".to_string(),
            |window| format_tokens_compact(window.value),
        ),
        thinking: answer(metadata.supports_reasoning()),
        vision: answer(metadata.supports_image_input()),
    }
}

impl FactCells {
    /// Some cell says `?`.
    pub(crate) fn has_unknown(&self) -> bool {
        self.window == "?" || self.thinking == "?" || self.vision == "?"
    }
}

fn answer(known: Option<bool>) -> &'static str {
    match known {
        Some(true) => "yes",
        Some(false) => "no",
        None => "?",
    }
}

/// The cells in columns under [`FACTS_HEADER`].
pub(crate) fn fact_columns(cells: &FactCells) -> String {
    format!(
        "{:>7}  {:<8}  {:<6}",
        cells.window, cells.thinking, cells.vision
    )
}

/// The cells on one line, each named: `context 200K · thinking yes ·
/// vision no`.
pub(crate) fn fact_line(cells: &FactCells) -> String {
    format!(
        "context {} · thinking {} · vision {}",
        cells.window, cells.thinking, cells.vision
    )
}

/// A model's row facts, in columns and on one line.
pub(crate) fn row_facts(metadata: &ModelMetadata) -> RowFacts {
    let cells = fact_cells(metadata);
    RowFacts {
        columns: fact_columns(&cells),
        stacked: fact_line(&cells),
    }
}

/// The keys of a model list whose model rows Enter chooses, `esc` saying
/// what Esc does: `? not known`, and, while models.dev is off and a row
/// `shows_unknown`, where it is turned on.
pub(crate) fn choose_hint(esc: &str, shows_unknown: bool, models_dev: bool) -> String {
    let mut hint = format!("enter choose · {esc} · ? not known");
    if shows_unknown && !models_dev {
        hint.push_str(" · models.dev can fill these: /config modelMetadata");
    }
    hint
}

/// The detail line of a model: `model` on `provider`, then each fact with
/// where it came from (`context 200K (reported) · max output 64K
/// (reported) · thinking low, high (reported) · vision no (learned)`).
pub(crate) fn fact_details(model: &str, provider: &str, metadata: &ModelMetadata) -> String {
    let window = metadata.context_window.as_ref().map_or_else(
        || "context not known (compacts on the Provider's error)".to_string(),
        |window| {
            format!(
                "context {} ({})",
                format_tokens_compact(window.value),
                window.source.label()
            )
        },
    );
    let output = metadata.max_output_tokens.as_ref().map(|output| {
        format!(
            "max output {} ({})",
            format_tokens_compact(output.value),
            output.source.label()
        )
    });
    let vision = metadata.image_input.as_ref().map_or_else(
        || "vision not known".to_string(),
        |vision| {
            format!(
                "vision {} ({})",
                answer(Some(vision.value)),
                vision.source.label()
            )
        },
    );
    let facts: Vec<String> = [
        Some(window),
        output,
        Some(thinking_detail(metadata)),
        Some(vision),
    ]
    .into_iter()
    .flatten()
    .collect();
    format!("{model} · {provider}: {}", facts.join(" · "))
}

/// Whether a model thinks, and at which levels when they are known.
fn thinking_detail(metadata: &ModelMetadata) -> String {
    let source = metadata
        .reasoning
        .as_ref()
        .map_or("", |fact| fact.source.label());
    match (metadata.known_reasoning(), metadata.supports_reasoning()) {
        (KnownReasoning::Levels { efforts, default }, _) => {
            let levels: Vec<&str> = efforts.iter().map(ModelReasoningEffort::as_str).collect();
            let default = default
                .map(|effort| format!(", default {}", effort.as_str()))
                .unwrap_or_default();
            format!("thinking {}{default} ({source})", levels.join(", "))
        }
        (_, Some(true)) => format!("thinking yes, levels not known ({source})"),
        (KnownReasoning::Unsupported { learned: true }, _) => {
            "thinking no (learned: the Provider refused it)".to_string()
        }
        (_, Some(false)) => format!("thinking no ({source})"),
        (_, None) => "thinking not known".to_string(),
    }
}

/// What an effort option sets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum EffortValue {
    Level(String),
    /// Unset: the Provider's default.
    Default,
    /// One typed.
    Custom,
}

/// One option of `/effort` and of the settings center's effort row.
#[derive(Clone, Debug)]
pub(crate) struct EffortOption {
    pub(crate) value: EffortValue,
    pub(crate) label: String,
    pub(crate) description: Option<&'static str>,
    pub(crate) current: bool,
}

/// The efforts offered for a model, the same wherever one is chosen: its
/// known levels (`levels`, `reasoning_effort_choices`), then Default and
/// Custom. The `current` effort is marked; one off the levels is the
/// custom one, named.
pub(crate) fn effort_options(levels: &[SettingChoice], current: Option<&str>) -> Vec<EffortOption> {
    let listed = levels
        .iter()
        .any(|level| Some(level.value.as_str()) == current);
    let custom = current.filter(|_| !listed);
    levels
        .iter()
        .map(|level| EffortOption {
            value: EffortValue::Level(level.value.clone()),
            label: level.label.clone(),
            description: None,
            current: Some(level.value.as_str()) == current,
        })
        .chain([
            EffortOption {
                value: EffortValue::Default,
                label: "Default".to_string(),
                description: Some("unset: the Provider's default"),
                current: current.is_none(),
            },
            EffortOption {
                value: EffortValue::Custom,
                label: custom.map_or_else(
                    || "Custom…".to_string(),
                    |effort| format!("Custom: {effort}"),
                ),
                description: Some("type an effort"),
                current: custom.is_some(),
            },
        ])
        .collect()
}

/// What a step of the effort (Alt+, and Alt+.) does.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum EffortStep {
    /// The session's effort becomes this level.
    To(ModelReasoningEffort),
    /// It stays, and the person is told why.
    Stays(String),
}

/// The level after (`raise`) or before the `current` effort among the
/// levels [`effort_options`] offers (Codex's `reasoning_shortcuts.rs`): an
/// unset effort, or one off the levels, counts as the model's default,
/// else the first level; a step past an end stays there.
pub(crate) fn step_effort(
    known: &KnownReasoning,
    current: Option<&str>,
    raise: bool,
) -> EffortStep {
    let levels: Vec<String> = effort_options(&reasoning_effort_choices(known), current)
        .into_iter()
        .filter_map(|option| match option.value {
            EffortValue::Level(level) => Some(level),
            EffortValue::Default | EffortValue::Custom => None,
        })
        .collect();
    if levels.is_empty() {
        return EffortStep::Stays(
            "No reasoning levels are known for this model; use /effort to type one.".to_string(),
        );
    }
    let default = match known {
        KnownReasoning::Levels { default, .. } => {
            default.as_ref().map(ModelReasoningEffort::as_str)
        }
        _ => None,
    };
    let index = |effort: Option<&str>| {
        effort.and_then(|effort| levels.iter().position(|level| level == effort))
    };
    let at = index(current).or_else(|| index(default)).unwrap_or(0);
    let next = if raise {
        Some(at + 1).filter(|next| *next < levels.len())
    } else {
        at.checked_sub(1)
    };
    match next.and_then(|next| ModelReasoningEffort::parse(&levels[next]).ok()) {
        Some(effort) => EffortStep::To(effort),
        None => EffortStep::Stays(format!(
            "Reasoning is already at the {} level ({}).",
            if raise { "highest" } else { "lowest" },
            capitalized(&levels[at])
        )),
    }
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
