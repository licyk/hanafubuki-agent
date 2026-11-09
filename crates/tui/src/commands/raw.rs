// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/raw [on|off]`: the transcript as the plain text it was made from,
//! for copying with the terminal's own selection; without an argument it
//! toggles. The app applies it and saves `tui.rawOutput`.

use super::{CommandContext, CommandSpec};
use crate::app_event::AppEvent;
use crate::history_cell;

const USAGE: &str = "Usage: /raw [on|off]";

pub(super) static RAW: CommandSpec = CommandSpec {
    name: "raw",
    aliases: &[],
    description: "toggle raw output for copy-friendly terminal selection",
    inline_args: true,
    during_task: true,
    needs_session: false,
    dispatch,
};

fn dispatch(context: &mut CommandContext<'_>, args: &str) {
    let raw = match args.trim().to_ascii_lowercase().as_str() {
        "" => None,
        "on" => Some(true),
        "off" => Some(false),
        _ => {
            let usage = history_cell::new_error_event(USAGE.to_string());
            context.send(AppEvent::InsertHistoryCell(Box::new(usage)));
            return;
        }
    };
    context.send(AppEvent::SetRawOutput(raw));
}
