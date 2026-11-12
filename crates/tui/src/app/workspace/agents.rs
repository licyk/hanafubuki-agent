// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The command center in the app: open over the whole screen (the
//! alternate screen in scrollback mode, with the mouse), what it reads
//! asked of its reads' thread and handed back to it, and its actions
//! carried out on the main conversation (`app/agents_overview_actions.rs`).
//! Opening, starting or forking a session moves the main conversation, so
//! they wait while its turn runs; a child opens beside its parent, as
//! `/subagents` shows it. The grouping chosen is written to
//! `tui.agentsGrouping`, quietly; the pins and the hidden sessions, and when
//! the session shown was last shown, to `tui-state.json`.

use std::io::{self, Write};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use hanafubuki_agent::{AgentAgentsGrouping, AgentSessionId, AgentSessionSummary};
use ratatui::backend::Backend;
use ratatui::crossterm::event::{KeyEvent, MouseEvent};
use serde_json::json;

use super::{Focus, SecondKind};
use crate::agent_bridge::{CenterRead, SessionCall};
use crate::agents_center::{AgentsCenter, CenterAction};
use crate::app::App;
use crate::app_event::{AgentCommand, Op};
use crate::resume_picker::ArchiveRequest;
use crate::text_formatting::capitalize_first;
use crate::tui::Tui;
use crate::tui_state::AgentsMarks;

const MAIN_BUSY: &str = "A turn runs in the main conversation; interrupt it or wait for it first.";

