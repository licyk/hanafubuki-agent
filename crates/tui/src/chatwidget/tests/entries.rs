//! Thinking and tool calls: entries of the transcript from their start,
//! in both presentations, that change in place; Scrollback's tail is told
//! when each completes, so it may write it.

use std::sync::Arc;

use hanafubuki_agent::{AgentReasoningSnapshot, AgentToolCallDisplay, AgentToolDisplay};

use super::*;
use crate::app_event::AppCommand;

/// A cell's lines, collapsed or expanded, as text.
fn cell_lines(cell: &dyn HistoryCell, expanded: bool) -> String {
    let lines = if expanded {
        cell.expanded_lines(80)
    } else {
        cell.display_lines(80)
    };
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// What the transcript holds: every cell the chat handed it, a live entry
/// as it is now; `true` if the chat said one completed, as it does to
/// Scrollback's tail.
fn transcript(events: &Receiver<AppEvent>, entries: &mut Vec<Arc<dyn HistoryCell>>) -> bool {
    let mut written = false;
    while let Ok(event) = events.try_recv() {
        match event {
            AppEvent::InsertHistoryCell(cell) => entries.push(cell.into()),
            AppEvent::InsertLiveEntry(cell) => entries.push(cell),
            AppEvent::LiveEntryCompleted(_) => written = true,
            _ => {}
        }
    }
    written
}

/// The entries' text, a live thinking's clock left out (it counts on).
fn texts(entries: &[Arc<dyn HistoryCell>]) -> Vec<String> {
    entries
        .iter()
        .map(|cell| {
            let text = cell_text(cell.as_ref());
            match text.strip_prefix("• Thinking ") {
                Some(_) => "• Thinking …".to_string(),
                None => text,
            }
        })
        .collect()
}

fn tool_started(chat: &mut ChatWidget, call_id: &str, name: &str, subject: Option<&str>) {
    chat.handle_turn_update(TurnUpdate::ToolStarted {
        call_id: call_id.to_string(),
        name: name.to_string(),
        subject: subject.map(str::to_string),
    });
}

/// The call finishes, and what it shows arrives after.
fn tool_completed(chat: &mut ChatWidget, call_id: &str, output: &str) {
    chat.handle_turn_update(TurnUpdate::ToolCompleted {
        call_id: call_id.to_string(),
        is_error: false,
    });
    tool_displayed(chat, call_id, None, output);
}

fn tool_displayed(chat: &mut ChatWidget, call_id: &str, subject: Option<&str>, output: &str) {
    chat.handle_turn_update(TurnUpdate::ToolDisplayed {
        call_id: call_id.to_string(),
        display: Some(AgentToolCallDisplay {
            subject: subject.map(str::to_string),
            display: Some(AgentToolDisplay::Text {
                text: output.to_string(),
            }),
        }),
    });
}

#[test]
fn a_tool_call_completes_where_it_started_in_both_presentations() {
    for owned in [true, false] {
        let (mut chat, events) = chat_in(owned);
        let mut entries = Vec::new();
        chat.handle_turn_update(TurnUpdate::Started);
        transcript(&events, &mut entries);
        entries.clear();
        tool_started(&mut chat, "call-1", "exec_command", Some("ls"));
        assert!(!transcript(&events, &mut entries));
        assert_eq!(texts(&entries), vec!["• Running exec_command: ls"]);
        // The transcript (or the tail) shows it; the chat does not draw it.
        assert!(!screen(&chat, 60).contains("exec_command"));
        let revision = entries[0].revision();
        // Finished, it keeps its shape until what it shows arrives, and
        // Scrollback's tail may not write it yet.
        chat.handle_turn_update(TurnUpdate::ToolCompleted {
            call_id: "call-1".to_string(),
            is_error: false,
        });
        assert!(!transcript(&events, &mut entries));
        assert_eq!(texts(&entries), vec!["• Called exec_command: ls"]);
        assert!(!chat.has_live_entries());
        tool_displayed(&mut chat, "call-1", None, "Cargo.toml");
        assert_eq!(transcript(&events, &mut entries), !owned);
        assert_eq!(
            texts(&entries),
            vec!["• Called exec_command: ls\n  └ Cargo.toml"]
        );
        assert!(entries[0].revision() > revision);
    }
}

#[test]
fn what_a_call_shows_arrives_after_its_turn_and_names_its_subject() {
    for owned in [true, false] {
        let (mut chat, events) = chat_in(owned);
        let mut entries = Vec::new();
        chat.handle_turn_update(TurnUpdate::Started);
        tool_started(&mut chat, "call-1", "read_file", None);
        tool_started(&mut chat, "call-2", "read_file", None);
        for call_id in ["call-1", "call-2"] {
            chat.handle_turn_update(TurnUpdate::ToolCompleted {
                call_id: call_id.to_string(),
                is_error: false,
            });
        }
        chat.handle_turn_update(TurnUpdate::Ended(TurnEnd::Completed));
        transcript(&events, &mut entries);
        entries.retain(|entry| cell_text(entry.as_ref()).contains("read_file"));

        tool_displayed(&mut chat, "call-2", Some("b.rs"), "fn b() {}");
        assert_eq!(transcript(&events, &mut entries), !owned);
        // A read that failed completes the call as it stands.
        chat.handle_turn_update(TurnUpdate::ToolDisplayed {
            call_id: "call-1".to_string(),
            display: None,
        });
        assert_eq!(transcript(&events, &mut entries), !owned);
        assert_eq!(
            texts(&entries),
            vec![
                "• Called read_file",
                "• Called read_file: b.rs\n  └ fn b() {}",
            ]
        );
    }
}

#[test]
fn an_interrupted_turn_says_so_and_completes_its_running_calls() {
    let (mut chat, events) = chat();
    drain(&events);
    chat.handle_turn_update(TurnUpdate::Started);
    tool_started(&mut chat, "call-1", "exec_command", Some("sleep 100"));
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
fn thinking_changes_in_place_and_ctrl_o_is_left_to_the_transcript() {
    for owned in [true, false] {
        let (mut chat, events) = chat_in(owned);
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
        assert_eq!(texts(&entries), vec!["• Thinking …"]);
        assert_eq!(entries[0].activity_ids(), vec!["thinking:1".to_string()]);
        // Its control is in the conversation; the footer has no hint.
        assert!(!screen(&chat, 60).contains("ctrl + o"));

        let ctrl_o = KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL);
        chat.handle_key_event(ctrl_o);
        assert!(other_events(&events)
            .iter()
            .any(|event| matches!(event, AppEvent::Command(AppCommand::ToggleThinking))));
        chat.handle_turn_update(TurnUpdate::ReasoningCompleted {
            reasoning_id: first,
            duration_millis: 1_500,
        });
        assert_eq!(transcript(&events, &mut entries), !owned);
        assert_eq!(texts(&entries), vec!["• Thought · 1.5s"]);
        assert_eq!(
            cell_lines(entries[0].as_ref(), true),
            "• Thought · 1.5s\n  Weighing the options."
        );
        assert_eq!(entries[0].activity_ids(), vec!["thinking:1".to_string()]);
    }
}

#[test]
fn the_status_line_adds_the_turns_thinking_time_and_the_separator_keeps_it() {
    let (mut chat, events) = chat();
    chat.handle_turn_update(TurnUpdate::Started);
    drain(&events);
    let status = screen(&chat, 80);
    assert!(status.contains("Working 0s • esc to interrupt"), "{status}");
    assert!(!status.contains("Thinking"), "{status}");
    // A step's reset keeps what the turn thought.
    chat.handle_turn_update(TurnUpdate::Reasoning(AgentReasoningSnapshot {
        sampling_step: 2,
        turn_completed_millis: 20_790,
        ..AgentReasoningSnapshot::default()
    }));
    let status = screen(&chat, 80);
    assert!(
        status.contains("Working 0s (Thinking 20.7s) • esc to interrupt"),
        "{status}"
    );
    tool_started(&mut chat, "call-1", "exec_command", Some("ls"));
    tool_completed(&mut chat, "call-1", "Cargo.toml");
    chat.handle_turn_update(TurnUpdate::MessageDelta("Done.".to_string()));
    chat.handle_turn_update(TurnUpdate::MessageCompleted);
    let (cells, _) = drain(&events);
    assert!(
        cells
            .iter()
            .any(|cell| cell.starts_with("─ Worked for 0s · thought for 20.7s ──")),
        "{cells:?}"
    );
}

#[test]
fn thinking_completes_before_the_answer_of_its_step() {
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
        vec!["• Thought · 2.0s", "• Thought · 3.0s", "• The answer."]
    );
}

