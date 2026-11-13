// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! The chat: the bottom pane (the composer, the status line and the
//! approval and question overlays), and the entries it hands the app's
//! transcript. A thinking or a tool call is a live entry the chat changes
//! in place, shown from its start in the conversation (Fullscreen) or in
//! the tail above the bottom pane (Scrollback), which writes it only once
//! it completes.
//!
//! What the turn does arrives as [`TurnUpdate`]s (`turn`); what the person
//! types goes to the bottom pane, and what it submits leaves as an
//! [`Op`] for the Agent through the chat's queue (`input_flow`,
//! `input_queue`), or runs as a slash command (`commands`).

mod appearance;
mod commands;
mod input_flow;
mod input_queue;
mod replay;
mod second;
mod shortcuts;
mod turn;
mod warnings;

use std::cell::Cell;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hanafubuki_agent::{AgentReasoningId, AgentSessionMode, AgentTuiPreferences};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;

use crate::agent_bridge::Warning;
use crate::app_event::{AppCommand, AppEvent, AppEventSender, Op};
use crate::appearance::Appearance;
use crate::bottom_pane::{BottomPane, BottomPaneParams, CancellationEvent, InputResult};
use crate::commands::PRESETS;
use crate::file_search::FileMatch;
use crate::history_cell::{self, HistoryCell, LiveCell, ThinkingCell, ToolCallCell};
use crate::key_hint::{self, KeyBinding};
use crate::keymap::{self, Action};
use crate::render::renderable::{Renderable, RenderableExt, RenderableItem};
use crate::render::Insets;
use crate::streaming::controller::{PlanStreamController, StreamController};
use crate::tui::FrameRequester;

/// How long a first Ctrl-C (or Ctrl-D) waits for the second that quits.
const QUIT_SHORTCUT_TIMEOUT: Duration = Duration::from_secs(1);

pub(crate) struct ChatWidgetInit {
    pub(crate) app_event_tx: AppEventSender,
    pub(crate) frame_requester: FrameRequester,
    pub(crate) enhanced_keys_supported: bool,
    /// The working directory, as the session header shows it.
    pub(crate) directory: String,
    /// The app's transcript view owns the screen (Fullscreen); otherwise
    /// Scrollback's tail is told when a live entry completes.
    pub(crate) owned_transcript: bool,
    /// The saved `tui.fullscreenTranscript`, which `/tui` marks.
    pub(crate) fullscreen_preference: bool,
    /// The sessions the interface creates get a model of their own (`-m`),
    /// so the model policy may leave it empty.
    pub(crate) model_override: bool,
    /// The theme, the status line and the title as `[tui]` has them.
    pub(crate) appearance: Appearance,
}

