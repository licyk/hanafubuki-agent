use super::*;

fn skill(name: &str) -> hanafubuki_agent_skills::SkillId {
    hanafubuki_agent_skills::SkillId::new(hanafubuki_agent_skills::SkillScope::User, name).unwrap()
}

fn history_item(item_id: u64, content: ConversationItem) -> AgentHistoryItem {
    AgentHistoryItem {
        source_session_id: AgentSessionId::new(1),
        item_id: AgentHistoryItemId::new(item_id),
        item_revision: 0,
        turn_id: None,
        logical_segment: 0,
        logical_order: item_id,
        turn_working_millis: None,
        content,
    }
}

#[test]
fn implicit_skill_activation_is_idempotent_only_within_the_current_turn() {
    let active = skill("workflow");
    let history = vec![
        history_item(
            1,
            ConversationItem::TurnStart {
                turn_id: AgentTurnId::new(1),
            },
        ),
        history_item(
            2,
            ConversationItem::SkillContext {
                skill: active.clone(),
                name: "active".to_string(),
                content: "first".to_string(),
            },
        ),
        history_item(
            3,
            ConversationItem::TurnStart {
                turn_id: AgentTurnId::new(2),
            },
        ),
        history_item(
            4,
            ConversationItem::UserMessage {
                text: "next".to_string(),
                images: Vec::new(),
            },
        ),
    ];
    assert!(!turn_has_skill_context(&history, &active));
    let mut current = history;
    current.push(history_item(
        5,
        ConversationItem::SkillContext {
            skill: active.clone(),
            name: "active".to_string(),
            content: "second".to_string(),
        },
    ));
    assert!(turn_has_skill_context(&current, &active));
    assert!(!turn_has_skill_context(&current, &skill("other")));
}
