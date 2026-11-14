// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/ps` lists the session's background terminals, each with its command,
//! its state, its directory and its latest output (Codex's
//! `new_unified_exec_processes_output`); `/stop` (or `/clean`) stops them
//! all after a confirmation.

use hanafubuki_agent::AgentBackgroundTerminalSnapshot;
use ratatui::style::Stylize;
use ratatui::text::Line;

use super::confirm::{confirm, Confirmation};
use super::{CommandContext, CommandSpec};
use crate::agent_bridge::{SessionCall, SessionReply};
use crate::history_cell::{self, PlainHistoryCell};

/// How many of a terminal's latest output lines are shown.
const OUTPUT_LINES: usize = 3;

pub(super) static PS: CommandSpec = CommandSpec {
    name: "ps",
    aliases: &[],
    description: "list background terminals",
    inline_args: false,
    during_task: true,
    needs_session: false,
    dispatch: |context, _args| context.session_call(SessionCall::Terminals),
};

pub(super) static STOP: CommandSpec = CommandSpec {
    name: "stop",
    aliases: &["clean"],
    description: "stop all background terminals",
    inline_args: false,
    during_task: true,
    needs_session: false,
    dispatch: |context, _args| {
        confirm(
            context,
            Confirmation {
                title: "Stop all background terminals?",
                subtitle: "Every background terminal of this session is stopped and its owner is notified.",
                no: "No, keep them running",
                yes: "Yes, stop them",
                yes_description: "Stop every background terminal now",
                call: SessionCall::StopTerminals,
            },
        )
    },
};

pub(super) fn on_reply(context: &mut CommandContext<'_>, reply: SessionReply) {
    match reply {
        SessionReply::Terminals(terminals) => {
            context.add_to_history(PlainHistoryCell::new(terminal_lines(&terminals)))
        }
        SessionReply::TerminalsStopped { stopped, failed } => {
            let noun = if stopped == 1 {
                "terminal"
            } else {
                "terminals"
            };
            context.add_to_history(history_cell::new_info_event(
                format!("Stopped {stopped} background {noun}"),
                None,
            ));
            for failure in failed {
                context.add_to_history(history_cell::new_error_event(format!(
                    "/stop: a background terminal kept running: {failure}"
                )));
            }
        }
        _ => {}
    }
}

/// `/ps`'s cell: one entry per terminal, or a line saying there is none.
pub(super) fn terminal_lines(terminals: &[AgentBackgroundTerminalSnapshot]) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = vec!["/ps".magenta().into()];
    if terminals.is_empty() {
        lines.push("  • No background terminals.".italic().into());
        return lines;
    }
    for terminal in terminals {
        let mut state = format!("{:?}", terminal.status).to_lowercase();
        if let Some(code) = terminal.exit_code {
            state.push_str(&format!(", exit code {code}"));
        }
        lines.push(Line::from(vec![
            "  • ".dim(),
            terminal.command.clone().cyan(),
            format!("  ({state})").dim(),
        ]));
        lines.push(Line::from(format!("    in {}", terminal.cwd).dim()));
        let skip = terminal.recent_output.len().saturating_sub(OUTPUT_LINES);
        for (index, output) in terminal.recent_output.iter().skip(skip).enumerate() {
            let prefix = if index == 0 { "    ↳ " } else { "      " };
            lines.push(Line::from(vec![prefix.dim(), output.clone().dim()]));
        }
    }
    lines
}