pub(crate) struct ChatWidget {
    app_event_tx: AppEventSender,
    frame_requester: FrameRequester,
    bottom_pane: BottomPane,
    /// The tool calls that run, in the order they started, by call id with
    /// their entries.
    running_tools: Vec<(String, Arc<LiveCell<ToolCallCell>>)>,
    /// The calls that finished and wait for what they show
    /// (`TurnUpdate::ToolDisplayed`); their entries stay live until it
    /// arrives.
    awaiting_display: Vec<(String, Arc<LiveCell<ToolCallCell>>)>,
    /// The entry of the exploring calls in a row, which the next exploring
    /// call joins until another entry comes.
    exploration: Option<Arc<LiveCell<ToolCallCell>>>,
    /// The reasoning items the model is thinking in, in the order they
    /// started.
    live_thinking: Vec<(AgentReasoningId, Arc<LiveCell<ThinkingCell>>)>,
    stream_controller: Option<StreamController>,
    plan_stream_controller: Option<PlanStreamController>,
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
    /// command turned off after it panicked, for `/warnings`.
    warnings: Vec<Warning>,
    appearance: Appearance,
    owned_transcript: bool,
    /// `[tui]` as the configuration resolves it (the saved
    /// `tui.fullscreenTranscript`, which `/tui` marks, among them).
    preferences: AgentTuiPreferences,
    /// The warnings dismissed in this run, by `Warning::key`.
    dismissed_warnings: HashSet<String>,
    /// The interface shows a session, or the first message is creating one.
    session_open: bool,
    /// With no session: no Provider or no model is set up, so a message
    /// opens the setup instead of failing.
    setup_needed: bool,
    model_override: bool,
    /// The hooks waiting to be trusted were looked for, once per run.
    hooks_checked: bool,
    /// What the person sent that the conversation has not taken yet.
    input_queue: input_queue::InputQueue,
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
            appearance,
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
            awaiting_display: Vec::new(),
            exploration: None,
            live_thinking: Vec::new(),
            stream_controller: None,
            plan_stream_controller: None,
            last_rendered_width: Cell::new(None),
            needs_final_message_separator: false,
            had_work_activity: false,
            retry_status_header: None,
            quit_shortcut_expires_at: None,
            quit_shortcut_key: None,
            directory,
            mode: AgentSessionMode::Default,
            warnings: Vec::new(),
            appearance,
            owned_transcript,
            preferences: AgentTuiPreferences {
                fullscreen_transcript: fullscreen_preference,
                ..AgentTuiPreferences::default()
            },
            dismissed_warnings: HashSet::new(),
            session_open: false,
            setup_needed: false,
            model_override,
            hooks_checked: false,
            input_queue: input_queue::InputQueue::default(),
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
                KeyCode::Char(c)
                    if c.eq_ignore_ascii_case(&'d')
                        && !self.bottom_pane.captures_keys()
                        && self.on_ctrl_d() =>
                {
                    return
                }
                _ => {}
            }
        }
        if key_event.kind == KeyEventKind::Press {
            self.bottom_pane.clear_quit_shortcut_hint();
            self.quit_shortcut_expires_at = None;
            self.quit_shortcut_key = None;
        }
        if !self.bottom_pane.captures_keys() {
            // The entries' disclosure is the transcript view's.
            if keymap::matches(Action::ToggleThinking, &key_event) {
                self.app_event_tx
                    .send(AppEvent::Command(AppCommand::ToggleThinking));
                return;
            }
            if keymap::matches(Action::OpenPresets, &key_event)
                && self.bottom_pane.no_modal_or_popup_active()
            {
                self.dispatch_command(&PRESETS, String::new());
                return;
            }
            if self.handle_session_key(&key_event) {
                self.request_redraw();
                return;
            }
            if key_event.kind == KeyEventKind::Press
                && self.bottom_pane.no_modal_or_popup_active()
                && ((keymap::matches(Action::EditQueuedMessage, &key_event)
                    && self.edit_queued_message())
                    || (keymap::matches(Action::InterruptTurn, &key_event)
                        && self.bottom_pane.is_task_running()
                        && self.esc_handled()))
            {
                self.request_redraw();
                return;
            }
        }
        match self.bottom_pane.handle_key_event(key_event) {
            InputResult::Submitted(text) => self.submit_user_message(text),
            InputResult::Queued(text) => self.queue_user_message(text),
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

    /// `[tui]` as the configuration resolves it.
    pub(crate) fn preferences(&self) -> &AgentTuiPreferences {
        &self.preferences
    }

    /// `[tui]` as the configuration resolves it after a change.
    pub(crate) fn set_preferences(&mut self, preferences: AgentTuiPreferences) {
        self.preferences = preferences;
    }

    /// Whether the open view takes every key as it is (`/keymap`'s
    /// capture).
    pub(crate) fn captures_keys(&self) -> bool {
        self.bottom_pane.captures_keys()
    }

    /// Adds a dim note to the transcript.
    /// Whether this conversation has the keys, of two panes.
    pub(crate) fn set_input_focus(&mut self, focused: bool) {
        self.bottom_pane.set_input_focus(focused);
    }

    pub(crate) fn add_info_message(&mut self, message: String) {
        self.add_to_history(history_cell::new_info_event(message, None));
        self.request_redraw();
    }

    pub(crate) fn add_error_message(&mut self, message: String) {
        self.add_to_history(history_cell::new_error_event(message));
        self.request_redraw();
    }

    pub(crate) fn is_task_running(&self) -> bool {
        self.bottom_pane.is_task_running()
    }

    /// The footer says a second Esc browses the prompts to edit one.
    pub(crate) fn show_esc_backtrack_hint(&mut self) {
        self.bottom_pane.show_esc_backtrack_hint();
    }

    pub(crate) fn clear_esc_backtrack_hint(&mut self) {
        self.bottom_pane.clear_esc_backtrack_hint();
    }

    /// Puts `text` in the composer, for the person to edit and send.
    pub(crate) fn set_composer_text(&mut self, text: String) {
        self.bottom_pane.set_composer_text(text);
    }

    /// Matches for the `@` token being typed, from the file search.
    pub(crate) fn apply_file_search_result(&mut self, query: String, matches: Vec<FileMatch>) {
        self.bottom_pane.on_file_search_result(query, matches);
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
                self.add_boxed_history(cell);
            }
            all_idle &= is_idle;
        }
        if let Some(controller) = self.plan_stream_controller.as_mut() {
            has_controller = true;
            let (cell, is_idle) = controller.on_commit_tick();
            if let Some(cell) = cell {
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
        self.close_exploration();
        if !cell.display_lines(u16::MAX).is_empty() {
            self.needs_final_message_separator = true;
        }
        self.app_event_tx.send(AppEvent::InsertHistoryCell(cell));
    }

    /// Hands the transcript an entry that changes in place from its start.
    fn insert_live_entry(&mut self, entry: Arc<dyn HistoryCell>) {
        self.close_exploration();
        self.needs_final_message_separator = true;
        self.app_event_tx.send(AppEvent::InsertLiveEntry(entry));
    }

    /// A live entry completed: Scrollback's tail may write it.
    fn complete_live_entry(&self, entry: Arc<dyn HistoryCell>) {
        if !self.owned_transcript {
            self.app_event_tx.send(AppEvent::LiveEntryCompleted(entry));
        }
    }

    /// Whether a thinking or a tool call is still live, so its timer and
    /// spinner are drawn again.
    pub(crate) fn has_live_entries(&self) -> bool {
        !self.live_thinking.is_empty() || !self.running_tools.is_empty()
    }

    fn request_redraw(&self) {
        self.frame_requester.schedule_frame();
    }

    fn as_renderable(&self) -> RenderableItem<'_> {
        RenderableItem::Borrowed(&self.bottom_pane).inset(Insets::tlbr(1, 0, 0, 0))
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
