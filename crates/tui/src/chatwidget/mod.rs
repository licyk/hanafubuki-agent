// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! The chat: the running tool calls above the bottom pane (the composer,
//! the status line and the approval and question overlays), and the
//! transcript it hands the app to write into the scrollback.
//!
//! What the turn does arrives as [`TurnUpdate`]s (`turn`); what the person
//! types goes to the bottom pane, and what it submits leaves as an
//! [`Op`] for the Agent, or runs as a slash command (`commands`).

mod commands;
mod live_tail;
mod replay;
mod turn;

pub(crate) use live_tail::ActiveCellTranscriptKey;

use std::cell::Cell;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

use hanafubuki_agent::{AgentReasoningId, AgentSessionMode};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;

use crate::agent_bridge::TurnUpdate;
use crate::app_event::{AppEvent, AppEventSender, Op};
use crate::bottom_pane::{
    BottomPane, BottomPaneParams, CancellationEvent, InputResult, ThinkingToggle,
};
use crate::commands::PROVIDERS;
use crate::file_search::FileMatch;
use crate::history_cell::{self, HistoryCell, ThinkingCell, ToolCallCell};
use crate::key_hint::{self, KeyBinding};
use crate::render::renderable::{
    ColumnRenderable, FlexRenderable, Renderable, RenderableExt, RenderableItem,
};
use crate::render::Insets;
use crate::streaming::controller::{PlanStreamController, StreamController};
use crate::tui::FrameRequester;

/// How long a first Ctrl-C (or Ctrl-D) waits for the second that quits.
const QUIT_SHORTCUT_TIMEOUT: Duration = Duration::from_secs(1);
/// How often a live thinking cell is redrawn, for its shimmer and timer.
const THINKING_FRAME: Duration = Duration::from_millis(32);

pub(crate) struct ChatWidgetInit {
    pub(crate) app_event_tx: AppEventSender,
    pub(crate) frame_requester: FrameRequester,
    pub(crate) enhanced_keys_supported: bool,
    /// The working directory, as the session header shows it.
    pub(crate) directory: String,
    /// The app draws the live cells in its transcript view (owned mode),
    /// so the chat does not.
    pub(crate) owned_transcript: bool,
    /// The saved `tui.fullscreenTranscript`, which `/tui` marks.
    pub(crate) fullscreen_preference: bool,
    /// The sessions the interface creates get a model of their own (`-m`),
    /// so the model policy may leave it empty.
    pub(crate) model_override: bool,
}

pub(crate) struct ChatWidget {
    app_event_tx: AppEventSender,
    frame_requester: FrameRequester,
    bottom_pane: BottomPane,
    /// The tool calls that run, in the order they started.
    running_tools: Vec<ToolCallCell>,
    /// The reasoning items the model is thinking in, in the order they
    /// started.
    live_thinking: Vec<(AgentReasoningId, ThinkingCell)>,
    /// Whether thinking is shown expanded: Ctrl-O toggles it for the live
    /// cells and every later one.
    thinking_expanded: bool,
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
    /// The status line while a model request is retried, kept until the
    /// retry gets an answer.
    retry_status_header: Option<String>,
    quit_shortcut_expires_at: Option<Instant>,
    quit_shortcut_key: Option<KeyBinding>,
    directory: String,
    /// The session's mode, which `/mode` offers to change.
    mode: AgentSessionMode,
    /// What went wrong in the interface itself in this run, such as a
    /// command turned off after it panicked.
    #[allow(dead_code, reason = "`/warnings` will read them")]
    warnings: Vec<String>,
    /// Advanced by every change to the live cells, so a cached layout of
    /// the live tail is refreshed.
    live_revision: u64,
    owned_transcript: bool,
    fullscreen_preference: bool,
    /// The interface shows a session, or the first message is creating one.
    session_open: bool,
    /// With no session: no Provider or no model is set up, so a message
    /// opens the setup instead of failing.
    setup_needed: bool,
    model_override: bool,
}

impl ChatWidget {
    pub(crate) fn new(init: ChatWidgetInit) -> Self {
        let ChatWidgetInit {
            app_event_tx,
            frame_requester,
            enhanced_keys_supported,
            directory,
            owned_transcript,
            fullscreen_preference,
            model_override,
        } = init;
        let bottom_pane = BottomPane::new(BottomPaneParams {
            app_event_tx: app_event_tx.clone(),
            frame_requester: frame_requester.clone(),
            has_input_focus: true,
            enhanced_keys_supported,
            placeholder_text: "Ask the Agent to do anything".to_string(),
        });
        Self {
            app_event_tx,
            frame_requester,
            bottom_pane,
            running_tools: Vec::new(),
            live_thinking: Vec::new(),
            thinking_expanded: false,
            stream_controller: None,
            plan_stream_controller: None,
            deferred: VecDeque::new(),
            last_rendered_width: Cell::new(None),
            needs_final_message_separator: false,
            had_work_activity: false,
            retry_status_header: None,
            quit_shortcut_expires_at: None,
            quit_shortcut_key: None,
            directory,
            mode: AgentSessionMode::Default,
            warnings: Vec::new(),
            live_revision: 0,
            owned_transcript,
            fullscreen_preference,
            session_open: false,
            setup_needed: false,
            model_override,
        }
    }

