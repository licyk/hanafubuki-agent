// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/effort`: the session's reasoning effort, picked from the efforts
//! known for its model (Codex's reasoning popup, `chatwidget/model_popups.rs`):
//! the levels its configuration or its Provider's listing names, then
//! Default (the Provider's) and Custom, typed. `/effort VALUE` and
//! `/effort default` set it at once. The kernel takes a change only
//! between turns, so it is refused during one.

use hanafubuki_agent::{reasoning_effort_choices, KnownReasoning, ModelReasoningEffort};

use super::prompt::PromptView;
use super::{send_step, CommandContext, CommandSpec};
use crate::agent_bridge::{EffortCall, EffortChoices};
use crate::app_event::{AgentCommand, AppEvent, AppEventSender, Op};
use crate::bottom_pane::{SelectionItem, SelectionViewParams};
use crate::history_cell;
use crate::model_facts::{effort_options, EffortOption, EffortValue};

pub(crate) static EFFORT: CommandSpec = CommandSpec {
    name: "effort",
    aliases: &[],
    description: "choose the reasoning effort of the model",
    inline_args: true,
    // Refused during a turn by dispatch, which says what to do instead.
    during_task: true,
    needs_session: true,
    dispatch,
};

/// What a change of the effort during a turn is told.
pub(crate) const DURING_TASK: &str = "'/effort' is disabled while a task is in progress. \
                                      Change the effort after this turn, or interrupt it.";

fn dispatch(context: &mut CommandContext<'_>, args: &str) {
    if context.task_running() {
        context.add_to_history(history_cell::new_error_event(DURING_TASK.to_string()));
        return;
    }
    match args.trim() {
        "" => context.agent_command(AgentCommand::Effort(EffortCall::List)),
        "default" => context.agent_command(AgentCommand::Effort(EffortCall::Set(None))),
        effort => match ModelReasoningEffort::parse(effort) {
            Ok(effort) => {
                context.agent_command(AgentCommand::Effort(EffortCall::Set(Some(effort))))
            }
            Err(error) => {
                context.add_to_history(history_cell::new_error_event(format!("/effort: {error}")))
            }
        },
    }
}

/// The picker over the efforts known for the session's model.
pub(crate) fn show_effort_choices(context: &mut CommandContext<'_>, choices: EffortChoices) {
    let known = choices.metadata.known_reasoning();
    let current = choices.current.as_ref().map(ModelReasoningEffort::as_str);
    let items = effort_options(&reasoning_effort_choices(&known), current)
        .into_iter()
        .map(|option| effort_item(option, current))
        .collect();
    let model = &choices.model;
    let subtitle = match known {
        KnownReasoning::Levels { .. } => {
            format!("The reasoning effort of {model} in this session.")
        }
        KnownReasoning::Unsupported { learned: true } => {
            format!("The Provider refused the reasoning parameter for {model}: no effort is sent.")
        }
        KnownReasoning::Unsupported { learned: false } => {
            format!("{model} takes no reasoning effort: none is sent.")
        }
        KnownReasoning::Unknown => {
            format!("No reasoning levels are known for {model}: keep the default or type one.")
        }
    };
    context.show_selection_view(SelectionViewParams {
        title: Some("Select Reasoning Effort".to_string()),
        subtitle: Some(subtitle),
        items,
        ..Default::default()
    });
}

fn effort_item(option: EffortOption, current: Option<&str>) -> SelectionItem {
    let effort = match &option.value {
        EffortValue::Level(level) => ModelReasoningEffort::parse(level.as_str()).ok(),
        EffortValue::Default | EffortValue::Custom => None,
    };
    let custom = option.value == EffortValue::Custom;
    // The custom effort set is typed again from itself.
    let typed = current
        .filter(|_| custom && option.current)
        .map(str::to_string);
    SelectionItem {
        name: option.label,
        description: option.description.map(str::to_string),
        is_current: option.current,
        actions: vec![Box::new(move |tx| {
            if custom {
                let typed = typed.clone();
                send_step(tx, &EFFORT, move |context| show_custom(context, typed));
            } else {
                set_effort(tx, effort.clone());
            }
        })],
        dismiss_on_select: true,
        ..Default::default()
    }
}

/// The prompt for an effort the list does not have, starting from the
/// session's.
fn show_custom(context: &mut CommandContext<'_>, current: Option<String>) {
    let mut view = PromptView::new(
        "Custom reasoning effort",
        "Type an effort and press Enter",
        context.app_event_tx(),
        Box::new(|tx, effort| {
            if let Ok(effort) = ModelReasoningEffort::parse(effort) {
                set_effort(tx, Some(effort));
            }
        }),
    )
    .with_check(|effort| ModelReasoningEffort::parse(effort).map(|_| ()));
    view.set_text(current.as_deref().unwrap_or_default());
    view.set_note(Some(
        "1 to 64 ASCII letters, digits, '-', '_' or '.'; sent as typed".to_string(),
    ));
    context.show_view(Box::new(view));
}

fn set_effort(tx: &AppEventSender, effort: Option<ModelReasoningEffort>) {
    tx.send(AppEvent::Op(Op::Command(AgentCommand::Effort(
        EffortCall::Set(effort),
    ))));
}

#[cfg(test)]
mod tests;
