use super::*;

fn snapshot(text: Option<&str>, completed_at: Option<u64>) -> WorkSnapshot {
    WorkSnapshot {
        id: "compaction:test".to_string(),
        title: "Compacting context".to_string(),
        text: text.map(str::to_string),
        started_at: 1_000,
        completed_at,
        warning: None,
        committed: false,
    }
}

#[test]
fn full_snapshots_replace_text_and_terminal_freezes_elapsed_and_partial_text() {
    let mut cell = WorkCell::new(snapshot(Some("first"), None));
    assert_eq!(cell.display_lines(80).len(), 1);
    assert!(cell.activity_disclosure(80).is_some());
    assert!(cell.transcript_animation_tick().is_some());
    cell.update(snapshot(Some("first second"), None));
    assert_eq!(cell.copy_source().as_deref(), Some("first second"));
    let id = cell.activity_ids();
    cell.update(snapshot(None, Some(3_500)));
    assert_eq!(cell.copy_source().as_deref(), Some("first second"));
    assert_eq!(cell.elapsed(), Duration::from_millis(2_500));
    assert_eq!(cell.activity_ids(), id);
    assert!(cell.transcript_animation_tick().is_none());
    cell.update(snapshot(Some("duplicate"), Some(9_000)));
    assert_eq!(cell.copy_source().as_deref(), Some("first second"));
    assert_eq!(cell.elapsed(), Duration::from_millis(2_500));
}

#[test]
fn replay_has_details_without_inventing_a_duration() {
    let cell = WorkCell::replayed(42, "saved summary".to_string());
    assert_eq!(cell.display_lines(80)[0].to_string(), "• Context compacted");
    assert_eq!(cell.copy_source().as_deref(), Some("saved summary"));
    assert!(cell
        .transcript_lines(80)
        .iter()
        .any(|line| line.to_string().contains("saved summary")));
    assert!(cell.finished());
    assert!(cell.transcript_animation_tick().is_none());
}
