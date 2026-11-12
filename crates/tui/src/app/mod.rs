// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).
// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The app loop: it receives the terminal's input, the Agent's events and
//! the widgets' requests on one channel, hands each where it goes, keeps
//! the finished cells and draws when a frame is due.
//!
//! The transcript's mode is fixed at launch ([`TranscriptMode`]). In
//! terminal mode the newest cells stay in a tail at the top of an inline
//! viewport and are written into the scrollback above it as they leave the
//! tail (`scrollback`); while an overlay (the transcript, `/diff`, the
//! resume picker) holds the alternate screen, it takes the keys and the
//! tail waits. In owned mode the app holds the alternate screen for
//! the whole run and draws the cells itself (`owned`), the overlays over
//! it. Esc twice on an empty composer browses the prompts to edit an
//! earlier one (`backtrack`).

mod backtrack;
mod history_pages;
mod keys;
mod owned;
mod preferences;
mod requests;
mod scrollback;
mod workspace;

pub(crate) use workspace::{SettingsPageAction, WorkspaceEvent};

use keys::is_transcript_key;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use std::io::{self, Write};

use ratatui::backend::Backend;
use ratatui::text::Line;

use ratatui::crossterm::event::MouseEvent;

use crate::agent_bridge::{AgentBridge, OlderTranscript, TurnUpdate};
use crate::app_event::{AppEvent, AppEventSender};
use crate::appearance::terminal_title::{TerminalTitle, TitleEncoding};
use crate::chatwidget::ChatWidget;
use crate::clipboard::Clipboard;
use crate::diff_render::git_diff_lines;
use crate::file_search::FileSearchManager;
use crate::get_git_diff::get_git_diff;
use crate::history_cell::{HistoryCell, HistoryRenderMode};
use crate::keymap::ChordState;
use crate::pager_overlay::Overlay;
use crate::resume_picker::ResumePicker;
use crate::transcript_mode::TranscriptMode;
use crate::transcript_view::{TranscriptHistoryState, TranscriptView, ViewAction};
use crate::tui::Tui;

/// The commit animation's step: one streamed line per tick.
const COMMIT_TICK: Duration = Duration::from_millis(50);
/// The longest the loop sleeps without a frame due.
const IDLE_WAIT: Duration = Duration::from_secs(3600);
/// At most one frame per this, however often frames are asked for.
const MIN_FRAME_INTERVAL: Duration = Duration::from_millis(8);
/// How often a transcript view is drawn while it scans a search or
/// scrolls a drag past its edge.
const VIEW_TICK: Duration = Duration::from_millis(16);
/// How often a transcript view is drawn while a live entry changes with
/// time (a timer, a spinner).
const LIVE_TICK: Duration = Duration::from_millis(50);

pub(crate) struct App {
    pub(crate) chat_widget: ChatWidget,
    pub(crate) bridge: AgentBridge,
    pub(crate) app_event_tx: AppEventSender,
    file_search: FileSearchManager,
    /// The workspace's first root, where `/diff` runs `git`.
    diff_directory: Option<PathBuf>,
    /// The session's cells as committed: the owned transcript's entries,
    /// or what the terminal-mode transcript overlay opens on.
    transcript_cells: Vec<Arc<dyn HistoryCell>>,
    mode: TranscriptMode,
    /// The owned transcript's viewport; in terminal mode, its tail's.
    transcript_view: TranscriptView,
    /// Terminal mode: the entries not written into the scrollback yet.
    tail: scrollback::Tail,
    tail_mouse: scrollback::TailMouse,
    render_mode: HistoryRenderMode,
    /// Where the session's unread earlier transcript begins, which the
    /// owned transcript loads as the reader nears its start.
    older_transcript: Option<OlderTranscript>,
    clipboard: Clipboard,
    /// What the last copy said, shown in the owned transcript until then.
    copy_notice: Option<(String, Instant)>,
    overlay: Option<Overlay>,
    resume_picker: Option<ResumePicker>,
    /// What was committed while an overlay was open, written into the
    /// scrollback when it closes.
    deferred_history_lines: Vec<Line<'static>>,
    commit_anim_running: Arc<AtomicBool>,
    next_frame: Option<Instant>,
    last_frame: Option<Instant>,
    backtrack: backtrack::Backtrack,
    terminal_title: TerminalTitle,
    /// A chord's first key, while it waits for the second.
    chords: ChordState,
    /// The second conversation, the panes and the command center.
    workspace: workspace::Workspace,
}

