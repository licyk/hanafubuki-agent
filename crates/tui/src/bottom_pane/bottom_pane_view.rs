// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

use hanafubuki_agent::{
    AgentUserInputRequest, AgentUserInputRequestId, ApprovalRequest, ApprovalRequestId,
};
use ratatui::crossterm::event::KeyEvent;

use super::CancellationEvent;
use crate::commands::CommandReply;
use crate::render::renderable::Renderable;

/// Trait implemented by every view that can be shown in the bottom pane.
pub(crate) trait BottomPaneView: Renderable {
    fn pointer_key(
        &mut self,
        _mouse: ratatui::crossterm::event::MouseEvent,
    ) -> Option<ratatui::crossterm::event::KeyCode> {
        None
    }

    /// Handle a key event while the view is active. A redraw is always
    /// scheduled after this call.
    fn handle_key_event(&mut self, _key_event: KeyEvent) {}

    /// Return `true` if the view has finished and should be removed.
    fn is_complete(&self) -> bool {
        false
    }

    /// When the view next needs to act on its own, if it does: the
    /// interface wakes for it then. A view with a countdown answers here.
    fn next_deadline(&self) -> Option<std::time::Instant> {
        None
    }

    /// Act on what [`Self::next_deadline`] announced. Returns whether the
    /// view changed, so the interface redraws.
    fn on_deadline(&mut self) -> bool {
        false
    }

    /// Pasted text, while this view is active; `false` when it takes none.
    fn handle_paste(&mut self, _pasted: String) -> bool {
        false
    }

    /// What the Agent answered a command's call: the command's view takes
    /// it, or hands it back.
    fn take_command_reply(&mut self, reply: CommandReply) -> Option<CommandReply> {
        Some(reply)
    }

    /// Handle Ctrl-C while this view is active.
    fn on_ctrl_c(&mut self) -> CancellationEvent {
        CancellationEvent::NotHandled
    }

    /// Try to handle approval request; return the original value if not
    /// consumed.
    fn try_consume_approval_request(
        &mut self,
        request: ApprovalRequest,
    ) -> Option<ApprovalRequest> {
        Some(request)
    }

    /// Try to handle request_user_input; return the original value if not
    /// consumed.
    fn try_consume_user_input_request(
        &mut self,
        request: AgentUserInputRequest,
    ) -> Option<AgentUserInputRequest> {
        Some(request)
    }

    /// Drop a pending approval that was answered elsewhere (a policy, the
    /// turn's end) without answering it again.
    fn dismiss_approval(&mut self, _request_id: ApprovalRequestId) {}

    /// Drop a pending question that was answered or withdrawn elsewhere.
    fn dismiss_user_input(&mut self, _request_id: AgentUserInputRequestId) {}

    /// Whether it takes every key as it is, before any binding (`/keymap`'s
    /// capture).
    fn captures_keys(&self) -> bool {
        false
    }

    /// The slash command whose view this is; `None` for the chat's own.
    fn command(&self) -> Option<&'static str> {
        None
    }
}

impl super::BottomPane {
    pub(crate) fn pointer_key(
        &mut self,
        mouse: ratatui::crossterm::event::MouseEvent,
    ) -> Option<ratatui::crossterm::event::KeyCode> {
        if let Some(view) = self.view_stack.last_mut() {
            return view.pointer_key(mouse);
        }
        self.composer.pointer_key(mouse)
    }

    /// Lets the active view act on what it announced as due, then schedules
    /// the frame it needs next. Returns whether the view changed.
    pub(crate) fn tick_active_view(&mut self) -> bool {
        let Some(view) = self.view_stack.last_mut() else {
            return false;
        };
        let changed = view.on_deadline();
        if view.is_complete() {
            self.view_stack.pop();
            self.on_active_view_complete();
        }
        match self.view_stack.last().and_then(|view| view.next_deadline()) {
            Some(at) => self
                .frame_requester
                .schedule_frame_in(at.saturating_duration_since(std::time::Instant::now())),
            None if changed => self.request_redraw(),
            None => {}
        }
        changed
    }
}
