// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/theme`: the settings center at the syntax highlighting theme's page,
//! a picker with a live preview (`appearance::theme_picker`); the app
//! saves `tui.theme`.

use super::{CommandContext, CommandSpec};
use crate::appearance::theme_picker::ThemePicker;
use crate::settings_center::{Attach, PageOverride};

pub(super) static THEME: CommandSpec = CommandSpec {
    name: "theme",
    aliases: &[],
    description: "choose a syntax highlighting theme",
    inline_args: false,
    during_task: true,
    needs_session: false,
    dispatch,
};

/// The settings center's page of `/theme`.
pub(crate) static PAGE: PageOverride = PageOverride {
    id: "theme",
    attach: Attach::Setting("tui.theme"),
    command: &THEME,
    open,
};

fn dispatch(context: &mut CommandContext<'_>, _args: &str) {
    super::open_settings(context, PAGE.id);
}

fn open(context: &mut CommandContext<'_>, _record: Option<&str>) {
    let picker = ThemePicker::new(context.appearance().theme(), context.app_event_tx());
    context.show_view(Box::new(picker));
}
