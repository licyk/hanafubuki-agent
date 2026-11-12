// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! The bottom pane is the interactive footer of the chat UI.
//!
//! The pane owns the [`ChatComposer`] (editable prompt input, with its slash-command and file
//! popups), the messages waiting above it ([`PendingInput`]) and a stack of transient
//! [`BottomPaneView`]s (the approval and question modals, the selection lists of `/model` and
//! `/mode`) that temporarily replace the composer for focused interactions.
//!
//! Input routing is layered: `BottomPane` decides which local surface receives a key (view vs
//! composer), while higher-level intent such as "interrupt" or "quit" is decided by the parent
//! widget (`ChatWidget`). This split matters for Ctrl+C/Ctrl+D: the bottom pane gives the active
//! view the first chance to consume Ctrl+C (typically to dismiss itself), and `ChatWidget` may
//! treat an unhandled Ctrl+C as an interrupt or as the first press of a double-press quit
//! shortcut.
//!
//! Some UI is time-based rather than input-based, such as the transient "press again to quit"
//! hint. The pane schedules redraws so those hints can expire even when the UI is otherwise idle.

mod approval_overlay;
mod bottom_pane_view;
mod chat_composer;
mod chat_composer_history;
mod command_popup;
mod file_search_popup;
mod footer;
mod form;
mod list_selection_view;
mod menu_view;
mod module_view;
mod multi_select_picker;
mod paste_burst;
mod pending_input_preview;
mod popup_consts;
mod request_user_input;
mod scroll_state;
mod selection_popup_common;
mod textarea;

use std::time::Duration;

use hanafubuki_agent::{
    AgentReasoningSnapshot, AgentUserInputRequest, AgentUserInputRequestId, ApprovalRequest,
    ApprovalRequestId,
};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::KeyCode;
use ratatui::crossterm::event::KeyEvent;
use ratatui::layout::Rect;
use ratatui::text::Line;

use crate::app_event::{AppEvent, AppEventSender, Op};
use crate::commands::{CommandReply, CommandSpec};
use crate::file_search::FileMatch;
use crate::key_hint;
use crate::key_hint::KeyBinding;
use crate::keymap::{self, Action};
use crate::render::renderable::FlexRenderable;
use crate::render::renderable::Renderable;
use crate::render::renderable::RenderableItem;
use crate::status_indicator_widget::StatusIndicatorWidget;
use crate::tui::FrameRequester;
use approval_overlay::ApprovalOverlay;
pub(crate) use bottom_pane_view::BottomPaneView;
use chat_composer::ChatComposer;
pub(crate) use form::{FieldId, FormEvent, FormItem, FormView, RowValue, TextField};
pub(crate) use list_selection_view::{ListSelectionView, SelectionItem, SelectionViewParams};
pub(crate) use menu_view::{MenuKey, MenuModel, MenuView};
pub(crate) use module_view::ViewFailure;
use module_view::{ModuleView, ViewFailures};
pub(crate) use multi_select_picker::{MultiSelectItem, MultiSelectPicker};
use paste_burst::PasteBurst;
pub(crate) use pending_input_preview::PendingInput;
use request_user_input::RequestUserInputOverlay;
pub(crate) use selection_popup_common::{render_menu_surface, GenericDisplayRow};
pub(crate) use textarea::{TextArea, TextAreaState};

/// How long the "press again to quit" hint stays visible.
///
/// This is shared between:
/// - `ChatWidget`: arming the double-press quit shortcut.
/// - `BottomPane`/`ChatComposer`: rendering and expiring the footer hint.
///
/// Keeping a single value ensures Ctrl+C and Ctrl+D behave identically.
pub(crate) const QUIT_SHORTCUT_TIMEOUT: Duration = Duration::from_secs(1);

