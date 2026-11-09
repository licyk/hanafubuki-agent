// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! The app loop: it receives the terminal's input, the Agent's events and
//! the widgets' requests on one channel, hands each where it goes, writes
//! the finished cells into the scrollback and draws the viewport when a
//! frame is due. While an overlay (the transcript, `/diff`, the resume
//! picker) holds the alternate screen, it takes the keys and the scrollback
//! waits.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use std::io::{self, Write};

use ratatui::backend::Backend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::style::Stylize;
use ratatui::text::Line;

use crate::agent_bridge::{AgentBridge, TurnUpdate};
use crate::app_event::{AgentCommand, AppEvent, AppEventSender, Op};
use crate::chatwidget::ChatWidget;
use crate::file_search::FileSearchManager;
use crate::get_git_diff::get_git_diff;
use crate::history_cell::HistoryCell;
use crate::pager_overlay::Overlay;
use crate::render::renderable::Renderable;
use crate::resume_picker::{ResumePicker, SessionSelection};
use crate::tui::Tui;

/// The commit animation's step: one streamed line per tick.
const COMMIT_TICK: Duration = Duration::from_millis(50);
/// The longest the loop sleeps without a frame due.
const IDLE_WAIT: Duration = Duration::from_secs(3600);
/// At most one frame per this, however often frames are asked for.
const MIN_FRAME_INTERVAL: Duration = Duration::from_millis(8);

pub(crate) struct App {
    pub(crate) chat_widget: ChatWidget,
    pub(crate) bridge: AgentBridge,
    pub(crate) app_event_tx: AppEventSender,
    file_search: FileSearchManager,
    /// The workspace's first root, where `/diff` runs `git`.
    diff_directory: Option<PathBuf>,
    /// The session's cells as committed, for the transcript overlay.
    transcript_cells: Vec<Arc<dyn HistoryCell>>,
    overlay: Option<Overlay>,
    resume_picker: Option<ResumePicker>,
    /// What was committed while an overlay was open, written into the
    /// scrollback when it closes.
    deferred_history_lines: Vec<Line<'static>>,
    commit_anim_running: Arc<AtomicBool>,
    has_emitted_history_lines: bool,
    next_frame: Option<Instant>,
    last_frame: Option<Instant>,
}

