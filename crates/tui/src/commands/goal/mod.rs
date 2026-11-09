// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/goal [objective|clear|pause|resume|edit]`: the goal the session keeps
//! working toward (the facade's goal calls; Codex's `/goal`, its summary,
//! its replace confirmation and its edit prompt). Without arguments a menu
//! shows the goal, its status, budgets and usage, with its actions;
//! `/goal OBJECTIVE` sets one (`--tokens N` first gives it a token
//! budget), asking first when it would replace an unfinished goal.
//! Editing replaces the goal, so its usage starts again.

mod menu;

use hanafubuki_agent::{AgentGoal, AgentGoalBudgets, AgentGoalStatus};

use super::{CommandContext, CommandSpec};
use crate::agent_bridge::{GoalCall, GoalReply};
use crate::app_event::{AgentCommand, AppEvent, AppEventSender, Op};
use crate::bottom_pane::{SelectionItem, SelectionViewParams};
use crate::commands::prompt::PromptView;
use crate::history_cell;

const USAGE: &str =
    "Usage: /goal [<objective> | --tokens N <objective> | clear | edit | pause | resume]";

pub(crate) static GOAL: CommandSpec = CommandSpec {
    name: "goal",
    aliases: &[],
    description: "set or view the goal the session keeps working toward",
    inline_args: true,
    during_task: true,
    needs_session: true,
    dispatch,
};

fn dispatch(context: &mut CommandContext<'_>, args: &str) {
    let call = match args.trim() {
        "" => GoalCall::Menu,
        "clear" => GoalCall::Clear,
        "pause" => GoalCall::Pause,
        "resume" => GoalCall::Resume,
        "edit" => GoalCall::Edit,
        args => match objective(args) {
            Some((objective, token_budget)) => GoalCall::Set {
                objective,
                budgets: AgentGoalBudgets {
                    token_budget,
                    time_budget_seconds: None,
                },
                confirmed: false,
            },
            None => {
                return context.add_to_history(history_cell::new_error_event(USAGE.to_string()))
            }
        },
    };
    context.agent_command(AgentCommand::Goal(call));
}

pub(crate) fn on_reply(context: &mut CommandContext<'_>, reply: GoalReply) {
    match reply {
        GoalReply::Menu(None) => info(context, "No goal is set.", Some(USAGE)),
        GoalReply::Menu(Some(goal)) => menu::show(context, &goal),
        GoalReply::Edit(goal) => edit(context, goal),
        GoalReply::Budget(goal) => budget(context, goal),
        GoalReply::ConfirmReplace {
            current,
            objective,
            budgets,
        } => confirm_replace(context, &current, objective, budgets),
        GoalReply::Set(goal) => {
            let hint = if context.task_running() {
                "The running turn turns to it at its next answer."
            } else {
                "The Agent works toward it from your next message."
            };
            info(
                context,
                &format!("Goal set: {}", goal.objective),
                Some(hint),
            );
        }
        GoalReply::Cleared(true) => info(context, "Goal cleared", None),
        GoalReply::Cleared(false) => info(context, "No goal is set.", None),
        GoalReply::Paused(goal) => match goal.status {
            AgentGoalStatus::Paused => info(context, "Goal paused", Some("/goal resume")),
            status => info(
                context,
                &format!(
                    "The goal is {}; only an active goal pauses.",
                    status_label(status)
                ),
                None,
            ),
        },
        GoalReply::Resumed(goal) => match goal.status {
            AgentGoalStatus::BudgetLimited => info(
                context,
                "The goal's budget is spent, so it stays limited by budget.",
                Some("/goal edit gives it a new start"),
            ),
            _ => info(
                context,
                "Goal resumed",
                Some("It continues from your next message."),
            ),
        },
        GoalReply::Failed(error) => {
            context.add_to_history(history_cell::new_error_event(format!("/goal: {error}")))
        }
    }
}

/// `--tokens N OBJECTIVE`, or the objective alone; `None` when the budget
/// does not read or no objective follows it.
fn objective(args: &str) -> Option<(String, Option<u64>)> {
    let Some(rest) = args.strip_prefix("--tokens") else {
        return Some((args.to_string(), None));
    };
    let (count, objective) = rest.trim_start().split_once(char::is_whitespace)?;
    let objective = objective.trim();
    let tokens = token_count(count)?;
    (!objective.is_empty()).then(|| (objective.to_string(), Some(tokens)))
}

