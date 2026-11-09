// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/statusline [items] [--colors|--no-colors]`: with no argument, the
//! setup view, which saves `tui.statusLine` and `tui.statusLineColors`;
//! with arguments, the status line of this run only (Codex's
//! `chatwidget/status_surface_commands.rs`).

use super::{CommandContext, CommandSpec};
use crate::app_event::AppEvent;
use crate::appearance::items::{StatusLineItem, SurfaceItem};
use crate::appearance::setup::status_line_setup;
use crate::appearance::{known_ids, AppearanceEvent};
use crate::history_cell;

pub(super) static STATUSLINE: CommandSpec = CommandSpec {
    name: "statusline",
    aliases: &[],
    description: "configure the status line under the composer",
    inline_args: true,
    during_task: true,
    needs_session: false,
    dispatch,
};

fn dispatch(context: &mut CommandContext<'_>, args: &str) {
    if args.trim().is_empty() {
        let view = status_line_setup(context.appearance(), context.app_event_tx());
        context.show_view(Box::new(view));
        return;
    }
    match parse(args) {
        Ok(event) => context.send(AppEvent::Appearance(event)),
        Err(message) => context.add_to_history(history_cell::new_error_event(message)),
    }
}

/// The items named, in order, and the colours switch, for this run.
pub(super) fn parse(args: &str) -> Result<AppearanceEvent, String> {
    let mut items = Vec::new();
    let mut colors = None;
    for token in args.split_whitespace() {
        match token {
            "--colors" => colors = Some(true),
            "--no-colors" => colors = Some(false),
            id => match StatusLineItem::parse(id) {
                Some(item) if !items.contains(&item) => items.push(item),
                Some(_) => {}
                None => {
                    return Err(format!(
                        "Unknown status-line item '{id}'. The items are: {}. \
                         Usage: /statusline [items] [--colors|--no-colors]",
                        known_ids::<StatusLineItem>()
                    ))
                }
            },
        }
    }
    Ok(AppearanceEvent::StatusLine {
        items: (!items.is_empty()).then_some(items),
        colors,
        save: false,
    })
}
