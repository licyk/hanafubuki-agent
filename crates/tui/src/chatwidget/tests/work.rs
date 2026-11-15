use super::*;
use crate::agent_bridge::WorkSnapshot;

fn work(id: &str, text: &str, done: bool) -> TurnUpdate {
    TurnUpdate::Work(WorkSnapshot {
        id: id.to_string(),
        title: "Working activity".to_string(),
        text: Some(text.to_string()),
        started_at: 1_000,
        completed_at: done.then_some(2_000),
        warning: None,
        committed: false,
    })
}

#[test]
fn concurrent_work_entries_update_in_place_and_end_once() {
    let (mut chat, events) = chat();
    chat.handle_turn_update(work("compaction:1", "partial", false));
    chat.handle_turn_update(work("permission-review:1", "", false));
    chat.handle_turn_update(work("permission-review:2", "", false));
    assert_eq!(chat.work_entries.len(), 3);
    assert!(chat.has_live_entries());
    chat.handle_turn_update(work("compaction:1", "partial summary", false));
    chat.handle_turn_update(work("compaction:1", "partial summary", true));
    chat.handle_turn_update(work("compaction:1", "duplicate", true));
    chat.handle_turn_update(work("permission-review:1", "approved", true));
    chat.handle_turn_update(work("permission-review:2", "denied", true));
    assert!(!chat.has_live_entries());
    let mut inserted = 0;
    let mut completed = 0;
    for event in events.try_iter() {
        match event {
            AppEvent::InsertLiveEntry(_) => inserted += 1,
            AppEvent::LiveEntryCompleted(_) => completed += 1,
            _ => {}
        }
    }
    assert_eq!((inserted, completed), (3, 3));
    assert_eq!(
        chat.work_entries["compaction:1"]
            .cell()
            .copy_source()
            .as_deref(),
        Some("partial summary")
    );
}

#[test]
fn reopening_failed_work_keeps_partial_text_and_completed_history_is_not_duplicated() {
    for committed in [false, true] {
        let (mut chat, events) = chat();
        let TurnUpdate::Work(mut snapshot) = work("compaction:1", "partial summary", true) else {
            unreachable!()
        };
        snapshot.committed = committed;
        snapshot.title = if committed {
            "Context compacted"
        } else {
            "Context compaction cancelled"
        }
        .to_string();
        chat.handle_turn_update(TurnUpdate::SessionOpened(OpenedSession {
            work: Some(snapshot.clone()),
            session_id: Some(AgentSessionId::new(7)),
            model: "fixture-model".to_string(),
            mode: hanafubuki_agent::AgentSessionMode::Default,
            transcript: if committed {
                vec![ReplayedItem::Compacted {
                    item_id: 4,
                    summary: "partial summary".to_string(),
                }]
            } else {
                vec![]
            },
            older: None,
            setup_needed: false,
        }));
        chat.handle_turn_update(TurnUpdate::Work(snapshot));
        let (cells, _) = drain(&events);
        let text = cells.join("\n");
        assert_eq!(text.matches("Context compact").count(), 1, "{text}");
        assert!(!chat.has_live_entries());
        assert_eq!(
            chat.work_entries["compaction:1"]
                .cell()
                .copy_source()
                .as_deref(),
            Some("partial summary")
        );
    }
}

#[test]
fn saved_review_replays_its_original_duration_and_rationale() {
    let (chat, _) = chat();
    let snapshot = WorkSnapshot::review(
        "saved".to_string(),
        "run command".to_string(),
        hanafubuki_agent::PermissionReviewStatus::Denied,
        Some("Outside the requested scope".to_string()),
        1_000,
        Some(3_500),
    );
    let cells = chat.replayed_cells(vec![ReplayedItem::PermissionReview(snapshot)]);
    let lines = cells[0]
        .transcript_lines(120)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        lines.contains("Automatic approval review denied: run command · 2.5s"),
        "{lines}"
    );
    assert!(lines.contains("Outside the requested scope"), "{lines}");
    assert!(cells[0].transcript_animation_tick().is_none());
}