impl App {
    pub(crate) fn new(
        chat_widget: ChatWidget,
        bridge: AgentBridge,
        app_event_tx: AppEventSender,
    ) -> Self {
        let roots = bridge.workspace_roots();
        Self {
            chat_widget,
            bridge,
            file_search: FileSearchManager::new(roots.clone(), app_event_tx.clone()),
            diff_directory: roots.into_iter().next(),
            app_event_tx,
            transcript_cells: Vec::new(),
            overlay: None,
            resume_picker: None,
            deferred_history_lines: Vec::new(),
            commit_anim_running: Arc::new(AtomicBool::new(false)),
            has_emitted_history_lines: false,
            next_frame: Some(Instant::now()),
            last_frame: None,
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
        Ok(())
    }

    /// `false` when the app should exit.
    pub(crate) fn handle_event<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        event: AppEvent,
    ) -> bool {
        match event {
            AppEvent::Key(key) => return self.handle_key_event(tui, key),
            // The overlays take no text.
            AppEvent::Paste(_) if self.alt_screen_open() => {}
            AppEvent::Paste(text) => {
                // Pasted text arrives with `\r` line endings in many terminals.
                self.chat_widget
                    .handle_paste(text.replace("\r\n", "\n").replace('\r', "\n"))
            }
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
            }
            AppEvent::Op(op) => {
                if let Some(update) = self.bridge.submit(op) {
                    self.show_update(update);
                }
            }
            AppEvent::InsertHistoryCell(cell) => self.insert_history_cell(tui, cell.into()),
            AppEvent::StartFileSearch(query) => self.file_search.on_user_query(query),
            AppEvent::FileSearchResult { query, matches } => {
                self.chat_widget.apply_file_search_result(query, matches)
            }
            AppEvent::Diff => self.start_diff(),
            AppEvent::DiffResult(text) => {
                self.open_overlay(
                    tui,
                    Overlay::new_static_with_lines(diff_lines(&text), "D I F F".to_string()),
                );
            }
            AppEvent::OpenResumePicker => self.open_resume_picker(tui, false),
            AppEvent::StartCommitAnimation => self.start_commit_animation(),
            AppEvent::StopCommitAnimation => {
                self.commit_anim_running.store(false, Ordering::Release)
            }
            AppEvent::CommitTick => self.chat_widget.on_commit_tick(),
            AppEvent::Exit => return false,
        }
        true
    }

    /// What a reply or a request led to, for the chat; a session opened
    /// replaces the transcript, which starts with its header.
    fn show_update(&mut self, update: TurnUpdate) {
        if matches!(update, TurnUpdate::SessionOpened(_)) {
            self.transcript_cells.clear();
        }
        self.chat_widget.handle_turn_update(update);
    }

    /// `false` when the app should exit.
    fn handle_key_event<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        key: KeyEvent,
    ) -> bool {
        if let Some(picker) = self.resume_picker.as_mut() {
            let selection = picker.handle_key(key);
            self.schedule_frame(Instant::now());
            let Some(selection) = selection else {
                self.load_picker_pages();
                return true;
            };
            self.resume_picker = None;
            // The session left is replaced, header and all: what it wrote
            // while the picker was open is not shown.
            if matches!(selection, SessionSelection::Resume(_)) {
                self.deferred_history_lines.clear();
            }
            self.leave_alt_screen(tui);
            match selection {
                SessionSelection::StartFresh => {}
                SessionSelection::Resume(session_id) => self
                    .app_event_tx
                    .send(AppEvent::Op(Op::Command(AgentCommand::Resume(session_id)))),
                SessionSelection::Exit => return false,
            }
            return true;
        }
        if let Some(overlay) = self.overlay.as_mut() {
            overlay.handle_key_event(key, tui.terminal.viewport_area);
            if overlay.is_done() {
                self.overlay = None;
                self.leave_alt_screen(tui);
            }
            self.schedule_frame(Instant::now());
            return true;
        }
        if key.kind == KeyEventKind::Press
            && key.modifiers == KeyModifiers::CONTROL
            && matches!(key.code, KeyCode::Char('t'))
        {
            let cells = self.transcript_cells.clone();
            self.open_overlay(tui, Overlay::new_transcript(cells));
            return true;
        }
        self.chat_widget.handle_key_event(key);
        true
    }

    fn alt_screen_open(&self) -> bool {
        self.overlay.is_some() || self.resume_picker.is_some()
    }

    fn open_overlay<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        overlay: Overlay,
    ) {
        if !self.alt_screen_open() {
            let _ = tui.enter_alt_screen();
        }
        self.overlay = Some(overlay);
        self.schedule_frame(Instant::now());
    }

    /// Opens the resume picker; `at_startup` when the interface was asked
    /// to resume, before it showed any session.
    pub(crate) fn open_resume_picker<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        at_startup: bool,
    ) {
        if !self.alt_screen_open() {
            let _ = tui.enter_alt_screen();
        }
        self.resume_picker = Some(ResumePicker::new(at_startup));
        self.load_picker_pages();
        self.schedule_frame(Instant::now());
    }

    /// Reads the pages the picker asks for.
    fn load_picker_pages(&mut self) {
        let Some(picker) = self.resume_picker.as_mut() else {
            return;
        };
        while let Some(request) = picker.page_request() {
            picker.ingest_page(self.bridge.list_sessions(request));
        }
    }

    /// The last overlay closed: the terminal's own screen again, with what
    /// was committed meanwhile written into its scrollback.
    fn leave_alt_screen<B: Backend<Error = io::Error> + Write>(&mut self, tui: &mut Tui<B>) {
        let _ = tui.leave_alt_screen();
        tui.insert_history_lines(std::mem::take(&mut self.deferred_history_lines));
        self.schedule_frame(Instant::now());
    }

    /// Runs `git diff` off the loop; its output opens the diff overlay.
    fn start_diff(&self) {
        let tx = self.app_event_tx.clone();
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

    fn insert_history_cell<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        cell: Arc<dyn HistoryCell>,
    ) {
        if let Some(Overlay::Transcript(transcript)) = &mut self.overlay {
            transcript.insert_cell(cell.clone());
        }
        self.transcript_cells.push(cell.clone());
        let width = tui.terminal.last_known_screen_size.width;
        let mut display = cell.display_lines(width);
        if display.is_empty() {
            return;
        }
        // Cells are separated by a blank line, a streamed message's chunks
        // are not.
        if !cell.is_stream_continuation() {
            if self.has_emitted_history_lines {
                display.insert(0, Line::from(""));
            } else {
                self.has_emitted_history_lines = true;
            }
        }
        if self.alt_screen_open() {
            self.deferred_history_lines.extend(display);
        } else {
            tui.insert_history_lines(display);
        }
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
        let tx = self.app_event_tx.clone();
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
        if let Some(overlay) = self.overlay.as_mut() {
            return tui.draw(u16::MAX, |frame| {
                let area = frame.area();
                overlay.render(area, frame.buffer_mut());
            });
        }
        let width = tui.terminal.size()?.width;
        let height = self.chat_widget.desired_height(width);
        let chat_widget = &self.chat_widget;
        tui.draw(height, |frame| {
            let area = frame.area();
            chat_widget.render(area, frame.buffer_mut());
            if let Some((x, y)) = chat_widget.cursor_pos(area) {
                frame.set_cursor_position((x, y));
            }
        })?;
        // A command's view that panicked while drawn is gone from the next
        // frame, which its error cell schedules.
        self.chat_widget.reap_failed_views();
        Ok(())
    }
}

/// The diff's lines as the pager shows them: added lines green, removed
/// ones red, hunk headers cyan and file headers bold.
fn diff_lines(text: &str) -> Vec<Line<'static>> {
    if text.trim().is_empty() {
        return vec!["No changes detected.".italic().into()];
    }
    text.lines()
        .map(|line| {
            let line = line.to_string();
            if line.starts_with("diff ")
                || line.starts_with("index ")
                || line.starts_with("+++")
                || line.starts_with("---")
            {
                line.bold().into()
            } else if line.starts_with('+') {
                line.green().into()
            } else if line.starts_with('-') {
                line.red().into()
            } else if line.starts_with("@@") {
                line.cyan().into()
            } else {
                line.into()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
