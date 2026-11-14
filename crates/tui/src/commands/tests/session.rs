//! The session commands' dispatch: what each asks of the Agent and of the
//! app for its arguments, and what their views do with the replies.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::*;
use hanafubuki_agent::AgentTuiPreferences;

use crate::agent_bridge::{ExportDestination, TurnUpdate};
use crate::app_event::AppCommand;
use crate::appearance::Appearance;
use crate::chatwidget::{ChatWidget, ChatWidgetInit};
use crate::render::renderable::Renderable;
use crate::tui::FrameRequester;

pub(crate) fn chat(owned_transcript: bool) -> (ChatWidget, Receiver<AppEvent>) {
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

/// What the chat sent, but frames: the ops, the app commands and the
/// cells' text.
#[derive(Debug, PartialEq)]
pub(crate) enum Sent {
    Op(Op),
    App(AppCommand),
    Cell(String),
    ResumePicker,
}

pub(crate) fn sent(events: &Receiver<AppEvent>) -> Vec<Sent> {
    std::iter::from_fn(|| events.try_recv().ok())
        .filter_map(to_sent)
        .collect()
}

pub(crate) fn to_sent(event: AppEvent) -> Option<Sent> {
    match event {
        AppEvent::Op(op) => Some(Sent::Op(op)),
        AppEvent::Command(command) => Some(Sent::App(command)),
        AppEvent::OpenResumePicker => Some(Sent::ResumePicker),
        AppEvent::InsertHistoryCell(cell) => Some(Sent::Cell(
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
        )),
        _ => None,
    }
}

pub(crate) fn call(call: SessionCall) -> Sent {
    Sent::Op(Op::Command(AgentCommand::Session(call)))
}

pub(crate) fn run(chat: &mut ChatWidget, events: &Receiver<AppEvent>, line: &str) -> Vec<Sent> {
    sent(events);
    let (name, args) = line.split_once(' ').unwrap_or((line, ""));
    chat.dispatch_command(find(name).unwrap(), args.to_string());
    sent(events)
}

pub(crate) fn key(chat: &mut ChatWidget, code: KeyCode) {
    chat.handle_key_event(KeyEvent::new(code, KeyModifiers::NONE));
}

pub(crate) fn screen(chat: &ChatWidget, width: u16) -> String {
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

#[test]
fn commands_with_an_argument_act_on_it_at_once() {
    let (mut chat, events) = chat(false);
    assert_eq!(
        run(&mut chat, &events, "rename  Login fix "),
        [
            Sent::Op(Op::EnsureSession),
            call(SessionCall::Rename("Login fix".to_string()))
        ]
    );
    assert_eq!(
        run(&mut chat, &events, "fork experiment"),
        [call(SessionCall::Fork(Some("experiment".to_string())))]
    );
    assert_eq!(
        run(&mut chat, &events, "fork"),
        [call(SessionCall::Fork(None))]
    );
    assert_eq!(
        run(&mut chat, &events, "clear next task"),
        [
            Sent::App(AppCommand::ClearScreen),
            call(SessionCall::Clear(Some("next task".to_string())))
        ]
    );
    assert_eq!(
        run(&mut chat, &events, "export notes/chat.md"),
        [call(SessionCall::Export(ExportDestination::File(
            PathBuf::from("notes/chat.md")
        )))]
    );
    assert_eq!(
        run(&mut chat, &events, "resume 12"),
        [Sent::App(AppCommand::ResumeNamed("12".to_string()))]
    );
    assert_eq!(run(&mut chat, &events, "resume"), [Sent::ResumePicker]);
}

#[test]
fn terminal_list_links_to_the_task_manager() {
    let lines = crate::commands::terminals::terminal_lines(&[]);
    insta::assert_snapshot!(
        "ps_empty_tasks_hint",
        lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn reports_ask_the_agent_and_init_submits_the_prompt() {
    let (mut chat, events) = chat(false);
    for (line, expected) in [
        ("pwd", SessionCall::Roots),
        ("cwd", SessionCall::Roots),
        ("ps", SessionCall::Terminals),
        ("copy", SessionCall::LastAnswer),
    ] {
        assert_eq!(run(&mut chat, &events, line), [call(expected)], "/{line}");
    }
    assert_eq!(
        run(&mut chat, &events, "recap"),
        [
            Sent::Cell("• Writing a recap…".to_string()),
            call(SessionCall::Recap)
        ]
    );
    assert_eq!(
        run(&mut chat, &events, "init"),
        [Sent::App(AppCommand::Submit(init::INIT_PROMPT.to_string()))]
    );
    assert!(init::INIT_PROMPT.contains("do not overwrite or modify it"));
}

#[test]
fn archive_delete_and_stop_ask_first_and_no_is_selected() {
    for (line, question, yes) in [
        ("archive", "Archive this session?", SessionCall::Archive),
        (
            "delete",
            "cannot be undone; its child sessions",
            SessionCall::Delete,
        ),
        (
            "stop",
            "Stop all background terminals?",
            SessionCall::StopTerminals,
        ),
        (
            "clean",
            "Stop all background terminals?",
            SessionCall::StopTerminals,
        ),
    ] {
        let (mut chat, events) = chat(false);
        assert_eq!(run(&mut chat, &events, line), []);
        assert!(screen(&chat, 100).contains(question), "/{line}");
        key(&mut chat, KeyCode::Enter);
        assert_eq!(sent(&events), [], "/{line}: no is first");

        run(&mut chat, &events, line);
        key(&mut chat, KeyCode::Down);
        key(&mut chat, KeyCode::Enter);
        assert_eq!(sent(&events), [call(yes)], "/{line}");
    }
}

#[test]
fn the_rename_prompt_takes_the_title_and_tab_takes_the_suggestion() {
    let (mut chat, events) = chat(false);
    assert_eq!(
        run(&mut chat, &events, "rename"),
        [Sent::Op(Op::EnsureSession), call(SessionCall::RenamePrompt)]
    );
    assert!(screen(&chat, 80).contains("Suggesting a title…"));

    chat.handle_turn_update(TurnUpdate::Session(SessionReply::Title(Some(
        "Old".to_string(),
    ))));
    chat.handle_turn_update(TurnUpdate::Session(SessionReply::TitleSuggestion(Some(
        "Fix the login timeout".to_string(),
    ))));
    let shown = screen(&chat, 80);
    assert!(shown.contains("Old"), "{shown}");
    assert!(
        shown.contains("Suggested: Fix the login timeout"),
        "{shown}"
    );

    key(&mut chat, KeyCode::Tab);
    key(&mut chat, KeyCode::Enter);
    assert_eq!(
        sent(&events),
        [call(SessionCall::Rename(
            "Fix the login timeout".to_string()
        ))]
    );
    chat.handle_turn_update(TurnUpdate::Session(SessionReply::Renamed(
        "Fix the login timeout".to_string(),
    )));
    assert_eq!(
        sent(&events),
        [Sent::Cell(
            "• Session renamed to \"Fix the login timeout\"".to_string()
        )]
    );
}

#[test]
fn without_a_suggestion_the_prompt_says_so_and_keeps_what_is_typed() {
    let (mut chat, events) = chat(false);
    run(&mut chat, &events, "rename");
    key(&mut chat, KeyCode::Char('x'));
    chat.handle_turn_update(TurnUpdate::Session(SessionReply::Title(Some(
        "Old".to_string(),
    ))));
    chat.handle_turn_update(TurnUpdate::Session(SessionReply::TitleSuggestion(None)));
    let shown = screen(&chat, 80);
    assert!(shown.contains("No title suggestion"), "{shown}");
    key(&mut chat, KeyCode::Tab);
    key(&mut chat, KeyCode::Enter);
    assert_eq!(sent(&events), [call(SessionCall::Rename("x".to_string()))]);
}

#[test]
fn copy_offers_the_answer_its_parts_and_in_fullscreen_selection() {
    let answer = "Run:\n\n```sh\nmake\n```\n";
    let (mut owned, events) = chat(true);
    run(&mut owned, &events, "copy");
    owned.handle_turn_update(TurnUpdate::Session(SessionReply::LastAnswer(Some(
        answer.to_string(),
    ))));
    let shown = screen(&owned, 80);
    for choice in ["Whole answer", "sh code", "Select in the transcript"] {
        assert!(shown.contains(choice), "{choice}: {shown}");
    }
    key(&mut owned, KeyCode::Down);
    key(&mut owned, KeyCode::Enter);
    assert_eq!(
        sent(&events),
        [Sent::App(AppCommand::Copy("make\n".to_string()))]
    );

    let (mut plain, events) = chat(false);
    run(&mut plain, &events, "copy");
    plain.handle_turn_update(TurnUpdate::Session(SessionReply::LastAnswer(None)));
    assert_eq!(
        sent(&events),
        [Sent::Cell("• There is no answer to copy yet".to_string())]
    );
}

#[test]
fn export_offers_the_clipboard_or_a_file_at_the_default_path() {
    let (mut chat, events) = chat(false);
    run(&mut chat, &events, "export");
    key(&mut chat, KeyCode::Enter);
    assert_eq!(
        sent(&events),
        [call(SessionCall::Export(ExportDestination::Clipboard))]
    );
    chat.handle_turn_update(TurnUpdate::Session(SessionReply::ExportedText(
        "# Conversation".to_string(),
    )));
    assert_eq!(
        sent(&events),
        [Sent::App(AppCommand::Copy("# Conversation".to_string()))]
    );

    run(&mut chat, &events, "export");
    key(&mut chat, KeyCode::Down);
    key(&mut chat, KeyCode::Enter);
    assert_eq!(sent(&events), [call(SessionCall::ExportPath)]);
    let default = PathBuf::from("/project/hanafubuki-agent-session-7.md");
    chat.handle_turn_update(TurnUpdate::Session(SessionReply::ExportPath(
        default.clone(),
    )));
    assert!(screen(&chat, 80).contains("hanafubuki-agent-session-7.md"));
    key(&mut chat, KeyCode::Enter);
    assert_eq!(
        sent(&events),
        [call(SessionCall::Export(ExportDestination::File(default)))]
    );
}

#[test]
fn a_failed_call_names_its_command() {
    let (mut chat, events) = chat(false);
    chat.handle_turn_update(TurnUpdate::Session(SessionReply::Failed {
        command: "archive",
        error: "there is no session yet".to_string(),
    }));
    assert_eq!(
        sent(&events),
        [Sent::Cell(
            "■ /archive: there is no session yet".to_string()
        )]
    );
}
