//! Thinking and tool calls: in Scrollback, live in the viewport and
//! written when they complete, held behind a streaming message; in
//! Fullscreen, entries of the transcript from their start that change in
//! place.

use std::sync::Arc;

use super::*;

/// What the transcript holds: every cell the chat handed it, a live entry
/// as it is now; `true` if any was written as Scrollback writes.
fn transcript(events: &Receiver<AppEvent>, entries: &mut Vec<Arc<dyn HistoryCell>>) -> bool {
    let mut written = false;
    while let Ok(event) = events.try_recv() {
        match event {
            AppEvent::InsertHistoryCell(cell) => entries.push(cell.into()),
            AppEvent::InsertLiveEntry(cell) => entries.push(cell),
            AppEvent::WriteLiveEntry(_) => written = true,
            _ => {}
        }
    }
    written
}

fn texts(entries: &[Arc<dyn HistoryCell>]) -> Vec<String> {
    entries
        .iter()
        .map(|cell| cell_text(cell.as_ref()))
        .collect()
}

fn tool_started(chat: &mut ChatWidget, call_id: &str, name: &str, subject: Option<&str>) {
    chat.handle_turn_update(TurnUpdate::ToolStarted {
        call_id: call_id.to_string(),
        name: name.to_string(),
        subject: subject.map(str::to_string),
    });
}

fn tool_completed(chat: &mut ChatWidget, call_id: &str, output: &str) {
    chat.handle_turn_update(TurnUpdate::ToolCompleted {
        call_id: call_id.to_string(),
        is_error: false,
        result: ToolResultView {
            text: Some(output.to_string()),
            ..ToolResultView::default()
        },
    });
}

#[test]
fn a_tool_call_runs_in_the_viewport_and_is_written_when_it_completes() {
    let (mut chat, events) = chat();
    drain(&events);
    chat.handle_turn_update(TurnUpdate::Started);
    chat.handle_turn_update(TurnUpdate::ToolStarted {
        call_id: "call-1".to_string(),
        name: "exec_command".to_string(),
        subject: Some("ls".to_string()),
    });
    assert_eq!(chat.running_tools.len(), 1);
    assert!(drain(&events).0.is_empty());
    chat.handle_turn_update(TurnUpdate::ToolCompleted {
        call_id: "call-1".to_string(),
        is_error: false,
        result: ToolResultView {
            text: Some("Cargo.toml".to_string()),
            ..ToolResultView::default()
        },
    });
    assert!(chat.running_tools.is_empty());
    assert_eq!(
        drain(&events).0,
        vec!["• Called exec_command: ls\n  └ Cargo.toml"]
    );
}

#[test]
fn a_call_that_starts_while_a_message_streams_waits_for_the_message() {
    let (mut chat, events) = chat();
    drain(&events);
    chat.handle_turn_update(TurnUpdate::Started);
    chat.handle_turn_update(TurnUpdate::MessageDelta("Looking.".to_string()));
    chat.handle_turn_update(TurnUpdate::ToolStarted {
        call_id: "call-1".to_string(),
        name: "read_file".to_string(),
        subject: None,
    });
    assert!(chat.running_tools.is_empty());
    chat.handle_turn_update(TurnUpdate::MessageCompleted);
    assert_eq!(chat.running_tools.len(), 1);
    assert_eq!(drain(&events).0, vec!["• Looking."]);
}

#[test]
fn an_interrupted_turn_says_so_and_writes_its_running_calls() {
    let (mut chat, events) = chat();
    drain(&events);
    chat.handle_turn_update(TurnUpdate::Started);
    chat.handle_turn_update(TurnUpdate::ToolStarted {
        call_id: "call-1".to_string(),
        name: "exec_command".to_string(),
        subject: Some("sleep 100".to_string()),
    });
    chat.handle_turn_update(TurnUpdate::Ended(TurnEnd::Interrupted));
    let (cells, _) = drain(&events);
    assert_eq!(cells.len(), 2, "{cells:?}");
    assert!(
        cells[0].contains("Running exec_command: sleep 100"),
        "{cells:?}"
    );
    assert!(cells[1].contains("Conversation interrupted"), "{cells:?}");
}

