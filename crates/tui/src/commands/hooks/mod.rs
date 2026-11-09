// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/hooks`: the browser of the configured hooks (Codex's hooks browser):
//! by event, each hook with its matcher, command, source, trust and
//! whether it is on; it turns hooks on and off, trusts one or every one
//! waiting, and lists what in the configuration was skipped. At start,
//! hooks waiting to be trusted are reported by a line and a warning, not by
//! Codex's blocking prompt (`startup_hooks_review.rs`).

mod browser;

use super::{CommandContext, CommandReply, CommandSpec};
use crate::agent_bridge::{HooksCall, HooksReply};
use crate::app_event::AgentCommand;
use crate::history_cell;
use browser::HooksBrowser;

pub(crate) static HOOKS: CommandSpec = CommandSpec {
    name: "hooks",
    aliases: &[],
    description: "review, trust and turn on or off the configured hooks",
    inline_args: false,
    during_task: true,
    needs_session: false,
    dispatch: |context, _args| {
        context.show_view(Box::new(HooksBrowser::new(context.app_event_tx())));
        context.agent_command(AgentCommand::Hooks(HooksCall::Load));
    },
};

/// The warning the chat keeps for hooks found waiting at start.
pub(crate) fn startup_warning(reply: &HooksReply) -> Option<String> {
    match reply {
        HooksReply::NeedReview(count) => Some(need_review(*count)),
        _ => None,
    }
}

/// The start's line goes into the transcript; the rest goes to the open
/// browser, and a failure none takes into the transcript.
pub(crate) fn on_reply(context: &mut CommandContext<'_>, reply: HooksReply) {
    if let HooksReply::NeedReview(count) = reply {
        return context.add_to_history(history_cell::new_info_event(need_review(count), None));
    }
    if let Some(CommandReply::Hooks(HooksReply::Failed(error))) =
        context.offer_reply(CommandReply::Hooks(reply))
    {
        context.add_to_history(history_cell::new_error_event(format!("/hooks: {error}")));
    }
}

fn need_review(count: usize) -> String {
    match count {
        1 => "1 hook needs review: /hooks".to_string(),
        count => format!("{count} hooks need review: /hooks"),
    }
}

#[cfg(test)]
mod tests;
