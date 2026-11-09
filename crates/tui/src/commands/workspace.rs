// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! The commands over the workspace's files: `/diff` and `/mention`.

use super::CommandSpec;
use crate::app_event::AppEvent;

pub(super) static DIFF: CommandSpec = CommandSpec {
    name: "diff",
    aliases: &[],
    description: "show git diff (including untracked files)",
    inline_args: false,
    during_task: true,
    needs_session: false,
    dispatch: |context, _args| context.send(AppEvent::Diff),
};

pub(super) static MENTION: CommandSpec = CommandSpec {
    name: "mention",
    aliases: &[],
    description: "mention a file",
    inline_args: false,
    during_task: true,
    needs_session: false,
    dispatch: |context, _args| context.insert_str("@"),
};
