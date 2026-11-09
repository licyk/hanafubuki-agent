use super::*;

use ConversationMessage::{Assistant, User};

/// Collects `oldest_first` the way the reader hands it over, newest first,
/// until the collector has enough.
fn exchanges(oldest_first: Vec<ConversationMessage>) -> Vec<Exchange> {
    let mut recent = RecentExchanges::default();
    for message in oldest_first.into_iter().rev() {
        if recent.push(message).is_break() {
            break;
        }
    }
    recent.finish()
}

fn user(text: &str) -> ConversationMessage {
    User(text.to_string())
}

fn assistant(text: &str) -> ConversationMessage {
    Assistant(text.to_string())
}

#[test]
fn exchanges_join_steering_and_keep_a_pending_request() {
    let history = recap_history(&exchanges(vec![
        assistant("an answer before any request is dropped"),
        user("Fix the login"),
        assistant("Fixed it."),
        user("Now add a test"),
        user("  "),
        user("and run it"),
        assistant("Added and ran it."),
        user("Deploy it"),
    ]));
    assert_eq!(
        history,
        "User: Fix the login\n\nAssistant: Fixed it.\n\n\
         User: Now add a test\n\nand run it\n\nAssistant: Added and ran it.\n\n\
         Pending user request: Deploy it"
    );
}

#[test]
fn only_the_latest_answered_exchanges_are_read() {
    let mut messages = Vec::new();
    for index in 0..RECAP_HISTORY_MAX_TURNS + 3 {
        messages.push(user(&format!("request {index}")));
        messages.push(assistant(&format!("answer {index}")));
    }
    messages.push(user("pending"));
    let exchanges = exchanges(messages);
    assert_eq!(exchanges.len(), RECAP_HISTORY_MAX_TURNS + 1);
    assert_eq!(exchanges[0].user, "request 3");
    assert_eq!(exchanges[RECAP_HISTORY_MAX_TURNS].user, "pending");
    assert!(exchanges[RECAP_HISTORY_MAX_TURNS].assistant.is_empty());
}

#[test]
fn a_long_conversation_is_cut_to_the_limit_keeping_the_newest_answer() {
    let max_bytes = history_max_bytes();
    let mut messages = Vec::new();
    for index in 0..RECAP_HISTORY_MAX_TURNS {
        messages.push(user(&format!("request {index}")));
        messages.push(assistant(&"界".repeat(max_bytes / 8)));
    }
    let history = recap_history(&exchanges(messages));
    assert!(history.len() <= max_bytes, "{}", history.len());
    assert!(history.starts_with(OMITTED_HISTORY));
    assert!(history.contains(&format!("request {}", RECAP_HISTORY_MAX_TURNS - 1)));

    let huge = format!("head {} tail", "界".repeat(max_bytes));
    let history = recap_history(&exchanges(vec![user("Explain"), assistant(&huge)]));
    assert!(history.len() <= max_bytes, "{}", history.len());
    assert!(history.contains("Assistant: head "));
    assert!(history.ends_with(" tail"));
    assert!(history.contains(EXCERPT_MARKER));
}

#[test]
fn no_request_is_no_history() {
    assert_eq!(recap_history(&exchanges(vec![assistant("Hello")])), "");
    assert_eq!(recap_history(&exchanges(Vec::new())), "");
}
