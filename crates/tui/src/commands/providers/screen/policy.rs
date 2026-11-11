// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The shared model policy form: which Provider and model new sessions run,
//! and how every model runs.

use hanafubuki_agent::{
    AgentImageInputPolicy, AgentModelPolicySettings, AgentProviderRegistrySnapshot,
    AgentRemoteCompactionCapability, AgentTurnBudgetSettings, ModelReasoningEffort,
    MAX_AGENT_CONSECUTIVE_STALLED_STEPS, MAX_AGENT_CUMULATIVE_MODEL_INPUT_TOKENS,
    MAX_AGENT_CUMULATIVE_MODEL_OUTPUT_TOKENS, MAX_AGENT_HOOK_FRAGMENT_BYTES,
    MAX_AGENT_IDLE_TIMEOUT_SECONDS, MAX_AGENT_IN_FLIGHT_TOOLS, MAX_AGENT_PENDING_INPUT_ITEMS,
    MAX_AGENT_TOOL_INVOCATIONS, MAX_AGENT_TOOL_OUTPUT_BYTES,
};

use crate::bottom_pane::{FieldId, FormItem, FormView};

pub(super) const DEFAULT_PROVIDER: FieldId = "default_provider";
const MODEL: FieldId = "model";
const REASONING: FieldId = "reasoning";
const IMAGE_INPUT: FieldId = "image_input";
const CONTEXT_WINDOW: FieldId = "context_window";
const AUTO_COMPACTION: FieldId = "auto_compaction";
const THRESHOLD: FieldId = "threshold";
const REMOTE_COMPACTION: FieldId = "remote_compaction";
const TURN_BUDGET: FieldId = "turn_budget";
const SAVE: FieldId = "save";

const IMAGE_POLICIES: [AgentImageInputPolicy; 2] =
    [AgentImageInputPolicy::Auto, AgentImageInputPolicy::Disabled];
const REMOTE_COMPACTIONS: [AgentRemoteCompactionCapability; 3] = [
    AgentRemoteCompactionCapability::Disabled,
    AgentRemoteCompactionCapability::ResponsesCompact,
    AgentRemoteCompactionCapability::ResponsesCompactV2,
];

/// The fields of an optional turn budget, their labels, current/default
/// values and supported range.
const BUDGET_NUMBERS: [(FieldId, &str, u64, u64); 11] = [
    (
        "input_tokens",
        "Input tokens",
        8_000_000,
        MAX_AGENT_CUMULATIVE_MODEL_INPUT_TOKENS,
    ),
    (
        "output_tokens",
        "Output tokens",
        1_000_000,
        MAX_AGENT_CUMULATIVE_MODEL_OUTPUT_TOKENS,
    ),
    ("tool_calls", "Tool calls", 256, MAX_AGENT_TOOL_INVOCATIONS),
    (
        "tool_output_bytes",
        "Tool output bytes",
        16 * 1024 * 1024,
        MAX_AGENT_TOOL_OUTPUT_BYTES,
    ),
    (
        "hook_fragment_bytes",
        "Hook fragment bytes",
        1024 * 1024,
        MAX_AGENT_HOOK_FRAGMENT_BYTES,
    ),
    (
        "pending_inputs",
        "Pending inputs",
        64,
        MAX_AGENT_PENDING_INPUT_ITEMS as u64,
    ),
    (
        "in_flight_tools",
        "In-flight tools",
        8,
        MAX_AGENT_IN_FLIGHT_TOOLS as u64,
    ),
    (
        "stalled_steps",
        "Stalled steps",
        4,
        MAX_AGENT_CONSECUTIVE_STALLED_STEPS as u64,
    ),
    (
        "model_idle_seconds",
        "Model idle seconds",
        120,
        MAX_AGENT_IDLE_TIMEOUT_SECONDS,
    ),
    (
        "tool_idle_seconds",
        "Tool idle seconds",
        600,
        MAX_AGENT_IDLE_TIMEOUT_SECONDS,
    ),
    (
        "approval_seconds",
        "Approval seconds",
        900,
        MAX_AGENT_IDLE_TIMEOUT_SECONDS,
    ),
];

