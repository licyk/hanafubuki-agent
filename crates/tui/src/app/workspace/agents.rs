// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The command center in the app: open over the whole screen (the
//! alternate screen in scrollback mode), its pages, children and approvals
//! read as it asks, and its actions carried out on the main conversation
//! (`app/agents_overview_actions.rs`). Opening, starting or forking a
//! session moves the main conversation, so they wait while its turn runs;
//! a child opens beside its parent, as `/subagents` shows it. The grouping
//! chosen is written to `tui.agentsGrouping`, quietly.

use std::io::{self, Write};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use hanafubuki_agent::{AgentAgentsGrouping, AgentSessionId, AgentSessionSummary};
use ratatui::backend::Backend;
use ratatui::crossterm::event::KeyEvent;
use serde_json::json;

use super::{Focus, SecondKind};
use crate::agent_bridge::SessionCall;
use crate::agents_center::{AgentsCenter, CenterAction};
use crate::app::App;
use crate::app_event::{AgentCommand, Op};
use crate::resume_picker::ArchiveRequest;
use crate::text_formatting::capitalize_first;
use crate::tui::Tui;

const MAIN_BUSY: &str = "A turn runs in the main conversation; interrupt it or wait for it first.";

impl App {
    pub(super) fn open_agents_center<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
    ) {
        if self.workspace.agents.is_some() {
            return;
        }
        self.enter_alt_screen(tui, false);
        let grouping = self.chat_widget.preferences().agents_grouping;
        self.workspace.agents = Some(AgentsCenter::new(self.main_session_id(), grouping));
        self.load_center_pages();
    }

    /// The center's key, while it is open: `false` when it is not.
    pub(crate) fn handle_agents_key<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        key: KeyEvent,
    ) -> bool {
        let Some(center) = self.workspace.agents.as_mut() else {
            return false;
        };
        let action = center.handle_key(key);
        if let Some(action) = action {
            self.run_center_action(tui, action);
        }
        self.load_center_pages();
        self.schedule_frame(Instant::now());
        true
    }

    /// Draws the center over the screen.
    pub(crate) fn draw_agents_center<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
    ) -> io::Result<bool> {
        let Some(center) = self.workspace.agents.as_mut() else {
            return Ok(false);
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis());
        tui.draw(u16::MAX, |frame| {
            let area = frame.area();
            center.render(area, frame.buffer_mut(), now);
        })?;
        // The list's height is known once it is drawn.
        if center.page_request().is_some() {
            self.load_center_pages();
            self.schedule_frame(Instant::now());
        }
        Ok(true)
    }

    fn load_center_pages(&mut self) {
        let Some(center) = self.workspace.agents.as_mut() else {
            return;
        };
        while let Some(request) = center.page_request() {
            center.ingest_page(self.bridge.list_sessions(request));
        }
        for (parent, request) in center.children_wanted() {
            center.ingest_children(parent, self.bridge.list_sessions(request));
        }
        for session_id in center.approvals_wanted() {
            center.ingest_approval(session_id, self.bridge.waits_for_approval(session_id));
        }
    }

    fn run_center_action<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        action: CenterAction,
    ) {
        let done = match action {
            CenterAction::Close => {
                self.close_agents_center(tui);
                return;
            }
            CenterAction::Open(summary) => match self.open_listed(tui, *summary) {
                Ok(()) => return,
                Err(error) => Err(error),
            },
            CenterAction::New => match self.main_moves(tui, None) {
                Ok(()) => {
                    self.submit_main(Op::Command(AgentCommand::NewSession));
                    return;
                }
                Err(error) => Err(error),
            },
            CenterAction::NewWorktree => match self.new_worktree(tui) {
                Ok(()) => return,
                Err(error) => Err(error),
            },
            CenterAction::Fork(session_id) => self.fork_listed(tui, session_id),
            CenterAction::Rename { session_id, title } => self
                .bridge
                .rename_listed(session_id, title)
                .map(|()| "Session renamed".to_string()),
            CenterAction::SetArchived {
                session_id,
                archive,
            } => self.archive_listed(session_id, archive),
            CenterAction::Delete(session_id) => self
                .bridge
                .delete_listed(session_id)
                .map(|()| "Session deleted".to_string()),
            CenterAction::Stop(session_id) => self
                .bridge
                .interrupt_listed(session_id)
                .map(|()| "Interrupting…".to_string()),
            CenterAction::Grouping(grouping) => {
                self.save_agents_grouping(grouping);
                return;
            }
        };
        let current = self.main_session_id();
        if let Some(center) = self.workspace.agents.as_mut() {
            match done {
                Ok(notice) => {
                    center.reload(current);
                    center.set_notice(notice);
                }
                Err(error) => center.set_notice(error),
            }
        }
    }

    fn close_agents_center<B: Backend<Error = io::Error> + Write>(&mut self, tui: &mut Tui<B>) {
        self.workspace.agents = None;
        self.leave_alt_screen(tui);
    }

    /// Shows the listed session: a top-level one as the main conversation,
    /// a child beside its parent.
    fn open_listed<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        summary: AgentSessionSummary,
    ) -> Result<(), String> {
        let shown = summary.parent_session_id.unwrap_or(summary.session_id);
        self.main_moves(tui, Some(shown))?;
        self.close_agents_center(tui);
        if self.bridge.session_id() != Some(shown) {
            self.submit_main(Op::Command(AgentCommand::Resume(shown)));
        }
        if summary.parent_session_id.is_some() {
            self.open_child(tui, summary.session_id);
        }
        Ok(())
    }

    /// The main conversation is about to show another session (`target`,
    /// or a new one): refused while its turn runs; else it is focused,
    /// and a second conversation that belongs to the session it leaves is
    /// let go.
    fn main_moves<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        target: Option<AgentSessionId>,
    ) -> Result<(), String> {
        let main_busy = match self.workspace.second.as_ref() {
            Some(second) if second.focus == Focus::Second => {
                second.parked.chat_widget.is_task_running()
            }
            _ => self.chat_widget.is_task_running(),
        };
        let leaving = target != self.main_session_id();
        if main_busy && leaving {
            return Err(MAIN_BUSY.to_string());
        }
        match self.workspace.second.as_ref().map(|second| &second.kind) {
            Some(SecondKind::Side { .. }) if leaving => self.close_second(tui),
            Some(SecondKind::Child { .. }) => self.leave_second(tui),
            _ => self.set_focus(tui, Focus::Main),
        }
        Ok(())
    }

    /// Hands `op` to the main conversation's bridge (focused by now).
    fn submit_main(&mut self, op: Op) {
        if let Some(update) = self.bridge.submit(op) {
            self.show_update(update);
        }
    }

    fn fork_listed<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        session_id: AgentSessionId,
    ) -> Result<String, String> {
        self.main_moves(tui, None)?;
        let fork = self.bridge.fork_listed(session_id)?;
        self.close_agents_center(tui);
        self.submit_main(Op::Command(AgentCommand::Resume(fork)));
        Ok(String::new())
    }

    fn archive_listed(
        &mut self,
        session_id: AgentSessionId,
        archive: bool,
    ) -> Result<String, String> {
        self.bridge
            .set_archived(ArchiveRequest {
                session_id,
                archive,
            })
            .map(|()| {
                if archive {
                    "Session archived".to_string()
                } else {
                    "Session restored".to_string()
                }
            })
            .map_err(|error| format!("The session was not changed: {error}"))
    }

    /// `w`: a new session in a new worktree, as `/worktree` starts one;
    /// where none can be made, the center says why.
    fn new_worktree<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
    ) -> Result<(), String> {
        if let Some(reason) = self.bridge.worktrees_unavailable() {
            return Err(capitalize_first(&reason));
        }
        self.main_moves(tui, None)?;
        self.close_agents_center(tui);
        let call = SessionCall::EnterWorktree {
            continued: false,
            name: None,
        };
        self.submit_main(Op::Command(AgentCommand::Session(call)));
        Ok(())
    }

    /// `g`: the grouping written to `tui.agentsGrouping` with no message;
    /// a failure is said in the center.
    fn save_agents_grouping(&mut self, grouping: AgentAgentsGrouping) {
        let value = json!(grouping);
        self.write_tui_settings(vec![("tui.agentsGrouping", value)], String::new());
    }
}
