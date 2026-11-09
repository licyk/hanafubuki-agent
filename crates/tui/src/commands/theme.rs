// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/theme`: the syntax highlighting theme, picked with a live preview
//! (`appearance::theme_picker`); the app saves `tui.theme`.

use super::{CommandContext, CommandSpec};
use crate::appearance::theme_picker::ThemePicker;

pub(super) static THEME: CommandSpec = CommandSpec {
    name: "theme",
    aliases: &[],
    description: "choose a syntax highlighting theme",
    inline_args: false,
    during_task: true,
    needs_session: false,
    dispatch,
};

fn dispatch(context: &mut CommandContext<'_>, _args: &str) {
    let picker = ThemePicker::new(context.appearance().theme(), context.app_event_tx());
    context.show_view(Box::new(picker));
}
