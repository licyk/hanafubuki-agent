// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/rename [name]`: a prompt filled with the session's title, which shows
//! the model's suggestion when it arrives (Tab takes it), Codex's
//! `show_rename_prompt`; with a name, the session is renamed at once.

use super::prompt::PromptView;
use super::{send_session_call, CommandContext, CommandReply, CommandSpec};
use crate::agent_bridge::{SessionCall, SessionReply};
use crate::history_cell;

pub(super) static RENAME: CommandSpec = CommandSpec {
    name: "rename",
    aliases: &[],
    description: "rename the current session",
    inline_args: true,
    during_task: false,
    needs_session: true,
    dispatch,
};

fn dispatch(context: &mut CommandContext<'_>, args: &str) {
    let name = args.trim();
    if !name.is_empty() {
        context.session_call(SessionCall::Rename(name.to_string()));
        return;
    }
    let mut view = PromptView::new(
        "Rename session",
        "Type a name and press Enter",
        context.app_event_tx(),
        Box::new(|app_event_tx, name| send_session_call(app_event_tx, SessionCall::Rename(name))),
    )
    .with_replies(take_reply);
    view.set_note(Some("Suggesting a title…".to_string()));
    context.show_view(Box::new(view));
    context.session_call(SessionCall::RenamePrompt);
}

/// The prompt takes the title (unless something is typed already) and the
/// suggestion.
fn take_reply(view: &mut PromptView, reply: CommandReply) -> Option<CommandReply> {
    match reply {
        CommandReply::Session(SessionReply::Title(title)) => {
            if view.text().is_empty() {
                view.set_text(&title.unwrap_or_default());
            }
        }
        CommandReply::Session(SessionReply::TitleSuggestion(suggestion)) => {
            let note = suggestion
                .is_none()
                .then(|| "No title suggestion".to_string());
            view.set_note(note);
            view.set_suggestion(suggestion);
        }
        reply => return Some(reply),
    }
    None
}

pub(super) fn on_reply(context: &mut CommandContext<'_>, reply: SessionReply) {
    match reply {
        SessionReply::Renamed(title) => context.add_to_history(history_cell::new_info_event(
            format!("Session renamed to \"{title}\""),
            None,
        )),
        // The prompt may be closed already: then nothing takes them.
        reply => drop(context.offer_reply(CommandReply::Session(reply))),
    }
}
