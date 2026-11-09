//! The MCP manager, one view through its steps: the list of servers with
//! each one's phase, tools and last error; a deletion's confirmation; the
//! form; and a login waiting for the browser. Writes name the settings
//! revision the manager last read.

use hanafubuki_agent::{
    McpApplicationSnapshot, McpConnectionPhase, McpCredentialKey, McpServerConfig, McpServerId,
    McpTransportConfig,
};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Widget, Wrap};

use super::form::{self, credential_names};
use crate::agent_bridge::{McpCall, McpReply};
use crate::app_event::{AgentCommand, AppEvent, AppEventSender, Op};
use crate::bottom_pane::{
    BottomPaneView, CancellationEvent, FormEvent, FormView, ListSelectionView, SelectionItem,
    SelectionViewParams,
};
use crate::commands::CommandReply;
use crate::render::renderable::Renderable;

const CONFLICT: &str =
    "The MCP settings changed while this screen was open and were read again; your entries are kept.";

pub(super) struct McpScreen {
    tx: AppEventSender,
    snapshot: Option<McpApplicationSnapshot>,
    stage: Stage,
    /// The login to start once the servers are read (`/mcp login ID`).
    pending_login: Option<McpServerId>,
    complete: bool,
}

enum Stage {
    Loading,
    List(ListSelectionView),
    ConfirmDelete {
        list: ListSelectionView,
        server_id: McpServerId,
    },
    Form {
        form: FormView,
        /// The server edited; `None` adds one.
        server_id: Option<McpServerId>,
        busy: bool,
    },
    Login {
        server_id: McpServerId,
        url: Option<String>,
    },
}

impl McpScreen {
    pub(super) fn new(tx: AppEventSender, login: Option<McpServerId>) -> Self {
        Self {
            tx,
            snapshot: None,
            stage: Stage::Loading,
            pending_login: login,
            complete: false,
        }
    }

    fn send(&self, call: McpCall) {
        self.tx
            .send(AppEvent::Op(Op::Command(AgentCommand::Mcp(call))));
    }

    fn revision(&self) -> u64 {
        self.snapshot
            .as_ref()
            .map_or(0, |snapshot| snapshot.settings.revision)
    }

    fn servers(&self) -> &[McpServerConfig] {
        self.snapshot
            .as_ref()
            .map_or(&[], |snapshot| snapshot.settings.servers.as_slice())
    }

    fn server(&self, server_id: &McpServerId) -> Option<&McpServerConfig> {
        self.servers().iter().find(|server| server.id == *server_id)
    }

    fn show_list(&mut self, notice: Option<Line<'static>>, selected: Option<usize>) {
        let mut items: Vec<SelectionItem> = self
            .servers()
            .iter()
            .map(|server| self.server_item(server))
            .collect();
        items.push(SelectionItem {
            name: "Add a server…".to_string(),
            description: Some("a program to run, or a Streamable HTTP address".to_string()),
            ..Default::default()
        });
        let list = ListSelectionView::new(
            SelectionViewParams {
                title: Some("MCP servers".to_string()),
                subtitle: Some("Changes apply at once; /mcp verbose lists the tools.".to_string()),
                footer_note: notice,
                footer_hint: Some(Line::from(
                    "enter edit · a add · d delete · e enable/disable · c connect · l log in · o log out · esc close",
                )),
                items,
                initial_selected_idx: selected,
                ..Default::default()
            },
            self.tx.clone(),
        );
        self.stage = Stage::List(list);
    }

    /// A server: its transport, phase and tools, and its last error.
    fn server_item(&self, server: &McpServerConfig) -> SelectionItem {
        let status = self.snapshot.as_ref().and_then(|snapshot| {
            snapshot
                .servers
                .iter()
                .find(|status| status.server_id == server.id)
        });
        let transport = match &server.transport {
            McpTransportConfig::Stdio(_) => "stdio",
            McpTransportConfig::StreamableHttp(_) => "http",
        };
        let mut description = match status {
            Some(status) => format!(
                "{transport} · {} · {} tools",
                phase_name(status.phase),
                status.tool_count
            ),
            None => format!("{transport} · not started"),
        };
        if !server.enabled {
            description.push_str(" · disabled");
        }
        if let Some(error) = status.and_then(|status| status.last_error.as_deref()) {
            description.push_str(&format!(" · {error}"));
        }
        SelectionItem {
            name: server.id.as_str().to_string(),
            description: Some(description),
            ..Default::default()
        }
    }

