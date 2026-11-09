//! `/mcp`: the manager of the MCP servers (a list with each server's
//! state, the form, connecting and logging in); `/mcp verbose` writes the
//! servers and their tools into the transcript; `/mcp login ID` opens the
//! manager on that server's OAuth login.

mod form;
mod screen;

use hanafubuki_agent::McpServerId;
use ratatui::style::Stylize;
use ratatui::text::Line;

use super::{CommandContext, CommandReply, CommandSpec};
use crate::agent_bridge::{McpCall, McpReply};
use crate::app_event::AgentCommand;
use crate::history_cell::{self, PlainHistoryCell};
use screen::McpScreen;

const USAGE: &str = "Usage: /mcp [verbose | login ID]";

pub(crate) static MCP: CommandSpec = CommandSpec {
    name: "mcp",
    aliases: &[],
    description: "manage MCP servers; /mcp verbose lists them with their tools",
    inline_args: true,
    during_task: true,
    needs_session: false,
    dispatch,
};

fn dispatch(context: &mut CommandContext<'_>, args: &str) {
    let words: Vec<&str> = args.split_whitespace().collect();
    match words.as_slice() {
        [] => open(context, None),
        ["verbose"] => context.agent_command(AgentCommand::Mcp(McpCall::Report)),
        ["login", id] => match McpServerId::parse(*id) {
            Ok(server_id) => open(context, Some(server_id)),
            Err(_) => context.add_to_history(history_cell::new_error_event(format!(
                "`{id}` is not an MCP server id (letters, digits, `-`, `_` and `.`)"
            ))),
        },
        _ => context.add_to_history(history_cell::new_error_event(USAGE.to_string())),
    }
}

/// Opens the manager on the servers as they are read, and starts the
/// login of `login` once they are.
fn open(context: &mut CommandContext<'_>, login: Option<McpServerId>) {
    let screen = McpScreen::new(context.app_event_tx(), login);
    context.show_view(Box::new(screen));
    context.agent_command(AgentCommand::Mcp(McpCall::Load));
}

/// The report goes into the transcript, and so do the login's address and
/// its end; the rest goes to the open manager, and a failure none takes
/// into the transcript.
pub(crate) fn on_reply(context: &mut CommandContext<'_>, reply: McpReply) {
    match &reply {
        McpReply::Report(snapshot) => {
            return context.add_to_history(history_cell::new_mcp_output(snapshot));
        }
        // The address on a line of its own, whole, to copy.
        McpReply::LoginUrl { server_id, url } => {
            context.add_to_history(PlainHistoryCell::new(vec![
                Line::from(vec![
                    "• ".dim(),
                    format!(
                        "Open this address in a browser on this machine to log in to {}:",
                        server_id.as_str()
                    )
                    .into(),
                ]),
                Line::from(url.clone().cyan()),
            ]))
        }
        McpReply::LoggedIn { server_id, .. } => context.add_to_history(
            history_cell::new_info_event(format!("{}: logged in", server_id.as_str()), None),
        ),
        _ => {}
    }
    let left = context.offer_reply(CommandReply::Mcp(reply));
    if let Some(CommandReply::Mcp(McpReply::Failed { error, .. })) = left {
        context.add_to_history(history_cell::new_error_event(format!("/mcp: {error}")));
    }
}

#[cfg(test)]
mod tests;
