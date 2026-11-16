use super::*;

fn snapshot(text: Option<&str>, completed_at: Option<u64>) -> WorkSnapshot {
    WorkSnapshot {
        id: "compaction:test".to_string(),
        title: "Compacting context".to_string(),
        subject: None,
        review_status: None,
        text: text.map(str::to_string),
        started_at: 1_000,
        completed_at,
        warning: None,
        committed: false,
    }
}

#[test]
fn compaction_blinks_white_until_it_finishes() {
    use ratatui::style::Color;

    let mut cell = WorkCell::new(snapshot(Some("partial summary"), None));
    for (millis, text) in [(0, "•"), (599, "•"), (600, " "), (1_200, "•")] {
        let bullet = cell.bullet(Duration::from_millis(millis));
        assert_eq!(bullet.content, text);
        assert_eq!(bullet.style.fg, Some(Color::White));
    }
    assert!(cell.transcript_animation_tick().is_some());
    cell.update(snapshot(None, Some(1_700)));
    let replay = WorkCell::replayed(42, "summary".to_string());
    for finished in [&cell, &replay] {
        for millis in [0, 600, 1_200] {
            assert_eq!(finished.bullet(Duration::from_millis(millis)).content, "•");
        }
        assert!(finished.transcript_animation_tick().is_none());
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

#[test]
fn approved_review_collapses_to_one_line_and_reveals_the_action_and_rationale() {
    use hanafubuki_agent::PermissionReviewStatus;

    let action = "Run cargo test --workspace --all-features";
    let mut cell = WorkCell::new(WorkSnapshot::review(
        "test".to_string(),
        action.to_string(),
        PermissionReviewStatus::Reviewing,
        None,
        1_000,
        None,
    ));
    let lines = cell.display_lines(100);
    assert!(lines[0]
        .to_string()
        .chars()
        .skip(2)
        .collect::<String>()
        .starts_with("Reviewing approval request · "));
    assert!(!lines[0].to_string().contains(action));
    assert_eq!(lines[1].to_string(), format!("  └ {action}"));
    assert!(cell.activity_disclosure(100).is_none());

    let narrow = cell.display_lines(28);
    assert!(narrow.len() > 2);
    assert!(narrow[1..].iter().all(|line| line.width() <= 28));
    assert_eq!(
        narrow[1..]
            .iter()
            .map(|line| line.to_string().chars().skip(4).collect::<String>())
            .collect::<Vec<_>>()
            .join("")
            .split_whitespace()
            .collect::<String>(),
        action.split_whitespace().collect::<String>()
    );

    cell.update(WorkSnapshot::review(
        "test".to_string(),
        action.to_string(),
        PermissionReviewStatus::Approved,
        Some("Matches the requested validation.".to_string()),
        1_000,
        Some(3_500),
    ));
    assert_eq!(cell.display_lines(100).len(), 1);
    for lines in [cell.transcript_lines(100), cell.raw_lines()] {
        assert_eq!(
            lines[0].to_string(),
            "• Automatic approval review approved · 2.5s"
        );
        assert_eq!(lines[1].to_string(), format!("  └ {action}"));
    }
    assert!(cell.activity_disclosure(100).is_some());
    assert!(cell.transcript_lines(100).iter().any(|line| line
        .to_string()
        .contains("Matches the requested validation.")));
}

#[test]
fn review_blinks_white_and_finishes_with_a_steady_outcome_color() {
    use hanafubuki_agent::PermissionReviewStatus;
    use ratatui::style::Color;

    let reviewing = WorkSnapshot::review(
        "test".to_string(),
        "Run cargo test".to_string(),
        PermissionReviewStatus::Reviewing,
        None,
        1_000,
        None,
    );
    let cell = WorkCell::new(reviewing.clone());
    for (millis, text) in [(0, "•"), (599, "•"), (600, " "), (1_200, "•")] {
        let bullet = cell.bullet(Duration::from_millis(millis));
        assert_eq!(bullet.content, text);
        assert_eq!(bullet.style.fg, Some(Color::White));
    }
    assert!(cell.transcript_animation_tick().is_some());

    for (status, color) in [
        (PermissionReviewStatus::Approved, Color::Green),
        (PermissionReviewStatus::Denied, Color::Red),
        (PermissionReviewStatus::TimedOut, Color::Red),
        (PermissionReviewStatus::Failed, Color::Red),
        (PermissionReviewStatus::Aborted, Color::Red),
    ] {
        let mut cell = WorkCell::new(reviewing.clone());
        let completed = WorkSnapshot::review(
            "test".to_string(),
            "Run cargo test".to_string(),
            status,
            None,
            1_000,
            Some(3_500),
        );
        let replayed = WorkCell::new(completed.clone());
        cell.update(completed);
        for view in [&cell, &replayed] {
            for millis in [0, 600, 1_200] {
                let bullet = view.bullet(Duration::from_millis(millis));
                assert_eq!(bullet.content, "•");
                assert_eq!(bullet.style.fg, Some(color));
            }
            assert!(view.transcript_animation_tick().is_none());
        }
    }
}
