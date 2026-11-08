// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! The chat: the running tool calls above the bottom pane (the composer,
//! the status line and the approval and question overlays), and the
//! transcript it hands the app to write into the scrollback.
//!
//! What the turn does arrives as [`TurnUpdate`]s (`turn`); what the person
//! types goes to the bottom pane, and what it submits leaves as an
//! [`Op`] for the Agent.

mod turn;

use std::cell::Cell;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;

use crate::agent_bridge::TurnUpdate;
use crate::app_event::{AppEvent, AppEventSender, Op};
use crate::bottom_pane::{BottomPane, BottomPaneParams, CancellationEvent, InputResult};
use crate::history_cell::{self, HistoryCell, ToolCallCell};
use crate::key_hint::{self, KeyBinding};
use crate::render::renderable::{
    ColumnRenderable, FlexRenderable, Renderable, RenderableExt, RenderableItem,
};
use crate::render::Insets;
use crate::streaming::controller::{PlanStreamController, StreamController};
use crate::tui::FrameRequester;

/// How long a first Ctrl-C (or Ctrl-D) waits for the second that quits.
const QUIT_SHORTCUT_TIMEOUT: Duration = Duration::from_secs(1);

/// What `/quit` and `/exit` are: the one command before the slash commands.
const QUIT_COMMANDS: [&str; 2] = ["/quit", "/exit"];

pub(crate) struct ChatWidgetInit {
    pub(crate) app_event_tx: AppEventSender,
    pub(crate) frame_requester: FrameRequester,
    pub(crate) enhanced_keys_supported: bool,
    pub(crate) model: String,
    pub(crate) directory: String,
}

pub(crate) struct ChatWidget {
    app_event_tx: AppEventSender,
    frame_requester: FrameRequester,
    bottom_pane: BottomPane,
    /// The tool calls that run, in the order they started.
    running_tools: Vec<ToolCallCell>,
    stream_controller: Option<StreamController>,
    plan_stream_controller: Option<PlanStreamController>,
    /// Updates held back while a message streams, so what the model said
    /// before a call is written before the call (Codex's interrupt queue).
    deferred: VecDeque<TurnUpdate>,
    last_rendered_width: Cell<Option<usize>>,
    /// Something was written since the last streamed message; the next one
    /// gets a separator, with the time worked when there was work.
    needs_final_message_separator: bool,
    had_work_activity: bool,
    /// The status line while a model request is retried, kept over the
    /// reasoning headlines until the retry gets an answer.
    retry_status_header: Option<String>,
    quit_shortcut_expires_at: Option<Instant>,
    quit_shortcut_key: Option<KeyBinding>,
}

impl ChatWidget {
    pub(crate) fn new(init: ChatWidgetInit) -> Self {
        let ChatWidgetInit {
            app_event_tx,
            frame_requester,
            enhanced_keys_supported,
            model,
            directory,
        } = init;
        let bottom_pane = BottomPane::new(BottomPaneParams {
            app_event_tx: app_event_tx.clone(),
            frame_requester: frame_requester.clone(),
            has_input_focus: true,
            enhanced_keys_supported,
            placeholder_text: "Ask the Agent to do anything".to_string(),
        });
        let widget = Self {
            app_event_tx,
            frame_requester,
            bottom_pane,
            running_tools: Vec::new(),
            stream_controller: None,
            plan_stream_controller: None,
            deferred: VecDeque::new(),
            last_rendered_width: Cell::new(None),
            needs_final_message_separator: false,
            had_work_activity: false,
            retry_status_header: None,
            quit_shortcut_expires_at: None,
            quit_shortcut_key: None,
        };
        widget
            .app_event_tx
            .send(AppEvent::InsertHistoryCell(Box::new(
                history_cell::SessionHeaderHistoryCell::new(
                    model,
                    directory,
                    env!("CARGO_PKG_VERSION"),
                ),
            )));
        widget
    }

    pub(crate) fn handle_key_event(&mut self, key_event: KeyEvent) {
        if key_event.kind == KeyEventKind::Press
            && key_event.modifiers.contains(KeyModifiers::CONTROL)
        {
            match key_event.code {
                KeyCode::Char(c) if c.eq_ignore_ascii_case(&'c') => {
                    self.on_ctrl_c();
                    return;
                }
                KeyCode::Char(c) if c.eq_ignore_ascii_case(&'d') && self.on_ctrl_d() => return,
                _ => {}
            }
        }
        if key_event.kind == KeyEventKind::Press {
            self.bottom_pane.clear_quit_shortcut_hint();
            self.quit_shortcut_expires_at = None;
            self.quit_shortcut_key = None;
        }
        match self.bottom_pane.handle_key_event(key_event) {
            InputResult::Submitted(text) => self.submit_user_message(text),
            InputResult::None => {}
        }
        self.request_redraw();
    }

    pub(crate) fn handle_paste(&mut self, text: String) {
        self.bottom_pane.handle_paste(text);
        self.request_redraw();
    }

    /// Flushes a paste burst that is due; `true` when this frame should be
    /// skipped (a burst flushed, or one is still being captured).
    pub(crate) fn handle_paste_burst_tick(&mut self) -> bool {
        if self.bottom_pane.flush_paste_burst_if_due() {
            self.request_redraw();
            true
        } else if self.bottom_pane.is_in_paste_burst() {
            self.frame_requester
                .schedule_frame_in(BottomPane::paste_burst_flush_delay());
            true
        } else {
            false
        }
    }

