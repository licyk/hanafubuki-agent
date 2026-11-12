//! An MCP server's OAuth login, a page of its record in the settings
//! center: the address to open as soon as there is one, and the wait for
//! the browser, which Esc (or Ctrl-C) cancels. `c` copies the address,
//! which the center's alternate screen keeps from being selected.

use hanafubuki_agent::McpServerId;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind};
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Widget, Wrap};

use crate::agent_bridge::{McpCall, McpReply};
use crate::app_event::{AgentCommand, AppCommand, AppEvent, AppEventSender, Op};
use crate::bottom_pane::{BottomPaneView, CancellationEvent};
use crate::commands::CommandReply;
use crate::render::renderable::Renderable;

pub(super) struct LoginView {
    server_id: McpServerId,
    tx: AppEventSender,
    url: Option<String>,
    /// Why the login failed; the page stays until Esc.
    error: Option<String>,
    complete: bool,
}

impl LoginView {
    pub(super) fn new(server_id: McpServerId, tx: AppEventSender) -> Self {
        Self {
            server_id,
            tx,
            url: None,
            error: None,
            complete: false,
        }
    }

    /// Ends the wait, unless it ended.
    fn close(&mut self) {
        if self.error.is_none() {
            self.tx.send(AppEvent::Op(Op::Command(AgentCommand::Mcp(
                McpCall::CancelLogin,
            ))));
        }
        self.complete = true;
    }

    fn lines(&self) -> Vec<Line<'static>> {
        let mut lines = vec![
            Line::from(format!("Logging in to {}", self.server_id.as_str()).bold()),
            Line::default(),
        ];
        if let Some(error) = &self.error {
            lines.push(Line::from(format!("The login failed: {error}").red()));
            lines.push(Line::default());
            lines.push(Line::from("esc closes".dim()));
            return lines;
        }
        match &self.url {
            Some(url) => {
                lines.push("Open this address in a browser on this machine:".into());
                lines.push(Line::from(url.clone().cyan()));
                lines.push(Line::default());
                lines.push(Line::from(
                    "Waiting for the browser… c copies the address · esc cancels".dim(),
                ));
            }
            None => lines.push(Line::from("Starting the login… esc cancels".dim())),
        }
        lines
    }
}

impl BottomPaneView for LoginView {
    fn handle_key_event(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        match key.code {
            KeyCode::Esc => self.close(),
            KeyCode::Char('c') => {
                if let Some(url) = &self.url {
                    self.tx
                        .send(AppEvent::Command(AppCommand::Copy(url.clone())));
                }
            }
            _ => {}
        }
    }

    fn take_command_reply(&mut self, reply: CommandReply) -> Option<CommandReply> {
        match reply {
            CommandReply::Mcp(McpReply::LoginUrl { url, .. }) => self.url = Some(url),
            CommandReply::Mcp(McpReply::LoggedIn(_)) => self.complete = true,
            CommandReply::Mcp(McpReply::Failed(error)) => self.error = Some(error),
            reply => return Some(reply),
        }
        None
    }

    fn is_complete(&self) -> bool {
        self.complete
    }

    fn on_ctrl_c(&mut self) -> CancellationEvent {
        self.close();
        CancellationEvent::Handled
    }
}

impl Renderable for LoginView {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        Paragraph::new(self.lines())
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }

    fn desired_height(&self, width: u16) -> u16 {
        Paragraph::new(self.lines())
            .wrap(Wrap { trim: false })
            .line_count(width.max(1)) as u16
    }
}
