// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/providers`: the model Providers, listed with the default marked, each
//! added, edited, tested, enabled, deleted or made the default, and after a
//! save the model new sessions run (Codex's `app/provider.rs`). The same
//! screen is the first-run setup: with no default Provider or no model it
//! opens at the start, and again when a message is sent before it is done.
//!
//! The screen is one view; what the Agent answers comes back to it through
//! the chat (`on_reply`).

mod form;
mod screen;

use super::{CommandContext, CommandReply, CommandSpec};
use crate::agent_bridge::{ProviderCall, ProviderReply};
use crate::app_event::AgentCommand;
use crate::history_cell;
use screen::ProvidersScreen;

pub(crate) static PROVIDERS: CommandSpec = CommandSpec {
    name: "providers",
    aliases: &[],
    description: "add, edit and test model Providers, and choose the default model",
    inline_args: false,
    during_task: false,
    needs_session: false,
    dispatch: |context, _args| open(context, false),
};

/// Opens the screen on the Providers as they are read; `setup` leads
/// through what is missing before the first message.
pub(crate) fn open(context: &mut CommandContext<'_>, setup: bool) {
    let screen = ProvidersScreen::new(context.app_event_tx(), setup);
    context.show_view(Box::new(screen));
    context.agent_command(AgentCommand::Providers(ProviderCall::Load));
}

/// What the Agent answered goes to the open screen; a failure no screen
/// takes is written into the transcript.
pub(crate) fn on_reply(context: &mut CommandContext<'_>, reply: ProviderReply) {
    let left = context.offer_reply(CommandReply::Providers(reply));
    if let Some(CommandReply::Providers(ProviderReply::Failed { error, .. })) = left {
        context.add_to_history(history_cell::new_error_event(format!(
            "/providers: {error}"
        )));
    }
}

#[cfg(test)]
mod tests;
