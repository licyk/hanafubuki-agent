use crate::appearance::Appearance;
use hanafubuki_agent::AgentTuiPreferences;
use std::sync::mpsc::{channel, Receiver};

use super::*;
use crate::agent_bridge::{
    HooksCall, HooksReply, McpCall, ModelChoices, OlderTranscript, OpenedSession, ReplayedItem,
    SteerOutcome, TurnEnd, TurnUpdate,
};
use crate::app::WorkspaceEvent;
use crate::app_event::AgentCommand;
use crate::bottom_pane::{SelectionItem, SelectionViewParams};
use crate::commands::{self, CommandSpec};
use crate::history_cell::ToolResultView;
use hanafubuki_agent::panic::is_panic_contained;
use hanafubuki_agent::{AgentRevision, AgentSessionId};

mod entries;

/// The chat in Scrollback.
fn chat() -> (ChatWidget, Receiver<AppEvent>) {
    chat_in(false)
}

/// The chat in Fullscreen (`owned_transcript`) or Scrollback.
fn chat_in(owned_transcript: bool) -> (ChatWidget, Receiver<AppEvent>) {
    let (sender, events) = channel();
    let sender = AppEventSender::new(sender);
    let chat = ChatWidget::new(ChatWidgetInit {
        app_event_tx: sender.clone(),
        frame_requester: FrameRequester::new(sender),
        enhanced_keys_supported: false,
        directory: "/project".to_string(),
        owned_transcript,
        fullscreen_preference: true,
        model_override: false,
        appearance: Appearance::new(&AgentTuiPreferences::default(), "/project".to_string()).0,
    });
    (chat, events)
}

/// The text of the cells the chat handed the transcript once complete
/// (a live entry when it completed), one string per cell, and the ops it
/// asked of the Agent.
fn drain(events: &Receiver<AppEvent>) -> (Vec<String>, Vec<Op>) {
    let mut cells = Vec::new();
    let mut ops = Vec::new();
    while let Ok(event) = events.try_recv() {
        match event {
            AppEvent::InsertHistoryCell(cell) => cells.push(cell_text(cell.as_ref())),
            AppEvent::LiveEntryCompleted(cell) => cells.push(cell_text(cell.as_ref())),
            AppEvent::Op(op) => ops.push(op),
            _ => {}
        }
    }
    (cells, ops)
}