    fn open_form(&mut self, server_id: Option<McpServerId>) {
        let Some(snapshot) = self.snapshot.as_ref() else {
            return;
        };
        let existing = server_id
            .as_ref()
            .and_then(|id| snapshot.settings.server(id));
        let form = form::server_form(existing, snapshot);
        self.stage = Stage::Form {
            form,
            server_id,
            busy: false,
        };
    }

    fn start_login(&mut self, server_id: McpServerId) {
        self.send(McpCall::Login(server_id.clone()));
        self.stage = Stage::Login {
            server_id,
            url: None,
        };
    }

    fn handle_list_key(&mut self, key: KeyEvent) {
        let Stage::List(list) = &self.stage else {
            return;
        };
        let selected = list.selected_index();
        let server = selected
            .and_then(|index| self.servers().get(index))
            .cloned();
        let plain = key.kind != KeyEventKind::Release && key.modifiers == KeyModifiers::NONE;
        let notice = |text: &str| Some(Line::from(text.to_string().dim()));
        match (plain, key.code, server) {
            (true, KeyCode::Char('a'), _) => self.open_form(None),
            (true, KeyCode::Char('d'), Some(server)) => self.confirm_delete(server.id),
            (true, KeyCode::Char('e'), Some(mut server)) => {
                server.enabled = !server.enabled;
                self.send(McpCall::Write {
                    expected_revision: self.revision(),
                    server: Box::new(server),
                    secrets: Vec::new(),
                    dropped: Vec::new(),
                });
                self.show_list(notice("Saving…"), selected);
            }
            (true, KeyCode::Char('c'), Some(server)) => {
                let ready = self.snapshot.as_ref().is_some_and(|snapshot| {
                    snapshot.servers.iter().any(|status| {
                        status.server_id == server.id
                            && matches!(
                                status.phase,
                                McpConnectionPhase::Ready | McpConnectionPhase::Refreshing
                            )
                    })
                });
                self.send(if ready {
                    McpCall::Refresh(server.id)
                } else {
                    McpCall::Connect(server.id)
                });
                self.show_list(notice("Connecting…"), selected);
            }
            (true, KeyCode::Char('l'), Some(server)) => self.start_login(server.id),
            (true, KeyCode::Char('o'), Some(server)) => {
                self.send(McpCall::Logout(server.id));
                self.show_list(notice("Logging out…"), selected);
            }
            _ => {
                let Stage::List(list) = &mut self.stage else {
                    return;
                };
                list.handle_key_event(key);
                if let Some(index) = list.take_last_selected_index() {
                    let server_id = self.servers().get(index).map(|server| server.id.clone());
                    self.open_form(server_id);
                } else if list.is_complete() {
                    self.complete = true;
                }
            }
        }
    }

