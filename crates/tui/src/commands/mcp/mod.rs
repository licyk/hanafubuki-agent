//! `/mcp`: the settings center's MCP servers, where each is added, edited,
//! connected and deleted, its secrets beside it (MCP secrets); `/mcp
//! verbose` writes the servers and their tools into the transcript; `/mcp
//! login ID` opens that server's OAuth login, a page of its record in the
//! center.

mod login;

use hanafubuki_agent::McpServerId;
use ratatui::style::Stylize;
use ratatui::text::Line;

use super::{CommandContext, CommandReply, CommandSpec};
use crate::agent_bridge::{McpCall, McpReply};
use crate::app::WorkspaceEvent;
use crate::app_event::{AgentCommand, AppEvent};
use crate::history_cell::{self, PlainHistoryCell};
use crate::settings_center::{Attach, PageOverride};
use login::LoginView;

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

/// The settings center's page of an MCP server's OAuth login.
pub(crate) static LOGIN_PAGE: PageOverride = PageOverride {
    id: "mcp-login",
    attach: Attach::Record {
        collection: "mcp",
        label: "Log in",
    },
    command: &MCP,
    open: |context, record| {
        let Some(server_id) = record.and_then(|id| McpServerId::parse(id).ok()) else {
            return;
        };
        context.show_view(Box::new(LoginView::new(
            server_id.clone(),
            context.app_event_tx(),
        )));
        context.agent_command(AgentCommand::Mcp(McpCall::Login(server_id)));
    },
};

fn dispatch(context: &mut CommandContext<'_>, args: &str) {
    let words: Vec<&str> = args.split_whitespace().collect();
    match words.as_slice() {
        [] => super::open_settings(context, "mcp"),
        ["verbose"] => context.agent_command(AgentCommand::Mcp(McpCall::Report)),
        ["login", id] => match McpServerId::parse(*id) {
            Ok(server_id) => context.send(AppEvent::Workspace(WorkspaceEvent::OpenSettings {
                target: Some(LOGIN_PAGE.id.to_string()),
                record: Some(server_id.as_str().to_string()),
            })),
            Err(_) => context.add_to_history(history_cell::new_error_event(format!(
                "`{id}` is not an MCP server id (letters, digits, `-`, `_` and `.`)"
            ))),
        },
        _ => context.add_to_history(history_cell::new_error_event(USAGE.to_string())),
    }
}

/// The report goes into the transcript, and so do the login's address and
/// its end; the rest goes to the open login, and a failure none takes into
/// the transcript.
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
        McpReply::LoggedIn(server_id) => context.add_to_history(history_cell::new_info_event(
            format!("{}: logged in", server_id.as_str()),
            None,
        )),
        McpReply::Failed(_) => {}
    }
    let left = context.offer_reply(CommandReply::Mcp(reply));
    if let Some(CommandReply::Mcp(McpReply::Failed(error))) = left {
        context.add_to_history(history_cell::new_error_event(format!("/mcp: {error}")));
    }
}
