// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/archive` and `/delete`: after a confirmation, the session is archived
//! (it can be restored from the resume picker) or deleted with its child
//! sessions, and the interface shows no session, as after `/new`.

use super::confirm::{confirm, Confirmation};
use super::{CommandContext, CommandSpec};
use crate::agent_bridge::{SessionCall, SessionReply};
use crate::history_cell;

pub(super) static ARCHIVE: CommandSpec = CommandSpec {
    name: "archive",
    aliases: &[],
    description: "archive this session",
    inline_args: false,
    during_task: false,
    needs_session: false,
    dispatch: |context, _args| {
        confirm(
            context,
            Confirmation {
                title: "Archive this session?",
                subtitle:
                    "It leaves the list; the resume picker's archived sessions (Tab) restore it.",
                no: "No, don't archive",
                yes: "Yes, archive",
                yes_description: "Archive this session and start a new one",
                call: SessionCall::Archive,
            },
        )
    },
};

pub(super) static DELETE: CommandSpec = CommandSpec {
    name: "delete",
    aliases: &[],
    description: "permanently delete this session",
    inline_args: false,
    during_task: false,
    needs_session: false,
    dispatch: |context, _args| {
        confirm(
            context,
            Confirmation {
                title: "Delete this session?",
                subtitle: "This cannot be undone; its child sessions are deleted too.",
                no: "No, keep this session",
                yes: "Yes, delete",
                yes_description: "Permanently delete this session and start a new one",
                call: SessionCall::Delete,
            },
        )
    },
};

pub(super) fn on_reply(context: &mut CommandContext<'_>, reply: SessionReply) {
    let message = match reply {
        SessionReply::Archived => "Session archived",
        SessionReply::Deleted => "Session deleted",
        _ => return,
    };
    context.add_to_history(history_cell::new_info_event(message.to_string(), None));
}