/// What a key the composer handled amounts to.
#[derive(Debug, PartialEq)]
pub(crate) enum InputResult {
    /// The message the person sent, trimmed, with large pastes put back.
    Submitted(String),
    /// The message the person queued (Tab) while a turn runs.
    Queued(String),
    /// A command, with the text after its name when it takes any.
    Command(&'static CommandSpec, String),
    None,
}

/// The result of offering a cancellation key to a bottom-pane surface.
///
/// This is primarily used for Ctrl+C routing: active views can consume the key to dismiss
/// themselves, and the caller can decide what higher-level action (if any) to take when the key is
/// not handled locally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CancellationEvent {
    Handled,
    NotHandled,
}

pub(crate) struct BottomPane {
    composer: ChatComposer,

    view_stack: Vec<Box<dyn BottomPaneView>>,

    app_event_tx: AppEventSender,
    frame_requester: FrameRequester,

    has_input_focus: bool,
    is_task_running: bool,

    status: Option<StatusIndicatorWidget>,
    /// The messages waiting for the turn, shown above the composer.
    pending_input: PendingInput,

    /// What the commands' views reported when they panicked.
    view_failures: ViewFailures,
}

pub(crate) struct BottomPaneParams {
    pub(crate) app_event_tx: AppEventSender,
    pub(crate) frame_requester: FrameRequester,
    pub(crate) has_input_focus: bool,
    pub(crate) enhanced_keys_supported: bool,
    pub(crate) placeholder_text: String,
}

impl BottomPane {
    pub(crate) fn new(params: BottomPaneParams) -> Self {
        let BottomPaneParams {
            app_event_tx,
            frame_requester,
            has_input_focus,
            enhanced_keys_supported,
            placeholder_text,
        } = params;
        Self {
            composer: ChatComposer::new(
                has_input_focus,
                app_event_tx.clone(),
                enhanced_keys_supported,
                placeholder_text,
            ),
            view_stack: Vec::new(),
            app_event_tx,
            frame_requester,
            has_input_focus,
            is_task_running: false,
            status: None,
            pending_input: PendingInput::default(),
            view_failures: ViewFailures::default(),
        }
    }

    pub(crate) fn status_widget(&self) -> Option<&StatusIndicatorWidget> {
        self.status.as_ref()
    }

    /// Whether the open view takes every key as it is (`/keymap`'s
    /// capture): no binding of the chat or the app applies.
    pub(crate) fn captures_keys(&self) -> bool {
        self.active_view()
            .is_some_and(BottomPaneView::captures_keys)
    }

    fn active_view(&self) -> Option<&dyn BottomPaneView> {
        self.view_stack.last().map(std::convert::AsRef::as_ref)
    }

    fn push_view(&mut self, view: Box<dyn BottomPaneView>) {
        self.view_stack.push(view);
        self.request_redraw();
    }

    pub(crate) fn handle_key_event(&mut self, key_event: KeyEvent) -> InputResult {
        // If a modal/view is active, handle it here; otherwise forward to composer.
        if let Some(view) = self.view_stack.last_mut() {
            view.handle_key_event(key_event);
            if view.is_complete() {
                self.view_stack.pop();
                self.on_active_view_complete();
            }
            self.request_redraw();
            return InputResult::None;
        }
        // While a task runs Esc interrupts it, even while the answer streams
        // and the status line is hidden; with a popup open, Esc closes the
        // popup.
        if keymap::matches(Action::InterruptTurn, &key_event)
            && self.is_task_running
            && !self.composer.popup_active()
        {
            match &self.status {
                Some(status) => status.interrupt(),
                None => self.app_event_tx.send(AppEvent::Op(Op::Interrupt)),
            }
            self.request_redraw();
            return InputResult::None;
        }
        let (input_result, needs_redraw) = self.composer.handle_key_event(key_event);
        if needs_redraw {
            self.request_redraw();
        }
        if self.composer.is_in_paste_burst() {
            self.request_redraw_in(PasteBurst::recommended_flush_delay());
        }
        input_result
    }

