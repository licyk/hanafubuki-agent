// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/title [items|--clear]`: with no argument, the setup view, which saves
//! `tui.terminalTitle`; with items, the terminal title of this run only;
//! `--clear` gives the terminal its own title back for this run (Codex's
//! `chatwidget/status_surface_commands.rs`).

use super::{CommandContext, CommandSpec};
use crate::app_event::AppEvent;
use crate::appearance::items::{SurfaceItem, TerminalTitleItem};
use crate::appearance::setup::terminal_title_setup;
use crate::appearance::{known_ids, AppearanceEvent};
use crate::history_cell;

pub(super) static TITLE: CommandSpec = CommandSpec {
    name: "title",
    aliases: &[],
    description: "configure the terminal title",
    inline_args: true,
    during_task: true,
    needs_session: false,
    dispatch,
};

fn dispatch(context: &mut CommandContext<'_>, args: &str) {
    if args.trim().is_empty() {
        let view = terminal_title_setup(context.appearance(), context.app_event_tx());
        context.show_view(Box::new(view));
        return;
    }
    match parse(args) {
        Ok(event) => context.send(AppEvent::Appearance(event)),
        Err(message) => context.add_to_history(history_cell::new_error_event(message)),
    }
}

/// The items named, in order, or none for `--clear`, for this run.
pub(super) fn parse(args: &str) -> Result<AppearanceEvent, String> {
    let mut items = Vec::new();
    let mut clear = false;
    for token in args.split_whitespace() {
        match token {
            "--clear" => clear = true,
            id => match TerminalTitleItem::parse(id) {
                Some(item) if !items.contains(&item) => items.push(item),
                Some(_) => {}
                None => {
                    return Err(format!(
                        "Unknown terminal-title item '{id}'. The items are: {}. \
                         Usage: /title [items|--clear]",
                        known_ids::<TerminalTitleItem>()
                    ))
                }
            },
        }
    }
    if clear && !items.is_empty() {
        return Err("Cannot combine --clear with terminal-title items.".to_string());
    }
    Ok(AppearanceEvent::TerminalTitle { items, save: false })
}
