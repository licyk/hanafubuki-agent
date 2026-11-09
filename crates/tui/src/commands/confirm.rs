// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A yes-or-no question before a command does what cannot be taken back
//! (`/archive`, `/delete`, `/stop`), Codex's confirmation popup: "no"
//! comes first and is selected.

use super::{send_session_call, CommandContext};
use crate::agent_bridge::SessionCall;
use crate::bottom_pane::{SelectionItem, SelectionViewParams};

pub(super) struct Confirmation {
    pub(super) title: &'static str,
    pub(super) subtitle: &'static str,
    pub(super) no: &'static str,
    pub(super) yes: &'static str,
    pub(super) yes_description: &'static str,
    /// What "yes" asks of the Agent.
    pub(super) call: SessionCall,
}

pub(super) fn confirm(context: &mut CommandContext<'_>, confirmation: Confirmation) {
    let call = confirmation.call;
    context.show_selection_view(SelectionViewParams {
        title: Some(confirmation.title.to_string()),
        subtitle: Some(confirmation.subtitle.to_string()),
        items: vec![
            SelectionItem {
                name: confirmation.no.to_string(),
                description: Some("Change nothing".to_string()),
                dismiss_on_select: true,
                ..Default::default()
            },
            SelectionItem {
                name: confirmation.yes.to_string(),
                description: Some(confirmation.yes_description.to_string()),
                actions: vec![Box::new(move |tx| send_session_call(tx, call.clone()))],
                dismiss_on_select: true,
                ..Default::default()
            },
        ],
        ..Default::default()
    });
}
