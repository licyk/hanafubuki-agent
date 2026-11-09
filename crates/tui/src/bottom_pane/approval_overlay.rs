// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! The modal that asks the person to approve what the turn is about to do.
//! Codex builds one per kind of request (a command, a patch, an MCP
//! elicitation); the Agent's requests share one shape, an action with a kind,
//! a risk and a summary, so the kind picks the question and the summary is
//! what is shown.

use hanafubuki_agent::{
    ApprovalActionKind, ApprovalRequest, ApprovalRequestId, ApprovalUserDecision,
};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::KeyCode;
use ratatui::crossterm::event::KeyEvent;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Wrap;

use crate::app_event::{AppEvent, AppEventSender, Op};
use crate::bottom_pane::list_selection_view::ListSelectionView;
use crate::bottom_pane::list_selection_view::SelectionItem;
use crate::bottom_pane::list_selection_view::SelectionViewParams;
use crate::bottom_pane::BottomPaneView;
use crate::bottom_pane::CancellationEvent;
use crate::history_cell;
use crate::key_hint;
use crate::key_hint::KeyBinding;
use crate::render::renderable::ColumnRenderable;
use crate::render::renderable::Renderable;
use crate::text_formatting::strip_terminal_controls;

/// Modal overlay asking the user to approve or deny one or more requests.
pub(crate) struct ApprovalOverlay {
    current_request: Option<ApprovalRequest>,
    queue: Vec<ApprovalRequest>,
    app_event_tx: AppEventSender,
    list: ListSelectionView,
    options: Vec<ApprovalOption>,
    current_complete: bool,
    done: bool,
}

impl ApprovalOverlay {
    pub fn new(request: ApprovalRequest, app_event_tx: AppEventSender) -> Self {
        let mut view = Self {
            current_request: None,
            queue: Vec::new(),
            app_event_tx: app_event_tx.clone(),
            list: ListSelectionView::new(Default::default(), app_event_tx),
            options: Vec::new(),
            current_complete: false,
            done: false,
        };
        view.set_current(request);
        view
    }

    pub fn enqueue_request(&mut self, req: ApprovalRequest) {
        self.queue.push(req);
    }

    fn set_current(&mut self, request: ApprovalRequest) {
        self.current_complete = false;
        let (options, params) = Self::build_options(&request);
        self.options = options;
        self.list = ListSelectionView::new(params, self.app_event_tx.clone());
        self.current_request = Some(request);
    }

    fn build_options(request: &ApprovalRequest) -> (Vec<ApprovalOption>, SelectionViewParams) {
        let options = approval_options(
            request.offers_allow_for_session,
            request.action.sandbox_escalation.is_some(),
        );
        let risk = format!("{:?}", request.action.risk).to_lowercase();
        let header = Box::new(ColumnRenderable::with([
            Line::from(question(request.action.kind).bold()).into(),
            Line::from("").into(),
            Box::new(
                Paragraph::new(Line::from(strip_terminal_controls(&request.action.summary)))
                    .wrap(Wrap { trim: false }),
            ) as Box<dyn Renderable>,
            Line::from(vec!["Risk: ".dim(), risk.into()]).into(),
        ]));

        let items = options
            .iter()
            .map(|opt| SelectionItem {
                name: opt.label.to_string(),
                display_shortcut: opt
                    .display_shortcut
                    .or_else(|| opt.additional_shortcuts.first().copied()),
                dismiss_on_select: false,
                ..Default::default()
            })
            .collect();

        let params = SelectionViewParams {
            footer_hint: Some(Line::from(vec![
                "Press ".into(),
                key_hint::plain(KeyCode::Enter).into(),
                " to confirm or ".into(),
                key_hint::plain(KeyCode::Esc).into(),
                " to cancel".into(),
            ])),
            items,
            header,
            ..Default::default()
        };

        (options, params)
    }

    fn apply_selection(&mut self, actual_idx: usize) {
        if self.current_complete {
            return;
        }
        let Some(decision) = self.options.get(actual_idx).map(|option| option.decision) else {
            return;
        };
        self.send_decision(decision);
        self.current_complete = true;
        self.advance_queue();
    }

    fn send_decision(&self, decision: ApprovalUserDecision) {
        let Some(request) = self.current_request.as_ref() else {
            return;
        };
        let approved = !matches!(decision, ApprovalUserDecision::Deny);
        let cell = history_cell::new_approval_decision_cell(
            &strip_terminal_controls(&request.action.summary),
            Some(approved),
        );
        self.app_event_tx
            .send(AppEvent::InsertHistoryCell(Box::new(cell)));
        self.app_event_tx.send(AppEvent::Op(Op::Approval {
            request_id: request.request_id,
            decision,
        }));
    }

