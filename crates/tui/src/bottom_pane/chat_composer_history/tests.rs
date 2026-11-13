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

use HistorySearchDirection::{Newer, Older};
use HistorySearchResult::{AtBoundary, Found, NotFound, Pending};

fn found(text: &str) -> HistorySearchResult {
    Found(text.to_string())
}

/// A history of this run's `local` (oldest first) whose stored messages
/// (newest first) are read.
fn searchable(local: &[&str], stored: &[&str]) -> ChatComposerHistory {
    let mut history = ChatComposerHistory::new();
    for entry in local {
        history.record_local_submission(entry.to_string());
    }
    assert!(history.begin_search());
    assert_eq!(
        history.on_stored_read(stored.iter().map(|entry| entry.to_string()).collect()),
        None
    );
    history
}

#[test]
fn the_search_walks_unique_matches_newest_first_and_stays_at_the_ends() {
    let mut history = searchable(
        &["git status", "cargo test", "git log", "git status"],
        &["git log", "GIT push", "ls"],
    );
    assert_eq!(history.search("git", Older, true), found("git status"));
    assert_eq!(history.search("git", Older, false), found("git log"));
    // The stored `git log` is a duplicate; a stored match comes after this
    // run's, and case does not matter.
    assert_eq!(history.search("git", Older, false), found("GIT push"));
    assert_eq!(history.search("git", Older, false), AtBoundary);
    assert_eq!(history.search("git", Newer, false), found("git log"));
    assert_eq!(history.search("git", Newer, false), found("git status"));
    assert_eq!(history.search("git", Newer, false), AtBoundary);
    // A new query starts again from the newest.
    assert_eq!(history.search("cargo", Older, false), found("cargo test"));
    assert_eq!(history.search("missing", Older, false), NotFound);
    assert_eq!(history.search("missing", Newer, false), NotFound);
}

#[test]
fn a_search_past_this_runs_matches_waits_for_the_stored_ones() {
    let mut history = ChatComposerHistory::new();
    history.record_local_submission("echo local".to_string());
    assert!(history.begin_search());
    // The stored messages are read once.
    assert!(!history.begin_search());

    assert_eq!(history.search("echo", Older, true), found("echo local"));
    assert_eq!(history.search("echo", Older, false), Pending);
    assert_eq!(
        history.on_stored_read(vec!["!echo stored".to_string(), "echo local".to_string()]),
        Some(found("!echo stored"))
    );
    assert_eq!(history.search("echo", Older, false), AtBoundary);
    assert_eq!(history.search("echo", Newer, false), found("echo local"));
}

#[test]
fn a_search_with_no_match_yet_waits_and_ends_with_none() {
    let mut history = ChatComposerHistory::new();
    assert!(history.begin_search());
    assert_eq!(history.search("nothing", Older, true), Pending);
    assert_eq!(history.on_stored_read(Vec::new()), Some(NotFound));

    let mut history = ChatComposerHistory::new();
    assert!(history.begin_search());
    assert_eq!(history.search("earlier", Older, true), Pending);
    // A newer step before any match waits as well.
    assert_eq!(history.search("earlier", Newer, false), Pending);
    assert_eq!(
        history.on_stored_read(vec!["an earlier message".to_string()]),
        Some(found("an earlier message"))
    );
}

#[test]
fn a_sent_message_ends_the_search() {
    let mut history = searchable(&["first"], &[]);
    assert_eq!(history.search("first", Older, true), found("first"));
    history.record_local_submission("first again".to_string());
    assert_eq!(history.search("first", Older, false), found("first again"));
}