    fn confirm_delete(&mut self, server_id: McpServerId) {
        let list = ListSelectionView::new(
            SelectionViewParams {
                title: Some(format!("Delete the MCP server {}?", server_id.as_str())),
                subtitle: Some("Its secrets that no other server uses go too.".to_string()),
                items: vec![
                    SelectionItem {
                        name: "Delete".to_string(),
                        ..Default::default()
                    },
                    SelectionItem {
                        name: "Keep it".to_string(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
            self.tx.clone(),
        );
        self.stage = Stage::ConfirmDelete { list, server_id };
    }

    fn handle_confirm_key(&mut self, key: KeyEvent) {
        let Stage::ConfirmDelete { list, server_id } = &mut self.stage else {
            return;
        };
        list.handle_key_event(key);
        let accepted = list.take_last_selected_index();
        let done = accepted.is_some() || list.is_complete();
        let server_id = server_id.clone();
        if accepted == Some(0) {
            let dropped: Vec<McpCredentialKey> = self
                .server(&server_id)
                .map(|server| credential_names(server).into_values().collect())
                .unwrap_or_default();
            self.send(McpCall::Delete {
                expected_revision: self.revision(),
                server_id,
                dropped,
            });
            self.show_list(Some("Deleting…".dim().into()), None);
        } else if done {
            self.show_list(None, None);
        }
    }

    fn handle_form_key(&mut self, key: KeyEvent) {
        let expected_revision = self.revision();
        let Stage::Form {
            form,
            server_id,
            busy,
        } = &mut self.stage
        else {
            return;
        };
        match form.handle_key(key) {
            FormEvent::Cancel => self.show_list(None, None),
            FormEvent::Action(_) if !*busy => {
                let Some(snapshot) = self.snapshot.as_ref() else {
                    return;
                };
                let existing = server_id
                    .as_ref()
                    .and_then(|id| snapshot.settings.server(id));
                let Some(write) = form::server_write(form, existing, snapshot) else {
                    return;
                };
                if existing.is_none() && snapshot.settings.server(&write.server.id).is_some() {
                    form.set_error(form::ID, "A server with this id exists.");
                    return;
                }
                *busy = true;
                form.set_status(vec!["Saving…".dim().into()]);
                let call = McpCall::Write {
                    expected_revision,
                    server: Box::new(write.server),
                    secrets: write.secrets,
                    dropped: write.dropped,
                };
                self.send(call);
            }
            _ => {}
        }
    }

    fn reply(&mut self, reply: McpReply) {
        match reply {
            McpReply::Report(_) => {}
            McpReply::Loaded(snapshot) => {
                self.snapshot = Some(snapshot);
                if !matches!(self.stage, Stage::Loading | Stage::List(_)) {
                    return;
                }
                match self.pending_login.take() {
                    Some(server_id) if self.server(&server_id).is_some() => {
                        self.start_login(server_id)
                    }
                    Some(server_id) => self.show_list(
                        Some(
                            format!("✗ There is no MCP server {}.", server_id.as_str())
                                .red()
                                .into(),
                        ),
                        None,
                    ),
                    None => self.show_list(None, None),
                }
            }
            McpReply::Changed(snapshot) => {
                let notice =
                    matches!(self.stage, Stage::Form { .. }).then(|| Line::from("Saved.".green()));
                let selected = self.list_selection();
                self.snapshot = Some(snapshot);
                if !matches!(self.stage, Stage::Login { .. }) {
                    self.show_list(notice, selected);
                }
            }
            McpReply::LoginUrl { server_id, url } => {
                if let Stage::Login {
                    server_id: waiting,
                    url: shown,
                } = &mut self.stage
                {
                    if *waiting == server_id {
                        *shown = Some(url);
                    }
                }
            }
            McpReply::LoggedIn { snapshot, .. } => {
                self.snapshot = Some(snapshot);
                if matches!(self.stage, Stage::Login { .. } | Stage::List(_)) {
                    self.show_list(Some("Logged in.".green().into()), None);
                }
            }
            McpReply::Failed { error, reloaded } => self.failed(error, reloaded),
        }
    }

    fn failed(&mut self, error: String, reloaded: Option<McpApplicationSnapshot>) {
        let conflict = reloaded.is_some();
        if let Some(snapshot) = reloaded {
            self.snapshot = Some(snapshot);
        }
        let message = if conflict {
            format!("{CONFLICT} Try again. ({error})")
        } else {
            error
        };
        match &mut self.stage {
            Stage::Form { form, busy, .. } => {
                *busy = false;
                form.set_status(vec![format!("✗ {message}").red().into()]);
            }
            _ if self.snapshot.is_none() => {
                self.tx.send(AppEvent::InsertHistoryCell(Box::new(
                    crate::history_cell::new_error_event(format!("/mcp: {message}")),
                )));
                self.complete = true;
            }
            _ => {
                let selected = self.list_selection();
                self.show_list(Some(format!("✗ {message}").red().into()), selected)
            }
        }
    }

    /// The row the list highlights, to keep it when the list is rebuilt.
    fn list_selection(&self) -> Option<usize> {
        match &self.stage {
            Stage::List(list) => list.selected_index(),
            _ => None,
        }
    }

    /// Esc in the login: it is cancelled, and the list shows why it ended.
    fn cancel_login(&mut self) {
        self.send(McpCall::CancelLogin);
        self.show_list(Some("Cancelling the login…".dim().into()), None);
    }
}

impl BottomPaneView for McpScreen {
    fn handle_key_event(&mut self, key: KeyEvent) {
        match &self.stage {
            Stage::Loading if key.code == KeyCode::Esc => self.complete = true,
            Stage::Loading => {}
            Stage::List(_) => self.handle_list_key(key),
            Stage::ConfirmDelete { .. } => self.handle_confirm_key(key),
            Stage::Form { .. } => self.handle_form_key(key),
            Stage::Login { .. } if key.code == KeyCode::Esc => self.cancel_login(),
            Stage::Login { .. } => {}
        }
    }

    fn handle_paste(&mut self, pasted: String) -> bool {
        match &mut self.stage {
            Stage::Form { form, .. } => form.handle_paste(&pasted),
            _ => false,
        }
    }

    fn take_command_reply(&mut self, reply: CommandReply) -> Option<CommandReply> {
        match reply {
            CommandReply::Mcp(reply) => {
                self.reply(reply);
                None
            }
            reply => Some(reply),
        }
    }

    fn is_complete(&self) -> bool {
        self.complete
    }

    fn on_ctrl_c(&mut self) -> CancellationEvent {
        if matches!(self.stage, Stage::Login { .. }) {
            self.send(McpCall::CancelLogin);
        }
        self.complete = true;
        CancellationEvent::Handled
    }
}

impl Renderable for McpScreen {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        match &self.stage {
            Stage::List(list) | Stage::ConfirmDelete { list, .. } => list.render(area, buf),
            Stage::Form { form, .. } => form.render(area, buf),
            Stage::Loading | Stage::Login { .. } => Paragraph::new(self.message())
                .wrap(Wrap { trim: false })
                .render(area, buf),
        }
    }

    fn desired_height(&self, width: u16) -> u16 {
        match &self.stage {
            Stage::List(list) | Stage::ConfirmDelete { list, .. } => list.desired_height(width),
            Stage::Form { form, .. } => form.desired_height(width),
            Stage::Loading | Stage::Login { .. } => Paragraph::new(self.message())
                .wrap(Wrap { trim: false })
                .line_count(width.max(1))
                as u16,
        }
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        match &self.stage {
            Stage::Form { form, .. } => form.cursor_pos(area),
            _ => None,
        }
    }
}

impl McpScreen {
    /// What the screen says while it reads or waits for the browser.
    fn message(&self) -> Vec<Line<'static>> {
        match &self.stage {
            Stage::Login { server_id, url } => {
                let mut lines = vec![
                    Line::from(format!("Logging in to {}", server_id.as_str()).bold()),
                    Line::default(),
                ];
                match url {
                    Some(url) => {
                        lines.push("Open this address in a browser on this machine:".into());
                        lines.push(Line::from(url.clone().cyan()));
                        lines.push(Line::default());
                        lines.push(Line::from("Waiting for the browser… esc cancels".dim()));
                    }
                    None => lines.push(Line::from("Starting the login… esc cancels".dim())),
                }
                lines
            }
            _ => vec![
                Line::from("MCP servers".bold()),
                Line::from("Reading the servers…".dim()),
            ],
        }
    }
}

/// A connection phase as it is serialized (`ready`).
fn phase_name(phase: McpConnectionPhase) -> &'static str {
    match phase {
        McpConnectionPhase::Disabled => "disabled",
        McpConnectionPhase::Starting => "starting",
        McpConnectionPhase::Ready => "ready",
        McpConnectionPhase::Refreshing => "refreshing",
        McpConnectionPhase::Stopping => "stopping",
        McpConnectionPhase::Failed => "failed",
    }
}
