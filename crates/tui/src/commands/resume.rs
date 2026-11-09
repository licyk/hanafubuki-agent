// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/resume [id|title]`: the resume picker; with an argument, the session
//! with that id, else the one session with that title, else the picker
//! searching for it (`resume_picker::find_session`).

use super::{CommandContext, CommandSpec};
use crate::app_event::{AppCommand, AppEvent};

pub(super) static RESUME: CommandSpec = CommandSpec {
    name: "resume",
    aliases: &[],
    description: "resume a saved chat",
    inline_args: true,
    during_task: false,
    needs_session: false,
    dispatch,
};

fn dispatch(context: &mut CommandContext<'_>, args: &str) {
    let text = args.trim();
    if text.is_empty() {
        context.send(AppEvent::OpenResumePicker);
    } else {
        context.send(AppEvent::Command(AppCommand::ResumeNamed(text.to_string())));
    }
}
