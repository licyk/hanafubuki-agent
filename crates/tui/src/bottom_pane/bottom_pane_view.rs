// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

use hanafubuki_agent::{
    AgentUserInputRequest, AgentUserInputRequestId, ApprovalRequest, ApprovalRequestId,
};
use ratatui::crossterm::event::KeyEvent;

use super::CancellationEvent;
use crate::render::renderable::Renderable;

/// Trait implemented by every view that can be shown in the bottom pane.
pub(crate) trait BottomPaneView: Renderable {
    /// Handle a key event while the view is active. A redraw is always
    /// scheduled after this call.
    fn handle_key_event(&mut self, _key_event: KeyEvent) {}

    /// Return `true` if the view has finished and should be removed.
    fn is_complete(&self) -> bool {
        false
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
}