impl App {
    pub(super) fn open_agents_center<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
    ) {
        if self.workspace.agents.is_some() {
            return;
        }
        self.enter_alt_screen(tui, true);
        // What the session shown changed so far, you saw.
        self.workspace.shown = None;
        self.note_shown();
        let grouping = self.chat_widget.preferences().agents_grouping;
        let marks = self.tui_state.agents.clone();
        let center = AgentsCenter::new(self.main_session_id(), grouping, marks);
        self.workspace.agents = Some(center);
        let reads = self.bridge.center_reader(self.app_event_tx.untagged());
        self.workspace.agents_reads = Some(reads);
        self.run_center_jobs();
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
        self.run_center_jobs();
        self.schedule_frame(Instant::now());
        true
    }

    /// The pointer, while the center is open: `false` when it is not.
    pub(crate) fn handle_agents_mouse<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        mouse: MouseEvent,
    ) -> bool {
        let Some(center) = self.workspace.agents.as_mut() else {
            return false;
        };
        if let Some(action) = center.handle_mouse(mouse) {
            self.run_center_action(tui, action);
        }
        self.run_center_jobs();
        self.schedule_frame(Instant::now());
        true
    }

    /// What a read of the center found, or its watch saw.
    pub(super) fn center_read(&mut self, read: CenterRead) {
        let Some(center) = self.workspace.agents.as_mut() else {
            return;
        };
        center.ingest(read);
        self.run_center_jobs();
    }

    /// `tui-state.json` was not written: said in the center while it is
    /// open, else in the conversation.
    pub(super) fn tui_state_not_saved(&mut self, error: String) {
        match self.workspace.agents.as_mut() {
            Some(center) => center.set_notice(error),
            None => self.chat_widget.add_error_message(error),
        }
    }

    /// The main conversation's session changed (or the center opens): the
    /// one it showed and the one it shows are seen now, so that only what
    /// changes after marks them unread.
    pub(crate) fn note_shown(&mut self) {
        let shown = self.main_session_id();
        if shown == self.workspace.shown {
            return;
        }
        let now = now_millis();
        let seen = &mut self.tui_state.agents.seen;
        for session_id in [self.workspace.shown, shown].into_iter().flatten() {
            seen.insert(session_id, now);
        }
        self.workspace.shown = shown;
        self.tui_state.save();
    }

    /// The interface ends: the session shown is seen now, written before
    /// the process leaves.
    pub(crate) fn save_state_on_exit(&mut self) {
        if let Some(session_id) = self.main_session_id() {
            self.tui_state.agents.seen.insert(session_id, now_millis());
        }
        let _ = self.tui_state.save_now();
    }

    /// Draws the center over the screen.
    pub(crate) fn draw_agents_center<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
    ) -> io::Result<bool> {
        let Some(center) = self.workspace.agents.as_mut() else {
            return Ok(false);
        };
        let now = now_millis();
        tui.draw(u16::MAX, |frame| {
            let area = frame.area();
            center.render(area, frame.buffer_mut(), now);
        })?;
        // The list's height is known once it is drawn.
        self.run_center_jobs();
        Ok(true)
    }

    /// Hands the reads the center needs to its thread.
    fn run_center_jobs(&mut self) {
        let workspace = &mut self.workspace;
        let (Some(center), Some(reads)) = (workspace.agents.as_mut(), &workspace.agents_reads)
        else {
            return;
        };
        for job in center.jobs() {
            reads.send(job);
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
            CenterAction::Open(summary) => {
                let root = summary.parent_session_id.unwrap_or(summary.session_id);
                match self.open_listed(tui, *summary) {
                    Ok(()) => {
                        // Opening a hidden session shows it again.
                        if self.tui_state.agents.hidden.remove(&root) {
                            self.tui_state.save();
                        }
                        return;
                    }
                    Err(error) => Err(error),
                }
            }
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
            CenterAction::Rename { session_id, title } => {
                let renamed = self.bridge.rename_listed(session_id, title);
                self.seen_after_change(session_id, &renamed);
                renamed.map(|()| "Session renamed".to_string())
            }
            CenterAction::SetArchived {
                session_id,
                archive,
            } => {
                let changed = self.archive_listed(session_id, archive);
                self.seen_after_change(session_id, &changed);
                changed
            }
            CenterAction::Delete(session_id) => {
                let deleted = self.bridge.delete_listed(session_id);
                if deleted.is_ok() {
                    self.tui_state.agents.forget(session_id);
                    self.tui_state.save();
                }
                deleted.map(|()| "Session deleted".to_string())
            }
            CenterAction::Stop(summary) => self
                .bridge
                .stop_listed(&summary)
                .map(|()| "Interrupting…".to_string()),
            CenterAction::Grouping(grouping) => {
                self.save_agents_grouping(grouping);
                return;
            }
            CenterAction::Marks(marks) => {
                self.save_agents_marks(marks);
                return;
            }
        };
        let current = self.main_session_id();
        let marks = self.tui_state.agents.clone();
        if let Some(center) = self.workspace.agents.as_mut() {
            center.set_marks(marks);
            match done {
                Ok(notice) => {
                    center.refresh(current);
                    center.set_notice(notice);
                }
                Err(error) => center.set_notice(error),
            }
        }
    }

    fn close_agents_center<B: Backend<Error = io::Error> + Write>(&mut self, tui: &mut Tui<B>) {
        self.workspace.agents = None;
        self.workspace.agents_reads = None;
        self.leave_alt_screen(tui);
    }

    /// A change the center made itself is no news: a session seen before
    /// stays seen.
    fn seen_after_change<T>(&mut self, session_id: AgentSessionId, done: &Result<T, String>) {
        let seen = &mut self.tui_state.agents.seen;
        if done.is_ok() && seen.contains_key(&session_id) {
            seen.insert(session_id, now_millis());
            self.tui_state.save();
        }
    }

    /// `p` or `h`: the pins and the hidden sessions kept in `tui-state.json`.
    fn save_agents_marks(&mut self, marks: AgentsMarks) {
        let state = &mut self.tui_state.agents;
        state.pinned = marks.pinned;
        state.hidden = marks.hidden;
        self.tui_state.save();
        let marks = self.tui_state.agents.clone();
        if let Some(center) = self.workspace.agents.as_mut() {
            center.set_marks(marks);
        }
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

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis())
}
