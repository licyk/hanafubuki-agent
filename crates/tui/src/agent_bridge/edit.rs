//! Editing an earlier message: the prompt chosen in the transcript is
//! found among the session's saved turns, and the facade forks the session
//! before that turn and archives the original (`edit_from_turn`); the
//! interface then shows the fork, with the prompt back in the composer.
//!
//! The transcript shows the latest items (older ones may not be loaded),
//! so a prompt is counted from the latest: the first one is the session's
//! latest message, and each prompt shown before the chosen one must match
//! a saved message, so a prompt that never reached the session (a turn
//! refused at its start) fails the mapping instead of picking the wrong
//! turn. Only messages show as prompts: continuation context, side and
//! approval notes and compaction summaries are skipped. A message sent
//! while the Agent worked (steering) is a prompt but not a turn of its
//! own, so it cannot be edited.

use hanafubuki_agent::{
    AgentHistorySortDirection, AgentSessionId, AgentTranscriptPageRequest, AgentTurnId,
    ConversationItem,
};

use super::{AgentBridge, OpenedSession};

/// The interface after an edit: the fork, and the message to edit.
#[derive(Debug)]
pub(crate) struct EditedPrompt {
    pub(crate) opened: OpenedSession,
    pub(crate) prompt: String,
    /// The images the message had, which the composer cannot attach.
    pub(crate) images: usize,
}

impl AgentBridge {
    /// Edits the prompt `prompts` ends with; `prompts` are the prompts the
    /// transcript shows, latest first, down to the chosen one.
    pub(crate) fn edit_prompt(&self, prompts: &[String]) -> Result<EditedPrompt, String> {
        let session_id = self
            .session_id
            .ok_or_else(|| "the conversation is not saved yet".to_string())?;
        let turn_id = self.prompt_turn(session_id, prompts)?;
        let edited = self.agent.edit_from_turn(session_id, turn_id)?;
        let opened = self.open(edited.session.id)?;
        Ok(EditedPrompt {
            opened,
            prompt: edited.restored_prompt,
            images: edited.restored_images.len(),
        })
    }

    /// The turn the last of `prompts` started, read from the session's
    /// transcript, latest first.
    fn prompt_turn(
        &self,
        session_id: AgentSessionId,
        prompts: &[String],
    ) -> Result<AgentTurnId, String> {
        let mut scan = PromptScan::new(prompts);
        let mut cursor = None;
        let mut through_revision = None;
        loop {
            let page = self
                .agent
                .read_transcript_page(AgentTranscriptPageRequest {
                    session_id,
                    page_size: self.page_size,
                    cursor,
                    direction: AgentHistorySortDirection::Descending,
                    through_revision,
                })
                .map_err(|error| format!("the conversation was not read: {error}"))?;
            for item in &page.items {
                if let Some(turn_id) = scan.push(&item.content)? {
                    return Ok(turn_id);
                }
            }
            through_revision = Some(page.through_revision);
            cursor = Some(
                page.older_cursor
                    .ok_or_else(|| "the message is not in the saved conversation".to_string())?,
            );
        }
    }
}

/// Finds the turn of a prompt in a session's items read latest first.
pub(super) struct PromptScan<'a> {
    /// The prompts shown, latest first, down to the chosen one.
    prompts: &'a [String],
    /// How many of them were matched.
    matched: usize,
}

impl<'a> PromptScan<'a> {
    pub(super) fn new(prompts: &'a [String]) -> Self {
        Self {
            prompts,
            matched: 0,
        }
    }

    /// The next item, older than the last; the chosen prompt's turn once
    /// its start is reached.
    pub(super) fn push(&mut self, item: &ConversationItem) -> Result<Option<AgentTurnId>, String> {
        let chosen = self.matched == self.prompts.len();
        match item {
            ConversationItem::TurnStart { turn_id } if chosen => Ok(Some(*turn_id)),
            ConversationItem::UserMessage { text, .. }
            | ConversationItem::SteeringMessage { text, .. }
                if !chosen =>
            {
                if self.prompts.get(self.matched) != Some(text) {
                    return Err(
                        "the transcript no longer matches the saved conversation".to_string()
                    );
                }
                self.matched += 1;
                if self.matched == self.prompts.len()
                    && matches!(item, ConversationItem::SteeringMessage { .. })
                {
                    return Err(
                        "a message sent while the Agent worked cannot be edited on its \
                        own: edit the message that started that turn"
                            .to_string(),
                    );
                }
                Ok(None)
            }
            _ => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests;