pub(super) fn policy_form(registry: &AgentProviderRegistrySnapshot) -> FormView {
    let policy = &registry.model_policy;
    let providers = enabled_providers(registry);
    let default = registry
        .default_provider_id
        .as_deref()
        .and_then(|id| registry.providers.iter().find(|provider| provider.id == id))
        .and_then(|provider| provider_option(registry, &provider.id))
        .map(|index| index + 1)
        .unwrap_or_default();
    let mut options = vec!["none"];
    options.extend(providers.iter().map(String::as_str));

    let budget_values = policy.turn_budget.map_or(
        BUDGET_NUMBERS
            .iter()
            .map(|(_, _, value, _)| *value)
            .collect::<Vec<_>>(),
        budget_values,
    );

    let mut items = vec![
        FormItem::choice(DEFAULT_PROVIDER, "Default Provider", &options, default),
        FormItem::text(MODEL, "Model", &policy.model).placeholder("gpt-5.1"),
        FormItem::text(
            REASONING,
            "Reasoning effort",
            policy
                .reasoning_effort
                .as_ref()
                .map_or("", |effort| effort.as_str()),
        )
        .placeholder("Provider default"),
        FormItem::choice(
            IMAGE_INPUT,
            "Image input",
            &["auto", "disabled"],
            IMAGE_POLICIES
                .iter()
                .position(|value| *value == policy.input_capabilities.image_input)
                .unwrap_or_default(),
        )
        .advanced(),
        FormItem::text(
            CONTEXT_WINDOW,
            "Context window tokens",
            &policy
                .context_window_tokens
                .map_or_else(String::new, |value| value.to_string()),
        )
        .placeholder("catalog or built-in default")
        .advanced(),
        FormItem::toggle(
            AUTO_COMPACTION,
            "Automatic compaction",
            policy.auto_compaction_enabled,
        )
        .advanced(),
        FormItem::text(
            THRESHOLD,
            "Compaction threshold (%)",
            &policy.auto_compact_threshold_percent.to_string(),
        )
        .advanced(),
        FormItem::choice(
            REMOTE_COMPACTION,
            "Remote compaction",
            &["disabled", "responses compact", "responses compact v2"],
            REMOTE_COMPACTIONS
                .iter()
                .position(|value| *value == policy.remote_compaction)
                .unwrap_or_default(),
        )
        .advanced(),
        FormItem::choice(
            TURN_BUDGET,
            "Turn budget",
            &["Agent defaults", "custom limits"],
            usize::from(policy.turn_budget.is_some()),
        )
        .advanced(),
    ];
    for ((id, label, _, _), value) in BUDGET_NUMBERS.iter().zip(budget_values) {
        items.push(
            FormItem::text(id, label, &value.to_string())
                .shown_if(TURN_BUDGET, 1)
                .advanced(),
        );
    }
    items.push(FormItem::action(SAVE, "Save"));
    FormView::new("Model policy", items).with_subtitle(
        "One policy runs every Provider; a protocol that cannot use a value narrows it for a session.",
    )
}

fn budget_values(budget: AgentTurnBudgetSettings) -> Vec<u64> {
    vec![
        budget.maximum_cumulative_model_input_tokens,
        budget.maximum_cumulative_model_output_tokens,
        budget.maximum_tool_invocations,
        budget.maximum_tool_output_bytes,
        budget.maximum_hook_fragment_bytes,
        budget.maximum_pending_input_items as u64,
        budget.maximum_in_flight_tools as u64,
        budget.maximum_consecutive_stalled_steps as u64,
        budget.model_stream_idle_timeout_seconds,
        budget.tool_idle_timeout_seconds,
        budget.approval_timeout_seconds,
    ]
}

fn enabled_providers(registry: &AgentProviderRegistrySnapshot) -> Vec<String> {
    registry
        .providers
        .iter()
        .filter(|provider| provider.enabled)
        .map(|provider| provider_label(registry, provider.id.as_str()))
        .collect()
}

