use super::*;

fn item(content: ConversationItem, id: u64) -> crate::AgentHistoryItem {
    crate::AgentHistoryItem {
        source_session_id: crate::AgentSessionId::new(1),
        item_id: crate::AgentHistoryItemId::new(id),
        item_revision: 0,
        turn_id: None,
        logical_segment: 0,
        logical_order: id,
        turn_working_millis: None,
        content,
    }
}

#[test]
fn identical_sections_are_noop_and_changed_or_removed_sections_are_explicit() {
    let first =
        AgentWorldStateSection::new(ContextSectionKind::WORKSPACE_ROOTS, "roots-a".to_string());
    let history = vec![item(
        ConversationItem::WorldStateUpdate {
            updates: vec![AgentWorldStateUpdate::Replace {
                kind: first.kind.clone(),
                version: first.version,
                content: first.content.clone(),
            }],
        },
        1,
    )];
    assert!(world_state_diff(
        &history,
        AgentWorldStateSnapshot {
            sections: vec![first.clone()],
        }
    )
    .is_empty());

    let changed =
        AgentWorldStateSection::new(ContextSectionKind::WORKSPACE_ROOTS, "roots-b".to_string());
    assert!(matches!(
        world_state_diff(
            &history,
            AgentWorldStateSnapshot {
                sections: vec![changed.clone()],
            }
        )
        .as_slice(),
        [AgentWorldStateUpdate::Replace { version, .. }] if *version == changed.version
    ));
    assert!(matches!(
        world_state_diff(&history, AgentWorldStateSnapshot::default()).as_slice(),
        [AgentWorldStateUpdate::Remove { previous_version, .. }]
            if *previous_version == first.version
    ));
}

#[test]
fn reverse_replay_checkpoint_rebuilds_only_the_latest_effective_sections() {
    let old = AgentWorldStateSection::new(
        ContextSectionKind::fixed("host_location"),
        "old".to_string(),
    );
    let new = AgentWorldStateSection::new(
        ContextSectionKind::fixed("host_location"),
        "new".to_string(),
    );
    let history = vec![
        item(
            ConversationItem::WorldStateUpdate {
                updates: vec![AgentWorldStateUpdate::Replace {
                    kind: old.kind,
                    version: old.version,
                    content: old.content,
                }],
            },
            1,
        ),
        item(
            ConversationItem::WorldStateUpdate {
                updates: vec![AgentWorldStateUpdate::Replace {
                    kind: new.kind,
                    version: new.version,
                    content: new.content.clone(),
                }],
            },
            2,
        ),
    ];
    let checkpoint = full_world_state_checkpoint(&history).unwrap();
    assert!(matches!(
        checkpoint,
        ConversationItem::WorldStateUpdate { updates }
            if matches!(updates.as_slice(), [AgentWorldStateUpdate::Replace { content, .. }] if content == "new")
    ));
}

#[test]
fn the_section_bound_has_a_slot_for_every_known_kind_and_the_plugin_kinds() {
    assert_eq!(
        super::MAXIMUM_AGENT_WORLD_STATE_SECTIONS,
        hanafubuki_agent_contracts::ContextSectionKind::KNOWN.len()
            + super::MAXIMUM_AGENT_PLUGIN_WORLD_STATE_SECTIONS
    );
}
