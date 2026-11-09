// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/export [path]`: the conversation as Markdown (the facade's
//! `export_session_markdown`), copied to the clipboard or saved into a
//! file, Codex's `chatwidget/transcript_export.rs`. The file is offered at
//! `<first root>/hanafubuki-agent-session-<id>.md`; with a path it is
//! saved there at once. An existing file is not replaced.

use std::path::PathBuf;

use super::prompt::PromptView;
use super::{send_session_call, CommandContext, CommandSpec};
use crate::agent_bridge::{ExportDestination, SessionCall, SessionReply};
use crate::app_event::{AppCommand, AppEvent};
use crate::bottom_pane::{SelectionItem, SelectionViewParams};
use crate::history_cell;

pub(super) static EXPORT: CommandSpec = CommandSpec {
    name: "export",
    aliases: &[],
    description: "export the conversation as Markdown",
    inline_args: true,
    during_task: false,
    needs_session: false,
    dispatch,
};

fn dispatch(context: &mut CommandContext<'_>, args: &str) {
    let path = args.trim();
    if !path.is_empty() {
        let destination = ExportDestination::File(PathBuf::from(path));
        context.session_call(SessionCall::Export(destination));
        return;
    }
    context.show_selection_view(SelectionViewParams {
        title: Some("Export conversation".to_string()),
        subtitle: Some("The whole conversation as Markdown".to_string()),
        items: vec![
            SelectionItem {
                name: "Copy to clipboard".to_string(),
                description: Some("Copy the Markdown".to_string()),
                actions: vec![Box::new(|tx| {
                    let call = SessionCall::Export(ExportDestination::Clipboard);
                    send_session_call(tx, call)
                })],
                dismiss_on_select: true,
                ..Default::default()
            },
            SelectionItem {
                name: "Save to file".to_string(),
                description: Some("Choose a Markdown file".to_string()),
                actions: vec![Box::new(|tx| {
                    send_session_call(tx, SessionCall::ExportPath)
                })],
                dismiss_on_select: true,
                ..Default::default()
            },
        ],
        ..Default::default()
    });
}

pub(super) fn on_reply(context: &mut CommandContext<'_>, reply: SessionReply) {
    match reply {
        SessionReply::ExportPath(path) => {
            let mut view = PromptView::new(
                "Save conversation",
                "Type a file name and press Enter",
                context.app_event_tx(),
                Box::new(|app_event_tx, path| {
                    let destination = ExportDestination::File(PathBuf::from(path));
                    send_session_call(app_event_tx, SessionCall::Export(destination))
                }),
            );
            view.set_text(&path.display().to_string());
            view.set_note(Some(
                "A relative path is read against the first workspace root".to_string(),
            ));
            context.show_view(Box::new(view));
        }
        SessionReply::ExportedText(markdown) => {
            context.send(AppEvent::Command(AppCommand::Copy(markdown)))
        }
        SessionReply::ExportedFile(path) => context.add_to_history(history_cell::new_info_event(
            format!("Saved the conversation to {}", path.display()),
            None,
        )),
        _ => {}
    }
}