fn cell_text(cell: &dyn HistoryCell) -> String {
    cell.display_lines(80)
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

fn command(name: &str) -> &'static CommandSpec {
    commands::find(name).unwrap()
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
fn a_submitted_message_is_written_and_sent_and_quit_exits() {
    let (mut chat, events) = chat();
    drain(&events);
    chat.submit_user_message("Do it".to_string());
    let (cells, ops) = drain(&events);
    assert!(cells[0].contains("› Do it"), "{cells:?}");
    assert_eq!(ops, vec![Op::UserInput("Do it".to_string())]);
    chat.dispatch_command(command("quit"), String::new());
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
    chat.dispatch_command(command("model"), String::new());
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
        session_id: Some(hanafubuki_agent::AgentSessionId::new(7)),
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
        older: Some(OlderTranscript {
            session_id: AgentSessionId::new(7),
            cursor: "older".to_string(),
            through_revision: AgentRevision::new(1),
        }),
        setup_needed: false,
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
        max_output_tokens: None,
        image_input: None,
        reasoning: None,
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
fn ctrl_o_between_two_ctrl_c_does_not_quit() {
    let (mut chat, events) = chat();
    chat.handle_turn_update(TurnUpdate::Started);
    drain(&events);
    let ctrl = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
    chat.handle_key_event(ctrl('c'));
    chat.handle_key_event(ctrl('o'));
    chat.handle_key_event(ctrl('c'));
    let events: Vec<_> = std::iter::from_fn(|| events.try_recv().ok()).collect();
    assert!(!events.iter().any(|event| matches!(event, AppEvent::Exit)));
    assert!(events
        .iter()
        .any(|event| matches!(event, AppEvent::Command(AppCommand::ToggleThinking))));
}

// Each panic says whether it was raised in a contained region, where the
// process's panic hook lets it unwind instead of aborting.
static PANICS: CommandSpec = CommandSpec {
    name: "panics",
    aliases: &[],
    description: "panics when run",
    inline_args: false,
    during_task: true,
    needs_session: false,
    dispatch: |_, _| panic!("the command broke (contained: {})", is_panic_contained()),
};

/// Its list panics when drawn.
static DRAWS_BADLY: CommandSpec = CommandSpec {
    name: "draws-badly",
    aliases: &[],
    description: "shows a list that panics when drawn",
    inline_args: false,
    during_task: true,
    needs_session: false,
    dispatch: |context, _| {
        context.show_selection_view(SelectionViewParams {
            header: Box::new(PanicsWhenDrawn),
            items: vec![SelectionItem {
                name: "item".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        })
    },
};

/// Its list's item panics when picked.
static PICKS_BADLY: CommandSpec = CommandSpec {
    name: "picks-badly",
    aliases: &[],
    description: "shows a list whose item panics when picked",
    inline_args: false,
    during_task: true,
    needs_session: false,
    dispatch: |context, _| {
        context.show_selection_view(SelectionViewParams {
            items: vec![SelectionItem {
                name: "item".to_string(),
                actions: vec![Box::new(|_| {
                    panic!("the pick broke (contained: {})", is_panic_contained())
                })],
                dismiss_on_select: true,
                ..Default::default()
            }],
            ..Default::default()
        })
    },
};

struct PanicsWhenDrawn;

impl Renderable for PanicsWhenDrawn {
    fn render(&self, _area: ratatui::layout::Rect, _buf: &mut ratatui::buffer::Buffer) {
        panic!("the view broke (contained: {})", is_panic_contained());
    }

    fn desired_height(&self, _width: u16) -> u16 {
        1
    }
}

/// `/name` failed with `message`, said so once, and is off; the composer
/// is back.
fn assert_turned_off(chat: &ChatWidget, cells: &[String], name: &'static str, message: &str) {
    assert_eq!(cells.len(), 1, "{cells:?}");
    let said = cells[0].replace('\n', " ");
    assert!(
        said.contains(&format!("'/{name}' failed and is turned off")) && said.contains(message),
        "{said}"
    );
    assert!(chat.bottom_pane.command_disabled(name));
    assert_eq!(chat.warnings.len(), 1, "{:?}", chat.warnings);
    assert!(chat.bottom_pane.no_modal_or_popup_active());
    assert!(screen(chat, 60).contains("Ask the Agent to do anything"));
}

#[test]
fn a_command_that_panics_is_turned_off_and_the_others_still_run() {
    let (mut chat, events) = chat();
    drain(&events);
    chat.dispatch_command(&PANICS, String::new());
    assert_turned_off(
        &chat,
        &drain(&events).0,
        "panics",
        "the command broke (contained: true)",
    );

    chat.dispatch_command(&PANICS, String::new());
    assert_eq!(drain(&events), (Vec::new(), Vec::new()));
    chat.dispatch_command(command("skills"), String::new());
    assert_eq!(drain(&events).1, vec![Op::Command(AgentCommand::Skills)]);
    chat.submit_user_message("Still here".to_string());
    assert_eq!(
        drain(&events).1,
        vec![Op::UserInput("Still here".to_string())]
    );
}

#[test]
fn a_command_whose_view_panics_when_drawn_is_turned_off() {
    let (mut chat, events) = chat();
    drain(&events);
    chat.dispatch_command(&DRAWS_BADLY, String::new());
    screen(&chat, 60);
    chat.reap_failed_views();
    assert_turned_off(
        &chat,
        &drain(&events).0,
        "draws-badly",
        "the view broke (contained: true)",
    );
}

#[test]
fn a_command_whose_view_panics_on_a_key_is_turned_off() {
    let (mut chat, events) = chat();
    drain(&events);
    chat.dispatch_command(&PICKS_BADLY, String::new());
    assert!(screen(&chat, 60).contains("item"));
    chat.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_turned_off(
        &chat,
        &drain(&events).0,
        "picks-badly",
        "the pick broke (contained: true)",
    );
    chat.dispatch_command(command("mode"), String::new());
    assert!(screen(&chat, 60).contains("Select Mode"));
}

#[test]
fn a_command_that_needs_a_session_asks_for_one_first() {
    let (mut chat, events) = chat();
    drain(&events);
    chat.dispatch_command(command("status"), String::new());
    assert_eq!(
        drain(&events).1,
        vec![Op::EnsureSession, Op::Command(AgentCommand::Status)]
    );
    chat.dispatch_command(command("mcp"), "verbose".to_string());
    assert_eq!(
        drain(&events).1,
        vec![Op::Command(AgentCommand::Mcp(McpCall::Report))]
    );
}

/// The events the chat sent but its transcript cells and ops.
fn other_events(events: &Receiver<AppEvent>) -> Vec<AppEvent> {
    std::iter::from_fn(|| events.try_recv().ok())
        .filter(|event| {
            !matches!(
                event,
                AppEvent::InsertHistoryCell(_) | AppEvent::Op(_) | AppEvent::ScheduleFrame(_)
            )
        })
        .collect()
}

#[test]
fn raw_takes_on_off_or_nothing_and_says_how_it_is_used() {
    let (mut chat, events) = chat();
    drain(&events);
    for (args, expected) in [("", None), ("on", Some(true)), (" OFF ", Some(false))] {
        chat.dispatch_command(command("raw"), args.to_string());
        let sent = other_events(&events);
        assert!(
            matches!(sent.as_slice(), [AppEvent::SetRawOutput(raw)] if *raw == expected),
            "{args:?}"
        );
    }
    chat.dispatch_command(command("raw"), "sideways".to_string());
    assert_eq!(drain(&events).0, vec!["■ Usage: /raw [on|off]"]);
}

#[test]
fn the_settings_commands_open_the_center_at_their_page() {
    let (mut chat, events) = chat();
    drain(&events);
    for (name, args, target) in [
        ("tui", "", "tui"),
        ("mcp", "", "mcp"),
        ("providers", "", "providers"),
        ("providers", "policy", "modelPolicy"),
        ("memories", "", "memory"),
        ("theme", "", "theme"),
        ("statusline", "", "statusline"),
        ("title", "", "title"),
        ("keymap", "", "keymap"),
        ("presets", "", "presets"),
        ("plugins", "", "packages"),
        ("hooks", "", "hook-review"),
    ] {
        chat.dispatch_command(command(name), args.to_string());
        let sent = other_events(&events);
        assert!(
            matches!(
                sent.as_slice(),
                [AppEvent::Workspace(WorkspaceEvent::OpenSettings { target: Some(opened), record: None })]
                    if opened == target
            ),
            "/{name} {args}: {} events",
            sent.len()
        );
        assert!(
            !chat.has_command_view(name),
            "/{name} shows no view over the composer"
        );
    }
    chat.dispatch_command(command("mcp"), "login demo".to_string());
    let sent = other_events(&events);
    assert!(
        matches!(
            sent.as_slice(),
            [AppEvent::Workspace(WorkspaceEvent::OpenSettings { target: Some(target), record: Some(record) })]
                if target == "mcp-login" && record == "demo"
        ),
        "{} events",
        sent.len()
    );
}

#[test]
fn the_hooks_are_checked_once_a_run_and_waiting_ones_are_a_warning() {
    let (mut chat, events) = chat();
    drain(&events);
    let opened = |session_id| OpenedSession {
        session_id,
        model: "fixture-model".to_string(),
        mode: hanafubuki_agent::AgentSessionMode::Default,
        transcript: Vec::new(),
        older: None,
        setup_needed: false,
    };
    chat.handle_turn_update(TurnUpdate::SessionOpened(opened(None)));
    assert_eq!(
        drain(&events).1,
        [Op::Command(AgentCommand::Hooks(HooksCall::Check))]
    );
    chat.handle_turn_update(TurnUpdate::SessionOpened(opened(Some(
        AgentSessionId::new(3),
    ))));
    assert!(drain(&events).1.is_empty());

    chat.handle_turn_update(TurnUpdate::Hooks(HooksReply::NeedReview(1)));
    assert_eq!(drain(&events).0, ["• 1 hook needs review: /hooks"]);
    let warnings: Vec<&str> = chat.warnings.iter().map(|w| w.text.as_str()).collect();
    assert_eq!(warnings, ["1 hook needs review: /hooks"]);
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

/// The cells written, a prompt's blank rows around it trimmed.
fn trimmed(cells: Vec<String>) -> Vec<String> {
    cells.iter().map(|cell| cell.trim().to_string()).collect()
}

fn steer_op(id: u64, text: &str) -> Op {
    Op::Steer {
        id,
        text: text.to_string(),
    }
}

#[test]
fn enter_during_a_turn_steers_it_one_message_at_a_time_and_writes_it_once_read() {
    let (mut chat, events) = chat();
    chat.handle_turn_update(TurnUpdate::Started);
    drain(&events);
    chat.submit_user_message("Look at the tests.".to_string());
    chat.submit_user_message("And the docs.".to_string());
    // Only the first is sent; neither is written yet.
    assert_eq!(
        drain(&events),
        (Vec::new(), vec![steer_op(0, "Look at the tests.")])
    );
    let waiting = screen(&chat, 80);
    assert!(
        waiting.contains("Messages to be submitted after next tool call"),
        "{waiting}"
    );
    assert!(waiting.contains("↳ And the docs."), "{waiting}");

    chat.handle_turn_update(TurnUpdate::Steered {
        id: 0,
        outcome: SteerOutcome::Accepted(1),
    });
    assert_eq!(drain(&events).1, vec![steer_op(1, "And the docs.")]);
    chat.handle_turn_update(TurnUpdate::Steered {
        id: 1,
        outcome: SteerOutcome::Accepted(2),
    });
    chat.handle_turn_update(TurnUpdate::SteeringConsumed(2));
    assert_eq!(
        trimmed(drain(&events).0),
        vec!["› Look at the tests.", "› And the docs."]
    );
    assert!(!screen(&chat, 80).contains("Messages to be submitted"));
}

#[test]
fn esc_with_steers_waiting_interrupts_and_sends_them_as_one_turn_keeping_the_draft() {
    let (mut chat, events) = chat();
    chat.handle_turn_update(TurnUpdate::Started);
    chat.submit_user_message("First.".to_string());
    chat.handle_turn_update(TurnUpdate::Steered {
        id: 0,
        outcome: SteerOutcome::Accepted(1),
    });
    chat.submit_user_message("Second.".to_string());
    chat.handle_turn_update(TurnUpdate::Steered {
        id: 1,
        outcome: SteerOutcome::Accepted(2),
    });
    chat.set_composer_text("draft".to_string());
    drain(&events);

    chat.handle_key_event(key(KeyCode::Esc));
    assert_eq!(drain(&events).1, vec![Op::Interrupt]);
    chat.handle_turn_update(TurnUpdate::Ended(TurnEnd::Interrupted));
    let (cells, ops) = drain(&events);
    assert!(
        cells[0].contains("Model interrupted to submit steer instructions."),
        "{cells:?}"
    );
    assert_eq!(cells[1].trim(), "› First.\n  Second.");
    assert_eq!(ops, vec![Op::UserInput("First.\nSecond.".to_string())]);
    assert_eq!(chat.bottom_pane.composer_text(), "draft");
}

#[test]
fn an_interrupt_puts_the_waiting_messages_back_before_the_draft() {
    let (mut chat, events) = chat();
    chat.handle_turn_update(TurnUpdate::Started);
    chat.submit_user_message("Steer.".to_string());
    chat.handle_turn_update(TurnUpdate::Steered {
        id: 0,
        outcome: SteerOutcome::Accepted(1),
    });
    chat.queue_user_message("Queued.".to_string());
    chat.set_composer_text("draft".to_string());
    drain(&events);
    // Ctrl-C (no Esc): nothing is sent.
    chat.handle_turn_update(TurnUpdate::Ended(TurnEnd::Interrupted));
    let (cells, ops) = drain(&events);
    assert!(cells[0].contains("Conversation interrupted"), "{cells:?}");
    assert!(ops.is_empty(), "{ops:?}");
    assert_eq!(chat.bottom_pane.composer_text(), "Steer.\nQueued.\ndraft");
    assert!(!screen(&chat, 80).contains("Queued follow-up inputs"));
}

#[test]
fn tab_queues_while_a_turn_runs_and_sends_at_once_with_nothing_running() {
    let (mut chat, events) = chat();
    chat.set_composer_text("now".to_string());
    chat.handle_key_event(key(KeyCode::Tab));
    assert_eq!(drain(&events).1, vec![Op::UserInput("now".to_string())]);

    chat.handle_turn_update(TurnUpdate::Started);
    chat.set_composer_text("later".to_string());
    let drafting = screen(&chat, 80);
    assert!(drafting.contains("tab to queue message"), "{drafting}");
    chat.handle_key_event(key(KeyCode::Tab));
    assert_eq!(drain(&events), (Vec::new(), Vec::new()));
    let queued = screen(&chat, 80);
    assert!(queued.contains("Queued follow-up inputs"), "{queued}");
    assert!(queued.contains("↳ later"), "{queued}");

    chat.handle_turn_update(TurnUpdate::Ended(TurnEnd::Failed("boom".to_string())));
    let (cells, ops) = drain(&events);
    assert_eq!(trimmed(cells), vec!["■ boom", "› later"]);
    assert_eq!(ops, vec![Op::UserInput("later".to_string())]);
}

#[test]
fn shift_left_and_alt_up_put_the_last_queued_message_back_before_the_draft() {
    let (mut chat, events) = chat();
    chat.handle_turn_update(TurnUpdate::Started);
    chat.queue_user_message("one".to_string());
    chat.queue_user_message("two".to_string());
    chat.set_composer_text("draft".to_string());
    chat.handle_key_event(KeyEvent::new(KeyCode::Left, KeyModifiers::SHIFT));
    assert_eq!(chat.bottom_pane.composer_text(), "two\ndraft");
    chat.handle_key_event(KeyEvent::new(KeyCode::Up, KeyModifiers::ALT));
    assert_eq!(chat.bottom_pane.composer_text(), "one\ntwo\ndraft");
    assert!(!screen(&chat, 80).contains("Queued follow-up inputs"));
    // Nothing queued: the key is the composer's, which does nothing.
    chat.handle_key_event(KeyEvent::new(KeyCode::Left, KeyModifiers::SHIFT));
    assert_eq!(chat.bottom_pane.composer_text(), "one\ntwo\ndraft");
    chat.handle_turn_update(TurnUpdate::Ended(TurnEnd::Completed));
    assert!(drain(&events).1.is_empty());
}

#[test]
fn a_popup_takes_tab_and_esc_before_the_queue_and_the_interrupt() {
    let (mut chat, events) = chat();
    chat.handle_turn_update(TurnUpdate::Started);
    chat.bottom_pane.insert_str("/stat");
    assert!(!chat.no_modal_or_popup_active());
    // Tab completes the command.
    chat.handle_key_event(key(KeyCode::Tab));
    assert_eq!(chat.bottom_pane.composer_text(), "/status ");
    assert!(!screen(&chat, 80).contains("Queued follow-up inputs"));
    chat.set_composer_text("@src".to_string());
    assert!(!chat.no_modal_or_popup_active());
    // Esc closes the popup, and only then interrupts.
    chat.handle_key_event(key(KeyCode::Esc));
    assert!(chat.no_modal_or_popup_active());
    assert!(drain(&events).1.is_empty());
    chat.handle_key_event(key(KeyCode::Esc));
    assert_eq!(drain(&events).1, vec![Op::Interrupt]);
}

#[test]
fn esc_interrupts_while_the_answer_streams() {
    let (mut chat, events) = chat();
    chat.handle_turn_update(TurnUpdate::Started);
    chat.handle_turn_update(TurnUpdate::MessageDelta("A first line.\nMore".to_string()));
    chat.on_commit_tick();
    // The answer streams: the status line stays for the turn.
    assert!(chat.bottom_pane.status_widget().is_some());
    drain(&events);
    chat.handle_key_event(key(KeyCode::Esc));
    assert_eq!(drain(&events).1, vec![Op::Interrupt]);
}

#[test]
fn a_session_opened_drops_what_waited_for_the_previous_one() {
    let (mut chat, events) = chat();
    chat.handle_turn_update(TurnUpdate::Started);
    chat.queue_user_message("for the old session".to_string());
    chat.handle_turn_update(TurnUpdate::SessionOpened(OpenedSession {
        session_id: None,
        model: "fixture-model".to_string(),
        mode: hanafubuki_agent::AgentSessionMode::Default,
        transcript: Vec::new(),
        older: None,
        setup_needed: false,
    }));
    assert!(!chat.has_pending_input());
    assert!(!screen(&chat, 80).contains("Queued follow-up inputs"));
    drain(&events);
    chat.submit_user_message("fresh".to_string());
    assert_eq!(drain(&events).1, vec![Op::UserInput("fresh".to_string())]);
}

#[test]
fn tab_in_the_file_popup_inserts_the_path_while_a_turn_runs() {
    let (mut chat, events) = chat();
    chat.handle_turn_update(TurnUpdate::Started);
    chat.set_composer_text("Read @not".to_string());
    chat.apply_file_search_result(
        "not".to_string(),
        vec![FileMatch {
            path: std::path::PathBuf::from("notes.txt"),
            root: std::path::PathBuf::from("/project"),
            primary: true,
            indices: Vec::new(),
        }],
    );
    chat.handle_key_event(key(KeyCode::Tab));
    assert_eq!(chat.bottom_pane.composer_text(), "Read notes.txt ");
    assert!(!screen(&chat, 80).contains("Queued follow-up inputs"));
    assert!(drain(&events).1.is_empty());
}

#[test]
fn a_message_that_reached_no_turn_goes_back_to_the_composer() {
    let (mut chat, events) = chat();
    chat.set_composer_text("draft".to_string());
    drain(&events);
    chat.handle_turn_update(TurnUpdate::Undelivered("Lost?".to_string()));
    let (cells, _) = drain(&events);
    assert!(cells[0].contains("did not reach the turn"), "{cells:?}");
    assert_eq!(chat.bottom_pane.composer_text(), "Lost?\ndraft");
}

#[test]
fn enter_before_the_turn_starts_queues_and_a_start_that_failed_releases_it() {
    let (mut chat, events) = chat();
    chat.submit_user_message("First.".to_string());
    // The turn has not started (the session may still be created): a turn
    // of its own, after.
    chat.submit_user_message("Second.".to_string());
    assert_eq!(drain(&events).1, vec![Op::UserInput("First.".to_string())]);
    assert!(chat.has_pending_input());
    // The session was not created, or the turn refused at its start.
    chat.handle_turn_update(TurnUpdate::Ended(TurnEnd::Failed(
        "no Provider".to_string(),
    )));
    let (cells, ops) = drain(&events);
    assert_eq!(trimmed(cells), vec!["■ no Provider", "› Second."]);
    assert_eq!(ops, vec![Op::UserInput("Second.".to_string())]);
    chat.handle_turn_update(TurnUpdate::Started);
    chat.handle_turn_update(TurnUpdate::Ended(TurnEnd::Completed));
    assert!(!chat.has_pending_input());
}
