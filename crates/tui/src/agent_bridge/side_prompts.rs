// Modified from OpenAI Codex cf3ff1530d82 (Apache-2.0).

use hanafubuki_agent::{
    AgentHandle, AgentHistorySortDirection, AgentSessionId, AgentTranscriptPageRequest,
    ConversationItem,
};

use super::{commands::keep_side_history, AgentBridge};
use crate::{app::WorkspaceEvent, app_event::AppEvent, user_message::prompt_preview};

impl AgentBridge {
    pub(crate) fn read_side_prompts(&self, parent: AgentSessionId, ids: Vec<AgentSessionId>) {
        let sender = self.app_event_tx.untagged();
        let agent = self.agent.clone();
        let page_size = self.page_size;
        self.agent.runtime().handle().spawn_blocking(move || {
            let prompts = ids
                .into_iter()
                .map(|id| (id, latest_prompt(&agent, id, page_size)))
                .collect();
            sender.send(AppEvent::Workspace(WorkspaceEvent::SidePrompts(
                parent, prompts,
            )));
        });
    }
}

fn latest_prompt(agent: &AgentHandle, id: AgentSessionId, page_size: usize) -> Option<String> {
    let mut request = AgentTranscriptPageRequest {
        session_id: id,
        page_size,
        cursor: None,
        direction: AgentHistorySortDirection::Descending,
        through_revision: None,
    };
    loop {
        let mut page = agent.read_transcript_page(request.clone()).ok()?;
        keep_side_history(&mut page, id);
        if let Some(prompt) = page.items.into_iter().find_map(|item| match item.content {
            ConversationItem::UserMessage { text, images }
            | ConversationItem::SteeringMessage { text, images, .. } => {
                prompt_preview(&text, !images.is_empty())
            }
            _ => None,
        }) {
            return Some(prompt);
        }
        request.cursor = Some(page.older_cursor?);
        request.through_revision = Some(page.through_revision);
    }
}