    /// The next request in the order they were asked.
    fn advance_queue(&mut self) {
        if self.queue.is_empty() {
            self.done = true;
        } else {
            let next = self.queue.remove(0);
            self.set_current(next);
        }
    }

    fn try_handle_shortcut(&mut self, key_event: &KeyEvent) -> bool {
        if let Some(idx) = self
            .options
            .iter()
            .position(|opt| opt.shortcuts().any(|s| s.is_press(*key_event)))
        {
            self.apply_selection(idx);
            true
        } else {
            false
        }
    }
}

impl BottomPaneView for ApprovalOverlay {
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        if self.try_handle_shortcut(&key_event) {
            return;
        }
        self.list.handle_key_event(key_event);
        if let Some(idx) = self.list.take_last_selected_index() {
            self.apply_selection(idx);
        }
    }

    /// Denies the current request and every queued one: the turn should not
    /// go on waiting for answers nobody will give.
    fn on_ctrl_c(&mut self) -> CancellationEvent {
        if self.done {
            return CancellationEvent::Handled;
        }
        if !self.current_complete {
            self.send_decision(ApprovalUserDecision::Deny);
        }
        for request in std::mem::take(&mut self.queue) {
            self.set_current(request);
            self.send_decision(ApprovalUserDecision::Deny);
        }
        self.done = true;
        CancellationEvent::Handled
    }

    fn is_complete(&self) -> bool {
        self.done
    }

    fn try_consume_approval_request(
        &mut self,
        request: ApprovalRequest,
    ) -> Option<ApprovalRequest> {
        self.enqueue_request(request);
        None
    }

    fn dismiss_approval(&mut self, request_id: ApprovalRequestId) {
        self.queue
            .retain(|request| request.request_id != request_id);
        let is_current = self
            .current_request
            .as_ref()
            .is_some_and(|request| request.request_id == request_id);
        if is_current && !self.current_complete {
            self.current_complete = true;
            self.advance_queue();
        }
    }
}

impl Renderable for ApprovalOverlay {
    fn desired_height(&self, width: u16) -> u16 {
        self.list.desired_height(width)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        self.list.render(area, buf);
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        self.list.cursor_pos(area)
    }
}

/// What the modal asks for an action of `kind`.
fn question(kind: ApprovalActionKind) -> &'static str {
    match kind {
        ApprovalActionKind::CommandExecution
        | ApprovalActionKind::ShellExecution
        | ApprovalActionKind::CodeExecution
        | ApprovalActionKind::InteractiveExecution => {
            "Would you like to run the following command?"
        }
        ApprovalActionKind::FileWrite
        | ApprovalActionKind::FilePatch
        | ApprovalActionKind::FileMove
        | ApprovalActionKind::FileDelete => "Would you like to make the following edits?",
        ApprovalActionKind::FileRead => "Would you like to allow reading the following?",
        ApprovalActionKind::McpToolCall => "Would you like to allow this tool call?",
        ApprovalActionKind::SkillManagement => "Would you like to allow this Skill change?",
        ApprovalActionKind::DomainMutation => "Would you like to allow this action?",
    }
}

#[derive(Clone)]
struct ApprovalOption {
    label: &'static str,
    decision: ApprovalUserDecision,
    display_shortcut: Option<KeyBinding>,
    additional_shortcuts: Vec<KeyBinding>,
}

impl ApprovalOption {
    fn shortcuts(&self) -> impl Iterator<Item = KeyBinding> + '_ {
        self.display_shortcut
            .into_iter()
            .chain(self.additional_shortcuts.iter().copied())
    }
}

/// An escalated command's session allowance covers that command only
/// (outside the sandbox), not the tool.
fn approval_options(offers_allow_for_session: bool, escalated: bool) -> Vec<ApprovalOption> {
    let mut options = vec![ApprovalOption {
        label: "Yes, proceed",
        decision: ApprovalUserDecision::AllowOnce,
        display_shortcut: None,
        additional_shortcuts: vec![key_hint::plain(KeyCode::Char('y'))],
    }];
    if offers_allow_for_session {
        options.push(ApprovalOption {
            label: if escalated {
                "Yes, and don't ask again for this command this session"
            } else {
                "Yes, and don't ask again this session"
            },
            decision: ApprovalUserDecision::AllowForSession,
            display_shortcut: None,
            additional_shortcuts: vec![key_hint::plain(KeyCode::Char('a'))],
        });
    }
    options.push(ApprovalOption {
        label: "No",
        decision: ApprovalUserDecision::Deny,
        display_shortcut: Some(key_hint::plain(KeyCode::Esc)),
        additional_shortcuts: vec![key_hint::plain(KeyCode::Char('n'))],
    });
    options
}

#[cfg(test)]
mod tests;