impl App {
    /// The app over `chat_widget`, which must have been made for `mode`.
    pub(crate) fn new(
        chat_widget: ChatWidget,
        bridge: AgentBridge,
        app_event_tx: AppEventSender,
        mode: TranscriptMode,
        render_mode: HistoryRenderMode,
    ) -> Self {
        let roots = bridge.workspace_roots();
        Self {
            chat_widget,
            bridge,
            file_search: FileSearchManager::new(roots.clone(), app_event_tx.clone()),
            diff_directory: roots.into_iter().next(),
            app_event_tx,
            transcript_cells: Vec::new(),
            mode,
            transcript_view: new_view(render_mode, TranscriptHistoryState::Complete),
            tail: scrollback::Tail::default(),
            tail_mouse: scrollback::TailMouse::default(),
            render_mode,
            older_transcript: None,
            clipboard: Clipboard::default(),
            copy_notice: None,
            overlay: None,
            resume_picker: None,
            deferred_history_lines: Vec::new(),
            commit_anim_running: Arc::new(AtomicBool::new(false)),
            next_frame: Some(Instant::now()),
            last_frame: None,
            backtrack: backtrack::Backtrack::default(),
            terminal_title: TerminalTitle::new(TitleEncoding::detect()),
            chords: ChordState::default(),
            workspace: workspace::Workspace::default(),
        }
    }

