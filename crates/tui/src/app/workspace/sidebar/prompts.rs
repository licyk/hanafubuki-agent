// Modified from OpenAI Codex cf3ff1530d82 (Apache-2.0).

use std::time::Instant;

use hanafubuki_agent::AgentSessionId;

use super::{App, Page};
use crate::history_cell::UserHistoryCell;

impl App {
    /// The chooser's hit rectangles are the rows that actually fit in this frame.
    pub(in crate::app) fn refresh_visible_side_prompts(&mut self) {
        let sidebar = &self.workspace.sidebar;
        let Some(parent) = sidebar.owner else { return };
        if !matches!(sidebar.page, Some(Page::Chooser))
            || sidebar.close.is_some()
            || sidebar.chooser.path_visible()
        {
            return;
        }
        let missing = sidebar
            .chooser
            .visible_chat_ids()
            .filter(|id| !sidebar.prompts.contains_key(id))
            .collect::<Vec<_>>();
        let mut read = Vec::new();
        for id in missing {
            // Opening the chooser parks the live chat. Its latest typed message can
            // already be on screen while the corresponding history commit is pending.
            let tab = self.workspace.hidden.iter().find(|tab| tab.session == id);
            let prompt = tab.and_then(|tab| {
                tab.parked.transcript_cells.iter().rev().find_map(|cell| {
                    cell.as_any()
                        .downcast_ref::<UserHistoryCell>()?
                        .prompt_preview()
                })
            });
            if prompt.is_some() {
                self.workspace.sidebar.chooser_dirty = true;
                self.schedule_frame(Instant::now());
            } else if tab.is_none_or(|tab| tab.parked.older_transcript.is_some()) {
                read.push(id);
            }
            self.workspace.sidebar.prompts.insert(id, prompt);
        }
        if !read.is_empty() {
            self.main_bridge().read_side_prompts(parent, read);
        }
    }

    pub(in crate::app::workspace) fn invalidate_side_prompt(&mut self, tag: u64) {
        if let Some(tab) = self.workspace.hidden.iter().find(|tab| tab.id == tag) {
            self.workspace.sidebar.prompts.remove(&tab.session);
        }
    }

    pub(in crate::app::workspace) fn side_prompts_read(
        &mut self,
        parent: AgentSessionId,
        prompts: Vec<(AgentSessionId, Option<String>)>,
    ) {
        let sidebar = &mut self.workspace.sidebar;
        if sidebar.owner == Some(parent) {
            for (id, prompt) in prompts {
                if let Some(value @ None) = sidebar.prompts.get_mut(&id) {
                    *value = prompt;
                }
            }
        }
    }
}