/// A positive count of tokens: digits, with `k` or `m` for thousands or
/// millions.
fn token_count(text: &str) -> Option<u64> {
    let text = text.trim().to_ascii_lowercase();
    let (digits, scale) = match text.strip_suffix('k') {
        Some(digits) => (digits, 1_000),
        None => match text.strip_suffix('m') {
            Some(digits) => (digits, 1_000_000),
            None => (text.as_str(), 1),
        },
    };
    digits
        .parse::<u64>()
        .ok()
        .and_then(|count| count.checked_mul(scale))
        .filter(|count| *count > 0)
}

/// The prompt that edits the objective: what is submitted replaces the
/// goal, with its budgets.
fn edit(context: &mut CommandContext<'_>, goal: AgentGoal) {
    let budgets = budgets(&goal);
    let mut view = PromptView::new(
        "Edit goal",
        "Type the objective and press Enter",
        context.app_event_tx(),
        Box::new(move |tx, objective| set(tx, objective, budgets)),
    );
    view.set_text(&goal.objective);
    view.set_note(Some(
        "This replaces the goal: its usage starts again.".to_string(),
    ));
    context.show_view(Box::new(view));
}

/// The prompt that sets the token budget: the goal is set again with it.
fn budget(context: &mut CommandContext<'_>, goal: AgentGoal) {
    let objective = goal.objective.clone();
    let time_budget_seconds = goal.time_budget_seconds;
    let mut view = PromptView::new(
        "Token budget",
        "A number of tokens, such as 200000 or 200k",
        context.app_event_tx(),
        Box::new(move |tx, count| match token_count(&count) {
            Some(tokens) => set(
                tx,
                objective.clone(),
                AgentGoalBudgets {
                    token_budget: Some(tokens),
                    time_budget_seconds,
                },
            ),
            None => tx.send(AppEvent::InsertHistoryCell(Box::new(
                history_cell::new_error_event(format!(
                    "/goal: `{count}` is not a number of tokens"
                )),
            ))),
        }),
    );
    if let Some(tokens) = goal.token_budget {
        view.set_text(&tokens.to_string());
    }
    view.set_note(Some(
        "This sets the goal again with the budget: its usage starts again.".to_string(),
    ));
    context.show_view(Box::new(view));
}

/// Codex's "Replace goal?"; keeping the current goal comes first.
fn confirm_replace(
    context: &mut CommandContext<'_>,
    current: &AgentGoal,
    objective: String,
    budgets: AgentGoalBudgets,
) {
    let subtitle = format!(
        "Current goal ({}): {}",
        status_label(current.status),
        truncate(&current.objective)
    );
    let items = vec![
        SelectionItem {
            name: "Cancel".to_string(),
            description: Some("Keep the current goal".to_string()),
            dismiss_on_select: true,
            ..Default::default()
        },
        SelectionItem {
            name: "Replace current goal".to_string(),
            description: Some(format!("New objective: {}", truncate(&objective))),
            actions: vec![Box::new(move |tx| set(tx, objective.clone(), budgets))],
            dismiss_on_select: true,
            ..Default::default()
        },
    ];
    context.show_selection_view(SelectionViewParams {
        title: Some("Replace goal?".to_string()),
        subtitle: Some(subtitle),
        items,
        ..Default::default()
    });
}

fn info(context: &mut CommandContext<'_>, message: &str, hint: Option<&str>) {
    context.add_to_history(history_cell::new_info_event(
        message.to_string(),
        hint.map(str::to_string),
    ));
}

/// Sets the goal without asking: the person has confirmed it.
fn set(tx: &AppEventSender, objective: String, budgets: AgentGoalBudgets) {
    tx.send(AppEvent::Op(Op::Command(AgentCommand::Goal(
        GoalCall::Set {
            objective,
            budgets,
            confirmed: true,
        },
    ))));
}

fn budgets(goal: &AgentGoal) -> AgentGoalBudgets {
    AgentGoalBudgets {
        token_budget: goal.token_budget,
        time_budget_seconds: goal.time_budget_seconds,
    }
}

pub(crate) fn status_label(status: AgentGoalStatus) -> &'static str {
    match status {
        AgentGoalStatus::Active => "active",
        AgentGoalStatus::Paused => "paused",
        AgentGoalStatus::Blocked => "blocked",
        AgentGoalStatus::BudgetLimited => "limited by budget",
        AgentGoalStatus::Complete => "complete",
    }
}

/// At most 200 characters of `text`, on one line.
fn truncate(text: &str) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match line.char_indices().nth(200) {
        Some((end, _)) => format!("{}…", &line[..end]),
        None => line,
    }
}

#[cfg(test)]
mod tests;