    /// Runs until the person quits.
    pub(crate) fn run<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        events: &Receiver<AppEvent>,
    ) -> io::Result<()> {
        loop {
            let wait = self
                .next_frame
                .map(|at| at.saturating_duration_since(Instant::now()))
                .unwrap_or(IDLE_WAIT);
            match events.recv_timeout(wait) {
                Ok(event) => {
                    if !self.handle_event(tui, event) {
                        break;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            if self.next_frame.is_some_and(|at| at <= Instant::now()) {
                self.draw(tui)?;
            }
        }
        self.commit_anim_running.store(false, Ordering::Release);
        self.write_tail_at_exit(tui)
    }

    /// `false` when the app should exit.
    pub(crate) fn handle_event<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        event: AppEvent,
    ) -> bool {
        let keep_running = match self.route_event(event) {
            workspace::Route::Here(event) => self.handle_here(tui, event),
            workspace::Route::Parked(event) => self.handle_parked(tui, event),
            workspace::Route::Workspace(event) => {
                self.workspace_event(tui, event);
                true
            }
            workspace::Route::Dropped => true,
        };
        self.sync_second(tui);
        keep_running
    }

    /// Handles `event` with the conversation the app's fields hold.
    fn handle_here<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        event: AppEvent,
    ) -> bool {
        match event {
            AppEvent::Key(key) => return self.handle_key_event(tui, key),
            AppEvent::Paste(text) => self.handle_paste(text),
            AppEvent::Mouse(mouse) => self.handle_mouse(tui, mouse),
            AppEvent::Redraw => self.schedule_frame(Instant::now()),
            AppEvent::ScheduleFrame(at) => self.schedule_frame(at),
            AppEvent::Agent(event) => {
                if let Some(update) = self.bridge.event(*event) {
                    self.chat_widget.handle_turn_update(update);
                }
            }
            AppEvent::AgentStatusChanged => {
                if let Some(update) = self.bridge.status_changed() {
                    self.chat_widget.handle_turn_update(update);
                }
            }
            AppEvent::AgentReply(reply) => {
                if let Some(update) = self.bridge.reply(reply) {
                    self.show_update(update);
                }
                self.queue_settings_page_check();
            }
            AppEvent::Op(op) => {
                if let Some(update) = self.bridge.submit(op) {
                    self.show_update(update);
                }
            }
            AppEvent::InsertHistoryCell(cell) => self.insert_history_cell(cell.into(), false),
            AppEvent::InsertLiveEntry(cell) => self.insert_history_cell(cell, true),
            AppEvent::LiveEntryCompleted(cell) => {
                self.tail.complete(&cell);
                self.schedule_frame(Instant::now());
            }
            AppEvent::StartFileSearch(query) => self.file_search.on_user_query(query),
            AppEvent::FileSearchResult { query, matches } => {
                self.chat_widget.apply_file_search_result(query, matches)
            }
            AppEvent::Diff => self.start_diff(),
            AppEvent::DiffResult(text) => {
                self.open_overlay(
                    tui,
                    Overlay::new_static_with_lines(git_diff_lines(&text), "D I F F".to_string()),
                );
            }
            AppEvent::OpenResumePicker => self.open_resume_picker(tui, false),
            AppEvent::StartCommitAnimation => self.start_commit_animation(),
            AppEvent::StopCommitAnimation => {
                self.commit_anim_running.store(false, Ordering::Release)
            }
            AppEvent::CommitTick => self.chat_widget.on_commit_tick(),
            AppEvent::SetRawOutput(raw) => self.set_raw_output(raw),
            AppEvent::Command(command) => self.run_app_command(tui, command),
            AppEvent::Appearance(event) => self.apply_appearance(event),
            AppEvent::CommandStep(step) => self.chat_widget.run_command_step(step),
            AppEvent::Second(..) | AppEvent::Workspace(_) => {}
            AppEvent::Exit => return false,
        }
        true
    }

    /// What a reply or a request led to, for the chat; a session opened
    /// replaces the transcript, which starts with its header.
    pub(crate) fn show_update(&mut self, update: TurnUpdate) {
        if let TurnUpdate::SessionOpened(opened) = &update {
            self.transcript_cells.clear();
            self.older_transcript = opened.older.clone().filter(|_| self.mode.is_owned());
            let history = if self.older_transcript.is_some() {
                TranscriptHistoryState::Partial
            } else {
                TranscriptHistoryState::Complete
            };
            if self.mode.is_owned() {
                self.transcript_view = new_view(self.render_mode, history);
            } else {
                // The tail's view writes the entries as they were shown.
                self.tail.finish();
            }
        }
        self.chat_widget.handle_turn_update(update);
    }

    /// Pasted text goes to the composer, or to a transcript search being
    /// edited; the other overlays take none.
    fn handle_paste(&mut self, text: String) {
        self.cancel_primed_backtrack();
        // Pasted text arrives with `\r` line endings in many terminals.
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        if self.handle_settings_paste(&text) {
            return;
        }
        let to_search = match self.overlay.as_mut() {
            Some(Overlay::Transcript(transcript)) => transcript.handle_paste(&text),
            Some(Overlay::Static(_)) => false,
            None if self.resume_picker.is_some() || self.workspace.agents_open() => false,
            None if self.mode.is_owned() && self.transcript_view.is_search_editing() => {
                self.transcript_view.paste_search(&text)
            }
            None => {
                self.transcript_view.clear_activity_focus();
                self.chat_widget.handle_paste(text);
                return;
            }
        };
        if to_search {
            self.schedule_frame(Instant::now());
        }
    }

    fn handle_mouse<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        mouse: MouseEvent,
    ) {
        if self.handle_settings_mouse(tui, mouse) {
            return;
        }
        if self.overlay.is_none() && self.handle_workspace_mouse(mouse) {
            return;
        }
        if !self.mode.is_owned() && !self.alt_screen_open() {
            return self.handle_tail_mouse(mouse);
        }
        let action = match self.overlay.as_mut() {
            Some(Overlay::Transcript(transcript)) => transcript.handle_mouse(mouse),
            Some(Overlay::Static(_)) => None,
            None if self.resume_picker.is_none() && self.mode.is_owned() => self
                .transcript_view
                .handle_mouse(mouse, &self.transcript_cells),
            None => None,
        };
        if let Some(action) = action {
            self.apply_view_action(action);
        }
    }

    /// What a transcript view asked for after it took an event.
    fn apply_view_action(&mut self, action: ViewAction) {
        let (text, follow) = match action {
            ViewAction::Changed | ViewAction::ExitSelectMode => (None, false),
            ViewAction::Copy(text) => (Some(text), false),
            ViewAction::CopyAndFollow(text) => (Some(text), true),
        };
        if let Some(text) = text {
            let copied = self.clipboard.copy(&text);
            let notice = match &copied {
                Ok(status) => status.message(text.chars().count()),
                Err(error) => format!("Copy failed: {error}"),
            };
            let follow = follow && copied.is_ok();
            match self.overlay.as_mut() {
                Some(Overlay::Transcript(transcript)) => {
                    transcript.set_notice(notice);
                    if follow {
                        transcript.follow_latest();
                    }
                }
                _ => {
                    self.copy_notice = Some((notice, Instant::now() + owned::COPY_NOTICE));
                    if follow {
                        self.transcript_view.jump_to_latest();
                    }
                }
            }
        }
        self.load_older_pages();
        self.schedule_frame(Instant::now());
    }

    fn alt_screen_open(&self) -> bool {
        self.overlay.is_some()
            || self.resume_picker.is_some()
            || self.workspace.agents_open()
            || self.workspace.settings_open()
    }

    fn open_overlay<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        overlay: Overlay,
    ) {
        // The transcript view reads the pointer; the pager leaves the
        // wheel to the terminal, which turns it into arrow keys.
        let capture_mouse = matches!(overlay, Overlay::Transcript(_));
        self.enter_alt_screen(tui, capture_mouse);
        self.overlay = Some(overlay);
        self.schedule_frame(Instant::now());
    }

    /// The alternate screen for an overlay: entered in terminal mode,
    /// already held in owned mode, where only the pointer changes hands.
    fn enter_alt_screen<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        capture_mouse: bool,
    ) {
        if self.alt_screen_open() {
            return;
        }
        if self.mode.is_owned() {
            let _ = tui.set_mouse_capture(capture_mouse);
        } else {
            let _ = tui.enter_alt_screen(capture_mouse);
        }
    }

    /// Opens the resume picker; `at_startup` when the interface was asked
    /// to resume, before it showed any session.
    pub(crate) fn open_resume_picker<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        at_startup: bool,
    ) {
        let picker = ResumePicker::new(at_startup, self.bridge.session_id());
        self.show_resume_picker(tui, picker);
    }

    fn show_resume_picker<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        picker: ResumePicker,
    ) {
        self.enter_alt_screen(tui, false);
        self.resume_picker = Some(picker);
        self.load_picker_pages();
        self.schedule_frame(Instant::now());
    }

    /// Carries out the archiving the picker asks for, and reads the pages
    /// it asks for.
    fn load_picker_pages(&mut self) {
        let Some(picker) = self.resume_picker.as_mut() else {
            return;
        };
        if let Some(request) = picker.take_archive_request() {
            picker.archive_done(request.session_id, self.bridge.set_archived(request));
        }
        while let Some(request) = picker.page_request() {
            picker.ingest_page(self.bridge.list_sessions(request));
        }
    }

    /// The last overlay closed: in terminal mode, the terminal's own screen
    /// again, with what was committed meanwhile written into its
    /// scrollback; in owned mode, the transcript view takes the pointer
    /// back.
    fn leave_alt_screen<B: Backend<Error = io::Error> + Write>(&mut self, tui: &mut Tui<B>) {
        if self.mode.is_owned() {
            let _ = tui.set_mouse_capture(true);
        } else {
            let _ = tui.leave_alt_screen();
            tui.insert_history_lines(std::mem::take(&mut self.deferred_history_lines));
        }
        self.schedule_frame(Instant::now());
    }

    /// Runs `git diff` off the loop; its output opens the diff overlay.
    fn start_diff(&self) {
        // The app's own, whichever conversation has the focus.
        let tx = self.app_event_tx.untagged();
        let directory = self.diff_directory.clone();
        std::thread::spawn(move || {
            let text = match directory.as_deref().map(get_git_diff) {
                Some(Ok((true, diff_text))) => diff_text,
                Some(Ok((false, _))) | None => {
                    "`/diff` — _not inside a git repository_".to_string()
                }
                Some(Err(e)) => format!("Failed to compute diff: {e}"),
            };
            tx.send(AppEvent::DiffResult(text));
        });
    }

    /// Adds `cell` to the transcript, where a `live` one goes on changing;
    /// in terminal mode, to the tail too.
    fn insert_history_cell(&mut self, cell: Arc<dyn HistoryCell>, live: bool) {
        // The transcript overlay shows the focused conversation.
        if let (Some(Overlay::Transcript(transcript)), false) =
            (&mut self.overlay, self.workspace.handling_parked())
        {
            transcript.insert_cell(cell.clone());
        }
        if !self.mode.is_owned() {
            // An answer's next lines are not a new entry: the wheel stays
            // the terminal's while it streams.
            if !cell.is_stream_continuation() {
                self.tail_mouse.released = false;
            }
            self.tail.push(cell.clone(), live);
        }
        self.transcript_cells.push(cell);
        self.schedule_frame(Instant::now());
    }

    fn start_commit_animation(&mut self) {
        if self
            .commit_anim_running
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return;
        }
        // The app's own, whichever conversation has the focus.
        let tx = self.app_event_tx.untagged();
        let running = self.commit_anim_running.clone();
        std::thread::spawn(move || {
            while running.load(Ordering::Relaxed) {
                std::thread::sleep(COMMIT_TICK);
                tx.send(AppEvent::CommitTick);
            }
        });
    }

    fn schedule_frame(&mut self, at: Instant) {
        let at = match self.last_frame {
            Some(last) => at.max(last + MIN_FRAME_INTERVAL),
            None => at,
        };
        self.next_frame = Some(self.next_frame.map_or(at, |next| next.min(at)));
    }

    pub(crate) fn draw<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
    ) -> io::Result<()> {
        self.next_frame = None;
        if self.chat_widget.handle_paste_burst_tick() {
            return Ok(());
        }
        self.sync_terminal_title(tui);
        self.last_frame = Some(Instant::now());
        if let Some(picker) = self.resume_picker.as_mut() {
            tui.draw(u16::MAX, |frame| {
                let area = frame.area();
                picker.render(area, frame.buffer_mut());
            })?;
            // The list's height is known once it is drawn; a list it
            // leaves unfilled asks for more.
            if picker.page_request().is_some() {
                self.load_picker_pages();
                self.schedule_frame(Instant::now());
            }
            return Ok(());
        }
        if self.draw_agents_center(tui)? {
            return Ok(());
        }
        if self.draw_settings_center(tui)? {
            return Ok(());
        }
        if self.overlay.is_some() {
            return self.draw_overlay(tui);
        }
        if self.mode.is_owned() {
            return self.draw_owned(tui);
        }
        self.draw_scrollback(tui)
    }

    /// Draws the open overlay.
    fn draw_overlay<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
    ) -> io::Result<()> {
        let Some(overlay) = self.overlay.as_mut() else {
            return Ok(());
        };
        let mut ticking = false;
        if let Overlay::Transcript(transcript) = overlay {
            let width = tui.terminal.size()?.width;
            if self.backtrack.is_browsing() {
                transcript.set_browsing_footer(backtrack::browsing_footer(width));
            }
            ticking = transcript.tick(width);
            if self.chat_widget.has_live_entries() {
                self.next_frame = Some(Instant::now() + LIVE_TICK);
            }
        }
        tui.draw(u16::MAX, |frame| {
            let area = frame.area();
            if let Some(position) = overlay.render(area, frame.buffer_mut()) {
                frame.set_cursor_position(position);
            }
        })?;
        if ticking {
            self.schedule_frame(Instant::now() + VIEW_TICK);
        }
        Ok(())
    }
}

/// A transcript view in `render_mode`'s compact presentation.
fn new_view(render_mode: HistoryRenderMode, history: TranscriptHistoryState) -> TranscriptView {
    let mut view = TranscriptView::default();
    view.set_presentation(false, render_mode);
    view.history = history;
    view
}

/// The diff's lines as the pager shows them: added lines green, removed
/// ones red, hunk headers cyan and file headers bold.
#[cfg(test)]
mod tests;
