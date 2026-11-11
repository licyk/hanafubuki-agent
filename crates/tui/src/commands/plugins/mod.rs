// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/plugins` (Codex's plugin catalog, `chatwidget/plugins.rs` and
//! `chatwidget/plugin_catalog.rs`): the installed plugins, the
//! marketplaces and their plugins, and the built-in plugins, on three tabs.
//! It installs (from a marketplace or a directory), turns on and off,
//! updates, retries and uninstalls plugins, trusts an installed plugin's
//! hooks, adds, upgrades and removes marketplaces, and turns the built-in
//! plugins on and off or retries them. Codex's remote marketplaces,
//! sharing, apps and their auth are not ported.

mod browser;
mod builtin;
mod code;
mod options;
mod pages;

use super::{CommandContext, CommandReply, CommandSpec};
use crate::agent_bridge::{PluginsCall, PluginsReply};
use crate::app_event::AgentCommand;
use crate::history_cell;
use browser::PluginsBrowser;

pub(crate) static PLUGINS: CommandSpec = CommandSpec {
    name: "plugins",
    aliases: &[],
    description: "install, turn on or off, update and uninstall plugins; manage marketplaces",
    inline_args: false,
    during_task: true,
    needs_session: false,
    dispatch: |context, _args| {
        context.show_view(Box::new(PluginsBrowser::new(context.app_event_tx())));
        context.agent_command(AgentCommand::Plugins(PluginsCall::Load));
    },
};

/// The answer goes to the open browser; a failure none takes goes into the
/// transcript.
pub(crate) fn on_reply(context: &mut CommandContext<'_>, reply: PluginsReply) {
    if let Some(CommandReply::Plugins(PluginsReply::Failed { error, .. })) =
        context.offer_reply(CommandReply::Plugins(reply))
    {
        context.add_to_history(history_cell::new_error_event(format!("/plugins: {error}")));
    }
}

#[cfg(test)]
mod tests;