#[test]
fn a_turn_that_ends_while_thinking_completes_the_thinking_so_far() {
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
    assert_eq!(cells.len(), 2, "{cells:?}");
    // With the time so far.
    assert!(cells[0].starts_with("• Thought · 0."), "{cells:?}");
    assert_eq!(cells[1], "■ the provider went away");
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
                item_id: 4,
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
    assert_eq!(cells[2], "• Thought · 12.0s", "{cells:?}");
    assert_eq!(cells[3], "• Here is why.", "{cells:?}");
}

#[test]
fn an_entry_that_starts_while_a_message_streams_goes_in_its_place_at_once() {
    for owned in [true, false] {
        let (mut chat, events) = chat_in(owned);
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
            vec!["• Looking.", "• Thinking …", "• Running read_file"]
        );
        chat.handle_turn_update(TurnUpdate::ReasoningCompleted {
            reasoning_id: reasoning_id(1),
            duration_millis: 2_000,
        });
        tool_completed(&mut chat, "call-1", "fn main() {}");
        chat.handle_turn_update(TurnUpdate::MessageDelta("Found it.".to_string()));
        chat.handle_turn_update(TurnUpdate::MessageCompleted);
        chat.handle_turn_update(TurnUpdate::Ended(TurnEnd::Completed));
        assert_eq!(transcript(&events, &mut entries), !owned);
        let texts = texts(&entries);
        assert_eq!(
            texts[..3],
            [
                "• Looking.",
                "• Thought · 2.0s",
                "• Called read_file\n  └ fn main() {}"
            ]
        );
        assert_eq!(texts.last().map(String::as_str), Some("• Found it."));
    }
}

#[test]
fn a_turn_that_ends_leaves_its_entries_where_they_are() {
    for owned in [true, false] {
        let (mut chat, events) = chat_in(owned);
        let mut entries = Vec::new();
        chat.handle_turn_update(TurnUpdate::Started);
        chat.handle_turn_update(TurnUpdate::ReasoningStarted(reasoning_id(1)));
        tool_started(&mut chat, "call-1", "exec_command", Some("sleep 100"));
        transcript(&events, &mut entries);
        let count = entries.len();
        chat.handle_turn_update(TurnUpdate::Ended(TurnEnd::Interrupted));
        assert_eq!(transcript(&events, &mut entries), !owned);
        let texts = texts(&entries);
        assert!(texts[count - 2].starts_with("• Thought · 0."), "{texts:?}");
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
}