    pub(crate) fn handle_key_event(&mut self, key_event: KeyEvent) {
        if key_event.kind == KeyEventKind::Press
            && key_event.modifiers.contains(KeyModifiers::CONTROL)
        {
            match key_event.code {
                KeyCode::Char(c) if c.eq_ignore_ascii_case(&'c') => {
                    self.on_ctrl_c();
                    self.reap_failed_views();
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
            if key_event.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(key_event.code, KeyCode::Char(c) if c.eq_ignore_ascii_case(&'o'))
            {
                self.toggle_thinking();
                return;
            }
        }
        match self.bottom_pane.handle_key_event(key_event) {
            InputResult::Submitted(text) => self.submit_user_message(text),
            InputResult::Command(command, args) => self.dispatch_command(command, args),
            InputResult::None => {}
        }
        self.reap_failed_views();
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

    pub(crate) fn composer_is_empty(&self) -> bool {
        self.bottom_pane.composer_is_empty()
    }

    /// Whether the composer has the keys: no popup, approval, question or
    /// command view is open.
    pub(crate) fn no_modal_or_popup_active(&self) -> bool {
        self.bottom_pane.no_modal_or_popup_active()
    }

    pub(crate) fn set_fullscreen_preference(&mut self, enabled: bool) {
        self.fullscreen_preference = enabled;
    }

    /// Adds a dim note to the transcript.
    pub(crate) fn add_info_message(&mut self, message: String) {
        self.add_to_history(history_cell::new_info_event(message, None));
        self.request_redraw();
    }

    pub(crate) fn add_error_message(&mut self, message: String) {
        self.add_to_history(history_cell::new_error_event(message));
        self.request_redraw();
    }

    #[cfg(test)]
    pub(crate) fn is_task_running(&self) -> bool {
        self.bottom_pane.is_task_running()
    }

    /// Matches for the `@` token being typed, from the file search.
    pub(crate) fn apply_file_search_result(&mut self, query: String, matches: Vec<FileMatch>) {
        self.bottom_pane.on_file_search_result(query, matches);
    }

    /// Sends `text` to the session, created by it when there is none; with
    /// no Provider or model set up yet, the text goes back to the composer
    /// and the setup opens.
    pub(crate) fn submit_user_message(&mut self, text: String) {
        if text.trim().is_empty() {
            return;
        }
        if self.setup_needed
            && !self.session_open
            && !self.bottom_pane.command_disabled(PROVIDERS.name)
        {
            self.bottom_pane.set_composer_text(text);
            self.add_info_message(
                "Set up a model Provider and choose a model first; your message is kept."
                    .to_string(),
            );
            self.open_setup();
            return;
        }
        self.session_open = true;
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

    fn toggle_thinking(&mut self) {
        self.live_revision += 1;
        self.thinking_expanded = !self.thinking_expanded;
        for (_, cell) in &mut self.live_thinking {
            cell.set_expanded(self.thinking_expanded);
        }
        self.sync_thinking_toggle();
        self.request_redraw();
    }

    /// The footer offers the thinking toggle while a thinking cell is live.
    fn sync_thinking_toggle(&mut self) {
        let toggle = (!self.live_thinking.is_empty()).then_some(if self.thinking_expanded {
            ThinkingToggle::Collapse
        } else {
            ThinkingToggle::Expand
        });
        self.bottom_pane.set_thinking_toggle(toggle);
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
        for (_, cell) in &self.live_thinking {
            tools.push_ref(cell);
        }
        for cell in &self.running_tools {
            tools.push_ref(cell);
        }
        let mut flex = FlexRenderable::new();
        if !self.owned_transcript
            && (!self.running_tools.is_empty() || !self.live_thinking.is_empty())
        {
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

/// A live thinking cell taller than the viewport leaves shows its latest
/// lines.
impl Renderable for ThinkingCell {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        use ratatui::widgets::{Paragraph, Widget};
        let lines = self.display_lines(area.width);
        let overflow = lines.len().saturating_sub(usize::from(area.height));
        Paragraph::new(lines)
            .scroll((u16::try_from(overflow).unwrap_or(u16::MAX), 0))
            .render(area, buf);
    }

    fn desired_height(&self, width: u16) -> u16 {
        HistoryCell::desired_height(self, width)
    }
}

impl Renderable for ChatWidget {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        if !self.live_thinking.is_empty() {
            self.frame_requester.schedule_frame_in(THINKING_FRAME);
        }
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
