use crate::conversation::AgentHistoryItem;
use crate::model::{AgentError, ConversationItem};
use crate::service::state::exact_turn;
use crate::service::{AgentService, TurnIdentity};

impl AgentService {
    pub(super) fn skill_is_active_in_turn(
        &self,
        identity: TurnIdentity,
        skill: &hanafubuki_agent_skills::SkillId,
    ) -> Result<bool, AgentError> {
        let cell = self.session(identity.session_id)?;
        let guard = cell.lock()?;
        let record = &*guard;
        exact_turn(record, identity.turn_id, identity.generation)?;
        Ok(turn_has_skill_context(&record.conversation.history, skill))
    }
}

pub(super) fn turn_has_skill_context<T: std::borrow::Borrow<AgentHistoryItem>>(
    history: &[T],
    skill: &hanafubuki_agent_skills::SkillId,
) -> bool {
    history
        .iter()
        .rev()
        .take_while(|item| !matches!(&item.borrow().content, ConversationItem::TurnStart { .. }))
        .any(|item| {
            matches!(
                &item.borrow().content,
                ConversationItem::SkillContext { skill: active, .. } if active == skill
            )
        })
}

pub(in crate::service) fn active_skills<T: std::borrow::Borrow<AgentHistoryItem>>(
    history: &[T],
) -> Vec<hanafubuki_agent_skills::SkillId> {
    let mut skills = history
        .iter()
        .rev()
        .take_while(|item| !matches!(&item.borrow().content, ConversationItem::TurnStart { .. }))
        .filter_map(|item| match &item.borrow().content {
            ConversationItem::SkillContext { skill, .. } => Some(skill.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    skills.sort();
    skills.dedup();
    skills
}
