// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

use super::*;

#[test]
fn duplicate_submissions_are_not_recorded() {
    let mut history = ChatComposerHistory::new();

    // Empty submissions are ignored.
    history.record_local_submission(String::new());
    assert_eq!(history.local_history.len(), 0);

    // First entry is recorded.
    history.record_local_submission("hello".to_string());
    assert_eq!(history.local_history.len(), 1);
    assert_eq!(history.local_history.last().unwrap(), "hello");

    // Identical consecutive entry is skipped.
    history.record_local_submission("hello".to_string());
    assert_eq!(history.local_history.len(), 1);

    // Different entry is recorded.
    history.record_local_submission("world".to_string());
    assert_eq!(history.local_history.len(), 2);
    assert_eq!(history.local_history.last().unwrap(), "world");
}

#[test]
fn navigation_walks_back_and_forward_through_the_session() {
    let mut history = ChatComposerHistory::new();
    history.record_local_submission("older".to_string());
    history.record_local_submission("latest".to_string());

    assert!(history.should_handle_navigation("", 0));
    assert_eq!(history.navigate_up().as_deref(), Some("latest"));
    assert_eq!(history.navigate_up().as_deref(), Some("older"));
    // Already at the oldest entry.
    assert_eq!(history.navigate_up(), None);
    assert_eq!(history.navigate_down().as_deref(), Some("latest"));
    // Past the newest entry the composer is cleared.
    assert_eq!(history.navigate_down().as_deref(), Some(""));
    assert_eq!(history.navigate_down(), None);
}

#[test]
fn reset_navigation_resets_cursor() {
    let mut history = ChatComposerHistory::new();
    history.record_local_submission("command2".to_string());
    history.record_local_submission("command3".to_string());

    assert_eq!(history.navigate_up().as_deref(), Some("command3"));
    assert_eq!(history.navigate_up().as_deref(), Some("command2"));

    history.reset_navigation();
    assert!(history.history_cursor.is_none());
    assert!(history.last_history_text.is_none());

    assert_eq!(history.navigate_up().as_deref(), Some("command3"));
}

#[test]
fn editing_a_recalled_entry_stops_navigation() {
    let mut history = ChatComposerHistory::new();
    history.record_local_submission("recalled".to_string());
    assert_eq!(history.navigate_up().as_deref(), Some("recalled"));
    assert!(history.should_handle_navigation("recalled", 0));
    assert!(!history.should_handle_navigation("recalled!", 0));
    assert!(!history.should_handle_navigation("recalled", 3));
}
