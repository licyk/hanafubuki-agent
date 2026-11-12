// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/config [target]` opens the settings center (`settings_center`) at a
//! tab, a section, a collection or a page.

use super::{CommandContext, CommandSpec};
use crate::app::WorkspaceEvent;
use crate::app_event::AppEvent;

pub(crate) static CONFIG: CommandSpec = CommandSpec {
    name: "config",
    aliases: &[],
    description: "view and change every Agent setting",
    inline_args: true,
    during_task: true,
    needs_session: false,
    dispatch,
};

fn dispatch(context: &mut CommandContext<'_>, args: &str) {
    let target = args.trim();
    context.send(AppEvent::Workspace(WorkspaceEvent::OpenSettings {
        target: (!target.is_empty()).then(|| target.to_string()),
        record: None,
    }));
}
