// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/subagents`: the session's subagents (Codex's `/subagents` status feed,
//! `app/agent_status_feed.rs`, and its agent picker): a cell with each
//! one's title, status and the start of its latest answer, then a list to
//! show one of them in place of the session. While one runs the list
//! follows their statuses (the bridge's watch reads them). The subagent
//! is shown as `/resume` shows a session, its parent kept aside: Esc (with
//! nothing running) or Ctrl+X ← (`focus_pane_left`) returns to it.

mod picker;

use hanafubuki_agent::{AgentSubagentState, AgentSubagentStatus};
use ratatui::style::Stylize;
use ratatui::text::Line;

use super::{CommandContext, CommandReply, CommandSpec};
use crate::agent_bridge::{SubagentsCall, SubagentsReply};
use crate::app_event::AgentCommand;
use crate::history_cell::{self, PlainHistoryCell};
use crate::text_formatting::truncate_text;
use picker::SubagentsPicker;

/// How much of a subagent's latest answer the status cell shows.
const ANSWER_PREVIEW_CHARS: usize = 160;

pub(crate) static SUBAGENTS: CommandSpec = CommandSpec {
    name: "subagents",
    aliases: &[],
    description: "show the subagents' status and switch to one",
    inline_args: false,
    during_task: true,
    needs_session: false,
    dispatch: |context, _args| {
        context.agent_command(AgentCommand::Subagents(SubagentsCall::List));
    },
};

/// The list goes into the transcript and opens the picker; later
/// statuses go to the open picker.
pub(crate) fn on_reply(context: &mut CommandContext<'_>, reply: SubagentsReply) {
    match reply {
        SubagentsReply::Listed(statuses) => {
            context.add_to_history(status_cell(&statuses));
            if !statuses.is_empty() {
                let picker = SubagentsPicker::new(statuses, context.app_event_tx());
                context.show_view(Box::new(picker));
            }
        }
        SubagentsReply::Changed(statuses) => {
            context.offer_reply(CommandReply::Subagents(statuses));
        }
        SubagentsReply::Interrupted => context.add_to_history(history_cell::new_info_event(
            "The subagent was interrupted.".to_string(),
            None,
        )),
        SubagentsReply::Failed(error) => context.add_to_history(history_cell::new_error_event(
            format!("/subagents: {error}"),
        )),
    }
}

/// The status feed: each subagent's title and status, and the start of
/// its latest answer.
pub(crate) fn status_cell(statuses: &[AgentSubagentStatus]) -> PlainHistoryCell {
    let mut lines: Vec<Line<'static>> = vec![
        "/subagents".magenta().into(),
        "Subagents".bold().into(),
        "".into(),
    ];
    if statuses.is_empty() {
        lines.push("  • No subagents in this conversation.".italic().into());
        return PlainHistoryCell::new(lines);
    }
    for status in statuses {
        lines.push(Line::from(vec![
            "  • ".dim(),
            status.title.clone().bold(),
            "  ".into(),
            state_span(status.status),
        ]));
        let answer = status.last_answer.as_deref().map_or_else(
            || "No answer yet.".dim().italic(),
            |answer| truncate_text(&single_line(answer), ANSWER_PREVIEW_CHARS).into(),
        );
        lines.push(Line::from(vec!["    ".into(), answer]));
    }
    PlainHistoryCell::new(lines)
}

pub(crate) fn state_name(state: AgentSubagentState) -> &'static str {
    match state {
        AgentSubagentState::Running => "running",
        AgentSubagentState::Idle => "idle",
        AgentSubagentState::Failed => "failed",
    }
}

fn state_span(state: AgentSubagentState) -> ratatui::text::Span<'static> {
    match state {
        AgentSubagentState::Running => state_name(state).cyan(),
        AgentSubagentState::Idle => state_name(state).dim(),
        AgentSubagentState::Failed => state_name(state).red(),
    }
}

/// `text` on one line: its line breaks as spaces.
pub(crate) fn single_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests;