    pub(crate) fn on_ctrl_c(&mut self) -> CancellationEvent {
        if let Some(view) = self.view_stack.last_mut() {
            let event = view.on_ctrl_c();
            if matches!(event, CancellationEvent::Handled) {
                if view.is_complete() {
                    self.view_stack.pop();
                    self.on_active_view_complete();
                }
                self.show_quit_shortcut_hint(key_hint::ctrl(KeyCode::Char('c')));
                self.request_redraw();
            }
            event
        } else if self.composer_is_empty() {
            CancellationEvent::NotHandled
        } else {
            self.composer.clear_for_ctrl_c();
            self.show_quit_shortcut_hint(key_hint::ctrl(KeyCode::Char('c')));
            self.request_redraw();
            CancellationEvent::Handled
        }
    }

    /// Pasted text goes to the open view (a command's form takes it, the
    /// modals take none), or to the composer.
    pub(crate) fn handle_paste(&mut self, pasted: String) {
        let taken = match self.view_stack.last_mut() {
            Some(view) => view.handle_paste(pasted),
            None => self.composer.handle_paste(pasted),
        };
        if taken {
            self.request_redraw();
        }
    }

    /// Inserts `text` at the composer's cursor (`/mention` inserts `@`).
    pub(crate) fn insert_str(&mut self, text: &str) {
        self.composer.insert_str(text);
        self.request_redraw();
    }

    pub(crate) fn on_file_search_result(&mut self, query: String, matches: Vec<FileMatch>) {
        self.composer.on_file_search_result(query, matches);
        self.request_redraw();
    }

    /// Shows a list to pick from over the composer, as `command`'s view.
    pub(crate) fn show_command_selection_view(
        &mut self,
        command: &'static str,
        params: SelectionViewParams,
    ) {
        let view = ListSelectionView::new(params, self.app_event_tx.clone());
        self.push_view(Box::new(ModuleView::new(
            command,
            Box::new(view),
            self.view_failures.clone(),
        )));
    }

    /// Shows `view` over the composer, as `command`'s, in place of the
    /// command's view already open.
    pub(crate) fn show_command_view(
        &mut self,
        command: &'static str,
        view: Box<dyn BottomPaneView>,
    ) {
        self.view_stack
            .retain(|open| open.command() != Some(command));
        self.push_view(Box::new(ModuleView::new(
            command,
            view,
            self.view_failures.clone(),
        )));
    }

    /// Whether a view of `command` is open.
    pub(crate) fn has_command_view(&self, command: &str) -> bool {
        self.view_stack
            .iter()
            .any(|view| view.command() == Some(command))
    }

    /// `command`'s view, when it is the one open (the settings center
    /// shows it as its page).
    pub(crate) fn command_view(&self, command: &str) -> Option<&dyn BottomPaneView> {
        self.active_view()
            .filter(|view| view.command() == Some(command))
    }

    /// Hands `reply` to `command`'s latest view; it comes back when no
    /// view of the command takes it.
    pub(crate) fn offer_command_reply(
        &mut self,
        command: &'static str,
        reply: CommandReply,
    ) -> Option<CommandReply> {
        let Some(view) = self
            .view_stack
            .iter_mut()
            .rev()
            .find(|view| view.command() == Some(command))
        else {
            return Some(reply);
        };
        let left = view.take_command_reply(reply);
        self.remove_completed_views();
        self.request_redraw();
        left
    }

    /// The commands whose views panicked since the last call; their views
    /// are dropped.
    pub(crate) fn take_view_failures(&mut self) -> Vec<ViewFailure> {
        let failures = std::mem::take(&mut *self.view_failures.borrow_mut());
        if !failures.is_empty() {
            self.remove_completed_views();
        }
        failures
    }

    /// Turns `/name` off for the rest of the run: its views close, the
    /// popup leaves it out, and typing it says it is off.
    pub(crate) fn disable_command(&mut self, name: &'static str) {
        self.composer.disable_command(name);
        let before = self.view_stack.len();
        self.view_stack.retain(|view| view.command() != Some(name));
        if self.view_stack.len() != before && self.view_stack.is_empty() {
            self.on_active_view_complete();
        }
        self.request_redraw();
    }

    pub(crate) fn command_disabled(&self, name: &str) -> bool {
        self.composer.command_disabled(name)
    }

