// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! The app loop: it receives the terminal's input, the Agent's events and
//! the widgets' requests on one channel, hands each where it goes, writes
//! the finished cells into the scrollback and draws the viewport when a
//! frame is due.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use std::io::{self, Write};

use ratatui::backend::Backend;
use ratatui::text::Line;

use crate::agent_bridge::AgentBridge;
use crate::app_event::{AppEvent, AppEventSender};
use crate::chatwidget::ChatWidget;
use crate::history_cell::HistoryCell;
use crate::render::renderable::Renderable;
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
        Self {
            chat_widget,
            bridge,
            app_event_tx,
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
            AppEvent::Key(key) => self.chat_widget.handle_key_event(key),
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
                    self.chat_widget.handle_turn_update(update);
                }
            }
            AppEvent::Op(op) => {
                if let Some(update) = self.bridge.submit(op) {
                    self.chat_widget.handle_turn_update(update);
                }
            }
            AppEvent::InsertHistoryCell(cell) => self.insert_history_cell(tui, cell.as_ref()),
            AppEvent::StartCommitAnimation => self.start_commit_animation(),
            AppEvent::StopCommitAnimation => {
                self.commit_anim_running.store(false, Ordering::Release)
            }
            AppEvent::CommitTick => self.chat_widget.on_commit_tick(),
            AppEvent::Exit => return false,
        }
        true
    }

    fn insert_history_cell<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
        cell: &dyn HistoryCell,
    ) {
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
        tui.insert_history_lines(display);
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
        let width = tui.terminal.size()?.width;
        let height = self.chat_widget.desired_height(width);
        let chat_widget = &self.chat_widget;
        tui.draw(height, |frame| {
            let area = frame.area();
            chat_widget.render(area, frame.buffer_mut());
            if let Some((x, y)) = chat_widget.cursor_pos(area) {
                frame.set_cursor_position((x, y));
            }
        })
    }
}

#[cfg(test)]
mod tests;
