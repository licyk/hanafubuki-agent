// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The list `/subagents` opens (Codex's agent picker): each subagent with
//! its status, model and reasoning effort and the start of its latest
//! answer, kept current while the watch reads new statuses; Enter shows
//! the one selected.

use hanafubuki_agent::AgentSubagentStatus;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::KeyEvent;
use ratatui::layout::Rect;

use super::{runs_on, single_line, state_name};
use crate::app::WorkspaceEvent;
use crate::app_event::{AppEvent, AppEventSender};
use crate::bottom_pane::{
    BottomPaneView, CancellationEvent, ListSelectionView, SelectionItem, SelectionViewParams,
};
use crate::commands::CommandReply;
use crate::render::renderable::Renderable;
use crate::text_formatting::truncate_text;

/// How much of a latest answer an entry shows.
const ANSWER_CHARS: usize = 60;

pub(super) struct SubagentsPicker {
    list: ListSelectionView,
    app_event_tx: AppEventSender,
}

impl SubagentsPicker {
    pub(super) fn new(statuses: Vec<AgentSubagentStatus>, app_event_tx: AppEventSender) -> Self {
        Self {
            list: list(&statuses, None, &app_event_tx),
            app_event_tx,
        }
    }
}

fn list(
    statuses: &[AgentSubagentStatus],
    selected: Option<usize>,
    app_event_tx: &AppEventSender,
) -> ListSelectionView {
    let items = statuses
        .iter()
        .map(|status| {
            let session_id = status.session_id;
            let answer = status
                .last_answer
                .as_deref()
                .map(|answer| truncate_text(&single_line(answer), ANSWER_CHARS));
            let state = format!("{} · {}", state_name(status.status), runs_on(status));
            let description = match answer {
                Some(answer) => format!("{state} · {answer}"),
                None => state,
            };
            SelectionItem {
                name: status.title.clone(),
                description: Some(description),
                actions: vec![Box::new(move |tx: &AppEventSender| {
                    tx.send(AppEvent::Workspace(WorkspaceEvent::OpenChild(session_id)));
                })],
                dismiss_on_select: true,
                ..Default::default()
            }
        })
        .collect();
    ListSelectionView::new(
        SelectionViewParams {
            title: Some("Subagents".to_string()),
            subtitle: Some("Enter shows one in place of this conversation".to_string()),
            items,
            initial_selected_idx: selected,
            ..Default::default()
        },
        app_event_tx.clone(),
    )
}

impl BottomPaneView for SubagentsPicker {
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        self.list.handle_key_event(key_event);
    }

    fn is_complete(&self) -> bool {
        self.list.is_complete()
    }

    fn take_command_reply(&mut self, reply: CommandReply) -> Option<CommandReply> {
        let CommandReply::Subagents(statuses) = reply else {
            return Some(reply);
        };
        let selected = self.list.selected_index();
        self.list = list(&statuses, selected, &self.app_event_tx);
        None
    }

    fn on_ctrl_c(&mut self) -> CancellationEvent {
        self.list.on_ctrl_c()
    }
}

impl Renderable for SubagentsPicker {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        self.list.render(area, buf);
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.list.desired_height(width)
    }
}