    /// Limits the commands the composer runs to `scope`.
    pub(crate) fn set_command_scope(&mut self, scope: Option<&'static [&'static str]>) {
        self.composer.set_command_scope(scope);
    }

    pub(crate) fn enhanced_keys_supported(&self) -> bool {
        self.composer.enhanced_keys_supported()
    }

    pub(crate) fn set_composer_text(&mut self, text: String) {
        self.composer.set_text_content(text);
        self.request_redraw();
    }

    /// Puts messages that did not reach the turn back in the composer,
    /// before the draft.
    pub(crate) fn restore_draft(&mut self, text: String) {
        self.composer.restore_draft(text);
        self.request_redraw();
    }

    pub(crate) fn set_pending_input(&mut self, pending_input: PendingInput) {
        if self.pending_input != pending_input {
            self.pending_input = pending_input;
            self.request_redraw();
        }
    }

    pub(crate) fn update_status(&mut self, header: String, details: Option<String>) {
        if let Some(status) = self.status.as_mut() {
            status.update_header(header);
            status.update_details(details);
            self.request_redraw();
        }
    }

    pub(crate) fn show_quit_shortcut_hint(&mut self, key: KeyBinding) {
        self.composer
            .show_quit_shortcut_hint(key, self.has_input_focus);
        // The hint expires with no input to redraw for it.
        let frame_requester = self.frame_requester.clone();
        std::thread::spawn(move || {
            std::thread::sleep(QUIT_SHORTCUT_TIMEOUT);
            frame_requester.schedule_frame();
        });
        self.request_redraw();
    }

    pub(crate) fn show_esc_backtrack_hint(&mut self) {
        self.composer.show_esc_backtrack_hint();
        self.request_redraw();
    }

    pub(crate) fn clear_esc_backtrack_hint(&mut self) {
        self.composer.clear_esc_backtrack_hint();
        self.request_redraw();
    }

    pub(crate) fn clear_quit_shortcut_hint(&mut self) {
        self.composer.clear_quit_shortcut_hint(self.has_input_focus);
        self.request_redraw();
    }

    /// The turn's thinking time, for the status line.
    pub(crate) fn update_reasoning(&mut self, reasoning: &AgentReasoningSnapshot) {
        if let Some(status) = self.status.as_mut() {
            status.update_reasoning(reasoning);
            self.request_redraw();
        }
    }

    /// The status line under the composer; `None` for the shortcuts hint.
    pub(crate) fn set_status_line(&mut self, line: Option<Line<'static>>) {
        self.composer.set_status_line(line);
        self.request_redraw();
    }

    pub(crate) fn set_task_running(&mut self, running: bool) {
        let was_running = self.is_task_running;
        self.is_task_running = running;
        self.composer.set_task_running(running);

        if running {
            if !was_running {
                self.ensure_status_indicator();
                if let Some(status) = self.status.as_mut() {
                    status.set_interrupt_hint_visible(true);
                }
                self.request_redraw();
            }
        } else {
            // Hide the status indicator when a task completes, but keep other modal views.
            self.hide_status_indicator();
        }
    }

    pub(crate) fn hide_status_indicator(&mut self) {
        if self.status.take().is_some() {
            self.request_redraw();
        }
    }

    pub(crate) fn ensure_status_indicator(&mut self) {
        if self.status.is_none() {
            self.status = Some(StatusIndicatorWidget::new(
                self.app_event_tx.clone(),
                self.frame_requester.clone(),
                true,
            ));
            self.request_redraw();
        }
    }

    pub(crate) fn set_interrupt_hint_visible(&mut self, visible: bool) {
        if let Some(status) = self.status.as_mut() {
            status.set_interrupt_hint_visible(visible);
            self.request_redraw();
        }
    }

    pub(crate) fn composer_is_empty(&self) -> bool {
        self.composer.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn composer_text(&self) -> String {
        self.composer.current_text_with_pending()
    }

    pub(crate) fn is_task_running(&self) -> bool {
        self.is_task_running
    }

    pub(crate) fn no_modal_or_popup_active(&self) -> bool {
        self.view_stack.is_empty() && !self.composer.popup_active()
    }

    pub(crate) fn push_approval_request(&mut self, request: ApprovalRequest) {
        let request = match self.view_stack.last_mut() {
            Some(view) => match view.try_consume_approval_request(request) {
                Some(request) => request,
                None => {
                    self.request_redraw();
                    return;
                }
            },
            None => request,
        };

        // Otherwise create a new approval modal overlay.
        let modal = ApprovalOverlay::new(request, self.app_event_tx.clone());
        self.pause_status_timer_for_modal();
        self.push_view(Box::new(modal));
    }

    pub(crate) fn dismiss_approval(&mut self, request_id: ApprovalRequestId) {
        for view in &mut self.view_stack {
            view.dismiss_approval(request_id);
        }
        self.remove_completed_views();
    }

    pub(crate) fn push_user_input_request(&mut self, request: AgentUserInputRequest) {
        let request = match self.view_stack.last_mut() {
            Some(view) => match view.try_consume_user_input_request(request) {
                Some(request) => request,
                None => {
                    self.request_redraw();
                    return;
                }
            },
            None => request,
        };

        let modal = RequestUserInputOverlay::new(request, self.app_event_tx.clone());
        self.pause_status_timer_for_modal();
        self.push_view(Box::new(modal));
    }

    pub(crate) fn dismiss_user_input(&mut self, request_id: AgentUserInputRequestId) {
        for view in &mut self.view_stack {
            view.dismiss_user_input(request_id);
        }
        self.remove_completed_views();
    }

    /// The turn ended: whatever it was waiting for is no longer asked. The
    /// commands' views stay.
    pub(crate) fn dismiss_all_requests(&mut self) {
        let before = self.view_stack.len();
        self.view_stack.retain(|view| view.command().is_some());
        if self.view_stack.len() != before {
            if self.view_stack.is_empty() {
                self.on_active_view_complete();
            }
            self.request_redraw();
        }
    }

    fn remove_completed_views(&mut self) {
        let before = self.view_stack.len();
        self.view_stack.retain(|view| !view.is_complete());
        if self.view_stack.len() != before {
            if self.view_stack.is_empty() {
                self.on_active_view_complete();
            }
            self.request_redraw();
        }
    }

    fn on_active_view_complete(&mut self) {
        if let Some(status) = self.status.as_mut() {
            status.resume_timer();
        }
    }

    fn pause_status_timer_for_modal(&mut self) {
        if let Some(status) = self.status.as_mut() {
            status.pause_timer();
        }
    }

    fn request_redraw(&self) {
        self.frame_requester.schedule_frame();
    }

    fn request_redraw_in(&self, dur: Duration) {
        self.frame_requester.schedule_frame_in(dur);
    }

    pub(crate) fn flush_paste_burst_if_due(&mut self) -> bool {
        self.composer.flush_paste_burst_if_due()
    }

    pub(crate) fn is_in_paste_burst(&self) -> bool {
        self.composer.is_in_paste_burst()
    }

    /// When to look again at a paste burst being captured.
    pub(crate) fn paste_burst_flush_delay() -> Duration {
        PasteBurst::recommended_flush_delay()
    }

    fn as_renderable(&'_ self) -> RenderableItem<'_> {
        if let Some(view) = self.active_view() {
            RenderableItem::Borrowed(view)
        } else {
            let mut flex = FlexRenderable::new();
            if let Some(status) = &self.status {
                flex.push(0, RenderableItem::Borrowed(status));
                flex.push(0, RenderableItem::Owned("".into()));
            }
            if !self.pending_input.is_empty() {
                flex.push(0, RenderableItem::Borrowed(&self.pending_input));
                flex.push(0, RenderableItem::Owned("".into()));
            }
            let mut flex2 = FlexRenderable::new();
            flex2.push(1, RenderableItem::Owned(flex.into()));
            flex2.push(0, RenderableItem::Borrowed(&self.composer));
            RenderableItem::Owned(Box::new(flex2))
        }
    }
}

impl Renderable for BottomPane {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        self.as_renderable().render(area, buf);
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
