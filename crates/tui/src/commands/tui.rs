// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/tui`: the settings center at the terminal interface's section, where
//! `tui.fullscreenTranscript` picks Scrollback or Fullscreen for the next
//! launch. The running interface keeps its mode.

use super::{CommandContext, CommandSpec};

pub(super) static TUI: CommandSpec = CommandSpec {
    name: "tui",
    aliases: &[],
    description: "choose the interface mode for the next launch",
    inline_args: false,
    during_task: true,
    needs_session: false,
    dispatch,
};

fn dispatch(context: &mut CommandContext<'_>, _args: &str) {
    super::open_settings(context, "tui");
}