#[test]
fn thinking_is_live_in_the_viewport_and_ctrl_o_expands_it_for_the_session() {
    let (mut chat, events) = chat();
    chat.handle_turn_update(TurnUpdate::Started);
    drain(&events);
    let first = reasoning_id(1);
    chat.handle_turn_update(TurnUpdate::ReasoningStarted(first));
    chat.handle_turn_update(TurnUpdate::ReasoningDelta {
        reasoning_id: first,
        delta: "Weighing the **options**.".to_string(),
    });
    let collapsed = screen(&chat, 60);
    assert!(collapsed.contains("• Thinking 0s"), "{collapsed}");
    assert!(!collapsed.contains("Weighing"), "{collapsed}");
    assert!(collapsed.contains("Working"), "{collapsed}");
    assert!(
        collapsed.contains("ctrl + o to show thinking"),
        "{collapsed}"
    );

    let ctrl_o = KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL);
    chat.handle_key_event(ctrl_o);
    let expanded = screen(&chat, 60);
    assert!(expanded.contains("Weighing the options."), "{expanded}");
    assert!(expanded.contains("ctrl + o to hide thinking"), "{expanded}");
    assert!(drain(&events).0.is_empty());

    chat.handle_turn_update(TurnUpdate::ReasoningCompleted {
        reasoning_id: first,
        duration_millis: 1_500,
    });
    assert_eq!(
        drain(&events).0,
        vec!["• Thought for 1s\n  Weighing the options."]
    );
    assert!(!screen(&chat, 60).contains("ctrl + o"));

    // The next cell opens expanded; toggling back collapses it.
    think(&mut chat, 2, "Second thought.", 0);
    assert_eq!(
        drain(&events).0,
        vec!["• Thought for 0s\n  Second thought."]
    );
    chat.handle_key_event(ctrl_o);
    think(&mut chat, 3, "Third thought.", 0);
    assert_eq!(drain(&events).0, vec!["• Thought for 0s"]);
}

#[test]
fn thinking_is_written_before_the_answer_of_its_step() {
    let (mut chat, events) = chat();
    chat.handle_turn_update(TurnUpdate::Started);
    drain(&events);
    think(&mut chat, 1, "First part.", 2_000);
    think(&mut chat, 2, "Second part.", 3_000);
    chat.handle_turn_update(TurnUpdate::MessageDelta("The answer.".to_string()));
    chat.handle_turn_update(TurnUpdate::MessageCompleted);
    chat.handle_turn_update(TurnUpdate::Ended(TurnEnd::Completed));
    assert_eq!(
        drain(&events).0,
        vec!["• Thought for 2s", "• Thought for 3s", "• The answer."]
    );
}

#[test]
fn thinking_after_a_streaming_message_waits_for_the_message() {
    let (mut chat, events) = chat();
    chat.handle_turn_update(TurnUpdate::Started);
    drain(&events);
    chat.handle_turn_update(TurnUpdate::MessageDelta("Looking.".to_string()));
    think(&mut chat, 1, "Then this.", 4_000);
    assert!(chat.live_thinking.is_empty());
    assert!(drain(&events).0.is_empty());
    chat.handle_turn_update(TurnUpdate::MessageCompleted);
    assert_eq!(drain(&events).0, vec!["• Looking.", "• Thought for 4s"]);
}

#[test]
fn a_turn_that_ends_while_thinking_writes_the_thinking_so_far() {
    let (mut chat, events) = chat();
    chat.handle_turn_update(TurnUpdate::Started);
    drain(&events);
    let reasoning_id = reasoning_id(1);
    chat.handle_turn_update(TurnUpdate::ReasoningStarted(reasoning_id));
    chat.handle_turn_update(TurnUpdate::ReasoningDelta {
        reasoning_id,
        delta: "Halfway".to_string(),
    });
    chat.handle_turn_update(TurnUpdate::Ended(TurnEnd::Failed(
        "the provider went away".to_string(),
    )));
    let (cells, _) = drain(&events);
    assert_eq!(
        cells,
        vec!["• Thought for 0s", "■ the provider went away"],
        "{cells:?}"
    );
    assert!(chat.live_thinking.is_empty());
    assert!(!screen(&chat, 60).contains("Thinking"));
}

#[test]
fn a_replayed_reasoning_item_is_a_completed_cell() {
    let (mut chat, events) = chat();
    drain(&events);
    chat.handle_turn_update(TurnUpdate::SessionOpened(OpenedSession {
        session_id: Some(hanafubuki_agent::AgentSessionId::new(7)),
        model: "fixture-model".to_string(),
        mode: hanafubuki_agent::AgentSessionMode::Default,
        transcript: vec![
            ReplayedItem::User("Why?".to_string()),
            ReplayedItem::Reasoning {
                text: "Because.".to_string(),
                duration_millis: 12_000,
                truncated: false,
            },
            ReplayedItem::Assistant("Here is why.".to_string()),
        ],
        older: None,
        setup_needed: false,
    }));
    let (cells, _) = drain(&events);
    assert_eq!(cells[2], "• Thought for 12s", "{cells:?}");
    assert_eq!(cells[3], "• Here is why.", "{cells:?}");
}