fn provider_label(registry: &AgentProviderRegistrySnapshot, provider_id: &str) -> String {
    let provider = registry
        .providers
        .iter()
        .find(|provider| provider.id == provider_id);
    match provider {
        Some(provider) => format!("{} ({})", provider.display_name, provider.id),
        None => provider_id.to_string(),
    }
}

fn provider_option(registry: &AgentProviderRegistrySnapshot, provider_id: &str) -> Option<usize> {
    enabled_providers(registry)
        .iter()
        .position(|label| *label == provider_label(registry, provider_id))
}

pub(super) fn policy_draft(form: &mut FormView) -> Option<AgentModelPolicySettings> {
    form.clear_errors();
    let reasoning = form.text(REASONING).trim();
    let reasoning = if reasoning.is_empty() {
        None
    } else {
        match ModelReasoningEffort::parse(reasoning) {
            Ok(effort) => Some(effort),
            Err(error) => {
                form.set_error(REASONING, error);
                return None;
            }
        }
    };
    let context = form.text(CONTEXT_WINDOW).trim();
    let context = if context.is_empty() {
        None
    } else {
        match context.parse::<u64>() {
            Ok(value) if value > 0 => Some(value),
            _ => {
                form.set_error(CONTEXT_WINDOW, "Enter a positive whole number.");
                return None;
            }
        }
    };
    let turn_budget = if form.choice(TURN_BUDGET) == 0 {
        None
    } else {
        let values = BUDGET_NUMBERS
            .iter()
            .map(|(id, _, minimum, maximum)| number(form, id, *minimum, *maximum))
            .collect::<Option<Vec<_>>>()?;
        Some(AgentTurnBudgetSettings {
            maximum_cumulative_model_input_tokens: values[0],
            maximum_cumulative_model_output_tokens: values[1],
            maximum_tool_invocations: values[2],
            maximum_tool_output_bytes: values[3],
            maximum_hook_fragment_bytes: values[4],
            maximum_pending_input_items: usize::try_from(values[5]).ok()?,
            maximum_in_flight_tools: usize::try_from(values[6]).ok()?,
            maximum_consecutive_stalled_steps: u32::try_from(values[7]).ok()?,
            model_stream_idle_timeout_seconds: values[8],
            tool_idle_timeout_seconds: values[9],
            approval_timeout_seconds: values[10],
        })
    };
    Some(AgentModelPolicySettings {
        model: form.text(MODEL).trim().to_string(),
        reasoning_effort: reasoning,
        input_capabilities: hanafubuki_agent::AgentInputCapabilitiesSettings {
            image_input: IMAGE_POLICIES.get(form.choice(IMAGE_INPUT)).copied()?,
        },
        context_window_tokens: context,
        auto_compaction_enabled: form.toggle(AUTO_COMPACTION),
        auto_compact_threshold_percent: number(form, THRESHOLD, 1, 100)?,
        remote_compaction: REMOTE_COMPACTIONS
            .get(form.choice(REMOTE_COMPACTION))
            .copied()?,
        turn_budget,
    })
}

/// The Provider id the selected default names.
pub(super) fn default_provider_id(
    registry: &AgentProviderRegistrySnapshot,
    selected: usize,
) -> Option<String> {
    registry
        .providers
        .iter()
        .filter(|provider| provider.enabled)
        .nth(selected.checked_sub(1)?)
        .map(|provider| provider.id.clone())
}

fn number<T: TryFrom<u64>>(
    form: &mut FormView,
    id: FieldId,
    minimum: u64,
    maximum: u64,
) -> Option<T> {
    let value = match form.text(id).trim().parse::<u64>() {
        Ok(value) if value >= minimum && value <= maximum => value,
        _ => {
            form.set_error(
                id,
                format!("Enter a whole number from {minimum} to {maximum}."),
            );
            return None;
        }
    };
    T::try_from(value).ok()
}
