use std::sync::mpsc::{channel, Receiver};

use super::*;
use crate::agent_bridge::{ModelChoices, OpenedSession, ReplayedItem, TurnEnd};
use crate::app_event::AgentCommand;
use crate::history_cell::ToolResultView;
use crate::slash_command::SlashCommand;

fn chat() -> (ChatWidget, Receiver<AppEvent>) {
    let (sender, events) = channel();
    let sender = AppEventSender::new(sender);
    let chat = ChatWidget::new(ChatWidgetInit {
        app_event_tx: sender.clone(),
        frame_requester: FrameRequester::new(sender),
        enhanced_keys_supported: false,
        directory: "/project".to_string(),
    });
    (chat, events)
}

/// The text of the cells the chat inserted into the transcript so far,
/// one string per cell, and the ops it asked of the Agent.
fn drain(events: &Receiver<AppEvent>) -> (Vec<String>, Vec<Op>) {
    let mut cells = Vec::new();
    let mut ops = Vec::new();
    while let Ok(event) = events.try_recv() {
        match event {
            AppEvent::InsertHistoryCell(cell) => cells.push(
                cell.display_lines(80)
                    .iter()
                    .map(|line| {
                        line.spans
                            .iter()
                            .map(|span| span.content.as_ref())
                            .collect::<String>()
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            AppEvent::Op(op) => ops.push(op),
            _ => {}
        }
    }
    (cells, ops)
}

/// The chat's viewport as drawn at `width`.
fn screen(chat: &ChatWidget, width: u16) -> String {
    let area = ratatui::layout::Rect::new(0, 0, width, chat.desired_height(width));
    let mut buf = ratatui::buffer::Buffer::empty(area);
    chat.render(area, &mut buf);
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn reasoning_id(id: u64) -> hanafubuki_agent::AgentReasoningId {
    hanafubuki_agent::AgentReasoningId::new(id)
}

fn think(chat: &mut ChatWidget, id: u64, text: &str, duration_millis: u64) {
    let reasoning_id = reasoning_id(id);
    chat.handle_turn_update(TurnUpdate::ReasoningStarted(reasoning_id));
    chat.handle_turn_update(TurnUpdate::ReasoningDelta {
        reasoning_id,
        delta: text.to_string(),
    });
    chat.handle_turn_update(TurnUpdate::ReasoningCompleted {
        reasoning_id,
        duration_millis,
    });
}

#[test]
fn a_streamed_message_is_written_when_it_completes() {
    let (mut chat, events) = chat();
    drain(&events);
    chat.handle_turn_update(TurnUpdate::Started);
    chat.handle_turn_update(TurnUpdate::MessageDelta("Hello, ".to_string()));
    chat.handle_turn_update(TurnUpdate::MessageDelta("world.".to_string()));
    assert!(drain(&events).0.is_empty());
    chat.handle_turn_update(TurnUpdate::MessageCompleted);
    chat.handle_turn_update(TurnUpdate::Ended(TurnEnd::Completed));
    assert_eq!(drain(&events).0, vec!["• Hello, world."]);
    assert!(!chat.bottom_pane.is_task_running());
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
fn a_submitted_message_is_written_and_sent_and_quit_exits() {
    let (mut chat, events) = chat();
    drain(&events);
    chat.submit_user_message("Do it".to_string());
    let (cells, ops) = drain(&events);
    assert!(cells[0].contains("› Do it"), "{cells:?}");
    assert_eq!(ops, vec![Op::UserInput("Do it".to_string())]);
    chat.dispatch_command(SlashCommand::Quit);
    assert!(matches!(events.try_recv(), Ok(AppEvent::Exit)));
}

#[test]
fn ctrl_c_interrupts_a_running_turn_and_twice_quits() {
    let (mut chat, events) = chat();
    chat.handle_turn_update(TurnUpdate::Started);
    drain(&events);
    let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
    chat.handle_key_event(ctrl_c);
    assert_eq!(drain(&events).1, vec![Op::Interrupt]);
    chat.handle_key_event(ctrl_c);
    let exited =
        std::iter::from_fn(|| events.try_recv().ok()).any(|event| matches!(event, AppEvent::Exit));
    assert!(exited);
}

#[test]
fn a_command_the_running_turn_forbids_says_so() {
    let (mut chat, events) = chat();
    chat.handle_turn_update(TurnUpdate::Started);
    drain(&events);
    chat.dispatch_command(SlashCommand::Model);
    let (cells, ops) = drain(&events);
    assert_eq!(
        cells,
        vec!["■ '/model' is disabled while a task is in progress."]
    );
    assert!(ops.is_empty());
}

#[test]
fn an_opened_session_replays_its_transcript_after_its_header() {
    let (mut chat, events) = chat();
    drain(&events);
    chat.handle_turn_update(TurnUpdate::SessionOpened(OpenedSession {
        session_id: hanafubuki_agent::AgentSessionId::new(7),
        model: "fixture-model".to_string(),
        mode: hanafubuki_agent::AgentSessionMode::Plan,
        transcript: vec![
            ReplayedItem::User("What changed?".to_string()),
            ReplayedItem::Tool {
                call_id: "call-1".to_string(),
                name: "exec_command".to_string(),
                is_error: false,
                result: ToolResultView {
                    text: Some("two files".to_string()),
                    ..ToolResultView::default()
                },
            },
            ReplayedItem::Assistant("Two files **changed**.".to_string()),
        ],
        earlier_items: true,
    }));
    let (cells, _) = drain(&events);
    assert_eq!(cells.len(), 5, "{cells:?}");
    assert!(cells[0].contains("model:     fixture-model"), "{cells:?}");
    assert!(
        cells[1].contains("Earlier messages are not shown."),
        "{cells:?}"
    );
    assert!(cells[2].contains("› What changed?"), "{cells:?}");
    assert!(cells[3].contains("exec_command") && cells[3].contains("two files"));
    assert_eq!(cells[4], "• Two files changed.");
    assert_eq!(chat.mode, hanafubuki_agent::AgentSessionMode::Plan);
}

#[test]
fn a_model_picked_from_the_catalogs_is_set_on_the_session() {
    let (mut chat, events) = chat();
    drain(&events);
    let entry = |id: &str| hanafubuki_agent::ModelCatalogEntry {
        id: id.to_string(),
        display_name: None,
        default_reasoning_effort: None,
        supported_reasoning_efforts: Vec::new(),
        context_window: None,
    };
    chat.handle_turn_update(TurnUpdate::ModelChoices(ModelChoices {
        provider_id: "provider-1".to_string(),
        model: "model-a".to_string(),
        catalogs: vec![
            hanafubuki_agent::AgentProviderModelCatalogResult {
                provider_id: "provider-1".to_string(),
                display_name: "First".to_string(),
                models: vec![entry("model-a"), entry("model-b")],
                error: None,
            },
            hanafubuki_agent::AgentProviderModelCatalogResult {
                provider_id: "provider-2".to_string(),
                display_name: "Second".to_string(),
                models: Vec::new(),
                error: Some("unreachable".to_string()),
            },
        ],
    }));
    let area = ratatui::layout::Rect::new(0, 0, 60, chat.desired_height(60));
    let mut buf = ratatui::buffer::Buffer::empty(area);
    chat.render(area, &mut buf);
    let screen = (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(screen.contains("Select Model"), "{screen}");
    assert!(
        screen.contains("models not listed: unreachable"),
        "{screen}"
    );

    chat.handle_key_event(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    chat.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(
        drain(&events).1,
        vec![Op::Command(AgentCommand::SetModel {
            provider_id: "provider-1".to_string(),
            model: "model-b".to_string(),
        })]
    );
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
        session_id: hanafubuki_agent::AgentSessionId::new(7),
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
        earlier_items: false,
    }));
    let (cells, _) = drain(&events);
    assert_eq!(cells[2], "• Thought for 12s", "{cells:?}");
    assert_eq!(cells[3], "• Here is why.", "{cells:?}");
}

#[test]
fn ctrl_o_between_two_ctrl_c_does_not_quit() {
    let (mut chat, events) = chat();
    chat.handle_turn_update(TurnUpdate::Started);
    drain(&events);
    let ctrl = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
    chat.handle_key_event(ctrl('c'));
    chat.handle_key_event(ctrl('o'));
    chat.handle_key_event(ctrl('c'));
    let exited =
        std::iter::from_fn(|| events.try_recv().ok()).any(|event| matches!(event, AppEvent::Exit));
    assert!(!exited);
    assert!(chat.thinking_expanded);
}