#[test]
fn in_fullscreen_a_tool_call_is_an_entry_from_its_start_and_completes_in_place() {
    let (mut chat, events) = chat_in(true);
    let mut entries = Vec::new();
    chat.handle_turn_update(TurnUpdate::Started);
    transcript(&events, &mut entries);
    entries.clear();
    tool_started(&mut chat, "call-1", "exec_command", Some("ls"));
    assert!(!transcript(&events, &mut entries));
    assert_eq!(texts(&entries), vec!["• Running exec_command: ls"]);
    // The transcript shows it; the chat does not draw it again.
    assert!(!screen(&chat, 60).contains("exec_command"));
    let revision = entries[0].revision();
    tool_completed(&mut chat, "call-1", "Cargo.toml");
    assert!(!transcript(&events, &mut entries));
    assert_eq!(
        texts(&entries),
        vec!["• Called exec_command: ls\n  └ Cargo.toml"]
    );
    assert!(entries[0].revision() > revision);
    assert!(!chat.has_live_entries());
}

#[test]
fn in_fullscreen_thinking_changes_in_place_and_ctrl_o_expands_it_there() {
    let (mut chat, events) = chat_in(true);
    let mut entries = Vec::new();
    chat.handle_turn_update(TurnUpdate::Started);
    transcript(&events, &mut entries);
    entries.clear();
    let first = reasoning_id(1);
    chat.handle_turn_update(TurnUpdate::ReasoningStarted(first));
    chat.handle_turn_update(TurnUpdate::ReasoningDelta {
        reasoning_id: first,
        delta: "Weighing the **options**.".to_string(),
    });
    transcript(&events, &mut entries);
    assert_eq!(texts(&entries), vec!["• Thinking 0s"]);
    chat.handle_key_event(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
    assert_eq!(
        texts(&entries),
        vec!["• Thinking 0s\n  Weighing the options."]
    );
    chat.handle_turn_update(TurnUpdate::ReasoningCompleted {
        reasoning_id: first,
        duration_millis: 1_500,
    });
    assert!(!transcript(&events, &mut entries));
    assert_eq!(
        texts(&entries),
        vec!["• Thought for 1s\n  Weighing the options."]
    );
}

#[test]
fn in_fullscreen_an_entry_that_starts_while_a_message_streams_goes_in_its_place_at_once() {
    let (mut chat, events) = chat_in(true);
    let mut entries = Vec::new();
    chat.handle_turn_update(TurnUpdate::Started);
    transcript(&events, &mut entries);
    entries.clear();
    chat.handle_turn_update(TurnUpdate::MessageDelta("Looking.".to_string()));
    chat.handle_turn_update(TurnUpdate::ReasoningStarted(reasoning_id(1)));
    tool_started(&mut chat, "call-1", "read_file", None);
    transcript(&events, &mut entries);
    assert_eq!(
        texts(&entries),
        vec!["• Looking.", "• Thinking 0s", "• Running read_file"]
    );
    chat.handle_turn_update(TurnUpdate::ReasoningCompleted {
        reasoning_id: reasoning_id(1),
        duration_millis: 2_000,
    });
    tool_completed(&mut chat, "call-1", "fn main() {}");
    chat.handle_turn_update(TurnUpdate::MessageDelta("Found it.".to_string()));
    chat.handle_turn_update(TurnUpdate::MessageCompleted);
    chat.handle_turn_update(TurnUpdate::Ended(TurnEnd::Completed));
    assert!(!transcript(&events, &mut entries));
    let texts = texts(&entries);
    assert_eq!(
        texts[..3],
        [
            "• Looking.",
            "• Thought for 2s",
            "• Called read_file\n  └ fn main() {}"
        ]
    );
    assert_eq!(texts.last().map(String::as_str), Some("• Found it."));
}

#[test]
fn in_fullscreen_a_turn_that_ends_leaves_its_entries_where_they_are() {
    let (mut chat, events) = chat_in(true);
    let mut entries = Vec::new();
    chat.handle_turn_update(TurnUpdate::Started);
    chat.handle_turn_update(TurnUpdate::ReasoningStarted(reasoning_id(1)));
    tool_started(&mut chat, "call-1", "exec_command", Some("sleep 100"));
    transcript(&events, &mut entries);
    let count = entries.len();
    chat.handle_turn_update(TurnUpdate::Ended(TurnEnd::Interrupted));
    assert!(!transcript(&events, &mut entries));
    let texts = texts(&entries);
    assert_eq!(texts[count - 2], "• Thought for 0s");
    assert!(
        texts[count - 1].contains("Running exec_command: sleep 100"),
        "{texts:?}"
    );
    assert!(
        texts[count].contains("Conversation interrupted"),
        "{texts:?}"
    );
    assert!(!chat.has_live_entries());
}