    pub(crate) fn submit_user_message(&mut self, text: String) {
        if QUIT_COMMANDS.contains(&text.trim()) {
            self.app_event_tx.send(AppEvent::Exit);
            return;
        }
        if text.trim().is_empty() {
            return;
        }
        self.add_to_history(history_cell::new_user_prompt(text.clone()));
        self.app_event_tx.send(AppEvent::Op(Op::UserInput(text)));
    }

    fn on_ctrl_c(&mut self) {
        let key = key_hint::ctrl(KeyCode::Char('c'));
        let modal_or_popup_active = !self.bottom_pane.no_modal_or_popup_active();
        if self.bottom_pane.on_ctrl_c() == CancellationEvent::Handled {
            if modal_or_popup_active {
                self.quit_shortcut_expires_at = None;
                self.quit_shortcut_key = None;
                self.bottom_pane.clear_quit_shortcut_hint();
            } else {
                self.arm_quit_shortcut(key);
            }
            return;
        }
        if self.quit_shortcut_active_for(key) {
            self.app_event_tx.send(AppEvent::Exit);
            return;
        }
        self.arm_quit_shortcut(key);
        if self.bottom_pane.is_task_running() {
            self.app_event_tx.send(AppEvent::Op(Op::Interrupt));
        }
    }

    /// Ctrl-D quits only from an empty composer with no overlay open.
    fn on_ctrl_d(&mut self) -> bool {
        let key = key_hint::ctrl(KeyCode::Char('d'));
        if self.quit_shortcut_active_for(key) {
            self.app_event_tx.send(AppEvent::Exit);
            return true;
        }
        if !self.bottom_pane.composer_is_empty() || !self.bottom_pane.no_modal_or_popup_active() {
            return false;
        }
        self.arm_quit_shortcut(key);
        true
    }

    fn quit_shortcut_active_for(&self, key: KeyBinding) -> bool {
        self.quit_shortcut_key == Some(key)
            && self
                .quit_shortcut_expires_at
                .is_some_and(|expires_at| Instant::now() < expires_at)
    }

    fn arm_quit_shortcut(&mut self, key: KeyBinding) {
        self.quit_shortcut_expires_at = Some(Instant::now() + QUIT_SHORTCUT_TIMEOUT);
        self.quit_shortcut_key = Some(key);
        self.bottom_pane.show_quit_shortcut_hint(key);
        self.frame_requester
            .schedule_frame_in(QUIT_SHORTCUT_TIMEOUT);
    }

    /// One step of the streamed messages' commit animation.
    pub(crate) fn on_commit_tick(&mut self) {
        let mut has_controller = false;
        let mut all_idle = true;
        if let Some(controller) = self.stream_controller.as_mut() {
            has_controller = true;
            let (cell, is_idle) = controller.on_commit_tick();
            if let Some(cell) = cell {
                self.bottom_pane.hide_status_indicator();
                self.add_boxed_history(cell);
            }
            all_idle &= is_idle;
        }
        if let Some(controller) = self.plan_stream_controller.as_mut() {
            has_controller = true;
            let (cell, is_idle) = controller.on_commit_tick();
            if let Some(cell) = cell {
                self.bottom_pane.hide_status_indicator();
                self.add_boxed_history(cell);
            }
            all_idle &= is_idle;
        }
        if has_controller && all_idle {
            self.app_event_tx.send(AppEvent::StopCommitAnimation);
        }
    }

    fn add_to_history(&mut self, cell: impl HistoryCell + 'static) {
        self.add_boxed_history(Box::new(cell));
    }

    fn add_boxed_history(&mut self, cell: Box<dyn HistoryCell>) {
        if !cell.display_lines(u16::MAX).is_empty() {
            self.needs_final_message_separator = true;
        }
        self.app_event_tx.send(AppEvent::InsertHistoryCell(cell));
    }

    fn request_redraw(&self) {
        self.frame_requester.schedule_frame();
    }

    fn as_renderable(&self) -> RenderableItem<'_> {
        let mut tools = ColumnRenderable::new();
        for cell in &self.running_tools {
            tools.push_ref(cell);
        }
        let mut flex = FlexRenderable::new();
        if !self.running_tools.is_empty() {
            flex.push(
                1,
                RenderableItem::Owned(Box::new(tools)).inset(Insets::tlbr(1, 0, 0, 0)),
            );
        }
        flex.push(
            0,
            RenderableItem::Borrowed(&self.bottom_pane).inset(Insets::tlbr(1, 0, 0, 0)),
        );
        RenderableItem::Owned(Box::new(flex))
    }
}

/// A running tool call is drawn in the viewport, as Codex draws its active
/// cell, until it is written to the transcript.
impl Renderable for ToolCallCell {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        use ratatui::widgets::{Paragraph, Widget};
        Paragraph::new(self.display_lines(area.width)).render(area, buf);
    }

    fn desired_height(&self, width: u16) -> u16 {
        HistoryCell::desired_height(self, width)
    }
}

impl Renderable for ChatWidget {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        self.as_renderable().render(area, buf);
        self.last_rendered_width.set(Some(area.width as usize));
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.as_renderable().desired_height(width)
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        self.as_renderable().cursor_pos(area)
    }
}

#[cfg(test)]
mod tests;
