// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/tui`: the transcript's mode for the next launch, Scrollback (the
//! terminal's) or Fullscreen (owned); the app saves
//! `tui.fullscreenTranscript`. The running interface keeps its mode.

use super::{CommandContext, CommandSpec};
use crate::app_event::AppEvent;
use crate::bottom_pane::{SelectionItem, SelectionViewParams};

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
    let current = context.fullscreen_preference();
    let items = [
        (
            false,
            "Scrollback",
            "the terminal's own scrollback, selection and copy",
        ),
        (
            true,
            "Fullscreen",
            "scroll, select and search in the interface",
        ),
    ]
    .into_iter()
    .map(|(enabled, name, description)| SelectionItem {
        name: name.to_string(),
        description: Some(description.to_string()),
        is_current: enabled == current,
        actions: vec![Box::new(move |tx| {
            tx.send(AppEvent::SetFullscreenTranscript(enabled))
        })],
        dismiss_on_select: true,
        ..Default::default()
    })
    .collect();
    context.show_selection_view(SelectionViewParams {
        title: Some("Interface mode for the next launch".to_string()),
        subtitle: Some(
            "Applies when the interface starts again; --no-alt-screen and \
             tui.alternateScreen still apply."
                .to_string(),
        ),
        items,
        ..Default::default()
    });
}
