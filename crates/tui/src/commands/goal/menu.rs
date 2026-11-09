// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The goal's summary menu (Codex's goal summary and `goal_display.rs`,
//! as a menu): the objective, its status, budgets and usage, then what can
//! be done with it.

use hanafubuki_agent::{AgentGoal, AgentGoalStatus};
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Wrap};

use super::status_label;
use crate::agent_bridge::GoalCall;
use crate::app_event::{AgentCommand, AppEvent, AppEventSender, Op};
use crate::bottom_pane::{SelectionItem, SelectionViewParams};
use crate::commands::CommandContext;
use crate::history_cell::format_tokens_compact;

pub(super) fn show(context: &mut CommandContext<'_>, goal: &AgentGoal) {
    let mut items = Vec::new();
    match goal.status {
        AgentGoalStatus::Active => items.push(action(
            "Pause goal",
            "stop continuing it; /goal resume picks it up",
            GoalCall::Pause,
        )),
        AgentGoalStatus::Paused | AgentGoalStatus::Blocked => items.push(action(
            "Resume goal",
            "continue it from the next message",
            GoalCall::Resume,
        )),
        AgentGoalStatus::BudgetLimited | AgentGoalStatus::Complete => {}
    }
    items.push(action(
        "Edit objective",
        "replace the goal; its usage starts again",
        GoalCall::Edit,
    ));
    items.push(action(
        "Set token budget",
        "set the goal again with a budget; its usage starts again",
        GoalCall::Budget,
    ));
    items.push(action("Clear goal", "remove the goal", GoalCall::Clear));
    context.show_selection_view(SelectionViewParams {
        header: Box::new(Paragraph::new(summary_lines(goal)).wrap(Wrap { trim: false })),
        items,
        ..Default::default()
    });
}

/// The objective, the status, the budgets and what is used of them.
pub(super) fn summary_lines(goal: &AgentGoal) -> Vec<Line<'static>> {
    let mut tokens = format_tokens_compact(goal.tokens_used);
    if let Some(budget) = goal.token_budget {
        tokens = format!("{tokens} / {}", format_tokens_compact(budget));
    }
    let mut time = elapsed(goal.time_used_seconds);
    if let Some(budget) = goal.time_budget_seconds {
        time = format!("{time} / {}", elapsed(budget));
    }
    vec![
        Line::from("Goal".bold()),
        Line::from(vec!["Objective: ".dim(), goal.objective.clone().into()]),
        Line::from(vec!["Status: ".dim(), status_label(goal.status).into()]),
        Line::from(vec!["Tokens: ".dim(), tokens.into()]),
        Line::from(vec!["Time: ".dim(), time.into()]),
    ]
}

fn action(name: &str, description: &str, call: GoalCall) -> SelectionItem {
    SelectionItem {
        name: name.to_string(),
        description: Some(description.to_string()),
        actions: vec![Box::new(move |tx: &AppEventSender| {
            tx.send(AppEvent::Op(Op::Command(AgentCommand::Goal(call.clone()))))
        })],
        dismiss_on_select: true,
        ..Default::default()
    }
}

/// Codex's compact duration: `45s`, `30m`, `1h 30m`, `2d 23h 42m`.
pub(super) fn elapsed(seconds: u64) -> String {
    if seconds < 60 {
        return format!("{seconds}s");
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes}m");
    }
    let (hours, minutes) = (minutes / 60, minutes % 60);
    if hours >= 24 {
        return format!("{}d {}h {minutes}m", hours / 24, hours % 24);
    }
    if minutes == 0 {
        format!("{hours}h")
    } else {
        format!("{hours}h {minutes}m")
    }
}
