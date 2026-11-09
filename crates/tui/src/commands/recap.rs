// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/recap`: a brief catch-up on the conversation by the session's model
//! (the facade's `recap_session`): a line while it is written, then the
//! recap and, when there is one, the next step (Codex's
//! `ThreadRecapHistoryCell`). Codex shows the wait as a live cell that the
//! recap replaces; here it is a line of its own. Without the `recap`
//! plugin, the failure says why.

use hanafubuki_agent::AgentRecap;
use ratatui::style::Stylize;
use ratatui::text::Line;

use super::{CommandContext, CommandSpec};
use crate::agent_bridge::SessionCall;
use crate::history_cell::{self, HistoryCell};
use crate::wrapping::{word_wrap_lines, RtOptions};

/// What the recap's lines are indented by, after its first.
const INDENT: &str = "          ";

pub(super) static RECAP: CommandSpec = CommandSpec {
    name: "recap",
    aliases: &[],
    description: "summarize the current conversation now",
    inline_args: false,
    during_task: false,
    needs_session: false,
    dispatch: |context, _args| {
        context.add_to_history(history_cell::new_info_event(
            "Writing a recap…".to_string(),
            None,
        ));
        context.session_call(SessionCall::Recap);
    },
};

pub(super) fn on_reply(context: &mut CommandContext<'_>, recap: AgentRecap) {
    context.add_to_history(RecapCell(recap));
}

/// The recap: `↳ Recap:` and the summary, then `Next:` and the next step.
#[derive(Debug)]
pub(super) struct RecapCell(AgentRecap);

impl RecapCell {
    fn source_lines(&self) -> Vec<Line<'static>> {
        let mut lines: Vec<Line<'static>> = self
            .0
            .summary
            .lines()
            .map(|line| Line::from(line.to_string()).italic())
            .collect();
        if let Some(next) = &self.0.next_action {
            lines.push(Line::from(vec![
                "Next: ".bold().cyan(),
                next.clone().into(),
            ]));
        }
        lines
    }
}

impl HistoryCell for RecapCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let options = RtOptions::new(usize::from(width.max(1)))
            .initial_indent(Line::from(vec!["  ".into(), "↳ ".dim(), "Recap: ".bold()]))
            .subsequent_indent(Line::from(INDENT));
        let mut lines = word_wrap_lines(self.source_lines(), options);
        for line in &mut lines {
            line.style = line.style.dim();
        }
        lines
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        let mut lines = vec![Line::from("Recap:")];
        lines.extend(
            self.0
                .summary
                .lines()
                .map(|line| Line::from(line.to_string())),
        );
        if let Some(next) = &self.0.next_action {
            lines.push(Line::from(format!("Next: {next}")));
        }
        lines
    }
}

#[cfg(test)]
mod tests;
