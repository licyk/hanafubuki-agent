// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use std::sync::mpsc::{channel, Receiver};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app_event::{AppEvent, AppEventSender};
use crate::bottom_pane::chat_composer::{ActivePopup, ChatComposer};
use crate::bottom_pane::InputResult;

fn composer(local: &[&str]) -> (ChatComposer, Receiver<AppEvent>) {
    let (tx, rx) = channel();
    let mut composer = ChatComposer::new(true, AppEventSender::new(tx), false, "Ask".into());
    for entry in local {
        composer.history.record_local_submission((*entry).into());
    }
    (composer, rx)
}

fn press(composer: &mut ChatComposer, code: KeyCode) {
    composer.handle_key_event(KeyEvent::new(code, KeyModifiers::NONE));
}

fn read(composer: &mut ChatComposer, entries: &[&str]) {
    composer.on_recent_messages(entries.iter().map(|entry| entry.to_string()).collect());
}

#[test]
fn submitted_commands_join_messages_and_recalled_commands_do_not_capture_arrow_keys() {
    let (mut composer, _) = composer(&[]);
    read(&mut composer, &[]);
    for input in [
        "ordinary message",
        "/goal 检查 Python 版本\n然后报告结果",
        "/status",
    ] {
        composer.set_text_content(input.into());
        let result = composer
            .handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .0;
        assert!(matches!(
            result,
            InputResult::Submitted(_) | InputResult::Command(..)
        ));
    }
    for expected in [
        "/status",
        "/goal 检查 Python 版本\n然后报告结果",
        "ordinary message",
    ] {
        press(&mut composer, KeyCode::Up);
        assert_eq!(composer.textarea.text(), expected);
        assert!(matches!(composer.active_popup, ActivePopup::None));
    }
    for expected in ["/goal 检查 Python 版本\n然后报告结果", "/status", ""] {
        press(&mut composer, KeyCode::Down);
        assert_eq!(composer.textarea.text(), expected);
    }
    press(&mut composer, KeyCode::Up);
    let result = composer
        .handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .0;
    assert!(
        matches!(result, InputResult::Command(command, args) if command.name == "status" && args.is_empty())
    );
    press(&mut composer, KeyCode::Up);
    assert_eq!(composer.textarea.text(), "/status");
    press(&mut composer, KeyCode::Up);
    assert!(
        composer.textarea.text().starts_with("/goal "),
        "consecutive duplicate submissions are coalesced"
    );
}

#[test]
fn popup_selection_records_the_executed_name_and_tab_only_completes() {
    let (mut composer, _) = composer(&[]);
    read(&mut composer, &[]);
    composer.set_text_content("/sta".into());
    assert!(matches!(composer.active_popup, ActivePopup::Command(_)));
    let result = composer
        .handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .0;
    assert!(matches!(result, InputResult::Command(command, _) if command.name == "status"));
    press(&mut composer, KeyCode::Up);
    assert_eq!(composer.textarea.text(), "/status");

    press(&mut composer, KeyCode::Down);
    composer.set_text_content("/mod".into());
    press(&mut composer, KeyCode::Tab);
    composer.set_text_content(String::new());
    press(&mut composer, KeyCode::Up);
    assert_eq!(composer.textarea.text(), "/status");
}

#[test]
fn command_arguments_expand_pastes_and_tab_submission_records_them() {
    let (mut composer, _) = composer(&[]);
    read(&mut composer, &[]);
    let arguments = "检查项目并报告结果。".repeat(300);
    composer.handle_paste(format!("/goal {arguments}"));
    composer.set_task_running(true);
    let result = composer
        .handle_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .0;
    assert!(
        matches!(result, InputResult::Command(command, args) if command.name == "goal" && args == arguments)
    );
    assert!(composer.pending_pastes.is_empty());
    press(&mut composer, KeyCode::Up);
    assert_eq!(composer.textarea.text(), format!("/goal {arguments}"));
}

#[test]
fn arrows_recall_stored_messages_after_restart_and_clear_past_newest() {
    let (mut composer, events) = composer(&[]);
    press(&mut composer, KeyCode::Up);
    assert!(matches!(
        events.try_recv(),
        Ok(AppEvent::ReadRecentMessages)
    ));
    press(&mut composer, KeyCode::Up);
    assert!(events.try_recv().is_err());
    read(&mut composer, &["recent", "older"]);
    assert_eq!(composer.textarea.text(), "recent");
    assert_eq!(composer.textarea.cursor(), "recent".len());
    press(&mut composer, KeyCode::Up);
    assert_eq!(composer.textarea.text(), "older");
    press(&mut composer, KeyCode::Up);
    assert_eq!(composer.textarea.text(), "older");
    press(&mut composer, KeyCode::Down);
    assert_eq!(composer.textarea.text(), "recent");
    press(&mut composer, KeyCode::Down);
    assert!(composer.is_empty());
    press(&mut composer, KeyCode::Up);
    assert_eq!(composer.textarea.text(), "recent");
    assert!(events.try_recv().is_err());
}

#[test]
fn local_entries_are_immediate_and_join_the_pending_stored_entries() {
    let (mut composer, events) = composer(&["one", "two", "one"]);
    for expected in ["one", "two", "one", "one"] {
        press(&mut composer, KeyCode::Up);
        assert_eq!(composer.textarea.text(), expected);
    }
    assert!(matches!(
        events.try_recv(),
        Ok(AppEvent::ReadRecentMessages)
    ));
    assert!(events.try_recv().is_err());
    read(&mut composer, &["one", "two", "stored"]);
    assert_eq!(composer.textarea.text(), "stored");
    press(&mut composer, KeyCode::Down);
    assert_eq!(composer.textarea.text(), "one");
}

#[test]
fn editing_or_leaving_a_pending_recall_prevents_a_late_reply_overwriting_the_draft() {
    for edit in [KeyCode::Char('x'), KeyCode::Down, KeyCode::Left] {
        let (mut composer, _) = composer(&[]);
        press(&mut composer, KeyCode::Up);
        press(&mut composer, edit);
        let draft = composer.textarea.text().to_string();
        read(&mut composer, &["stored"]);
        assert_eq!(composer.textarea.text(), draft, "after {edit:?}");
    }
    let (mut composer, _) = composer(&[]);
    press(&mut composer, KeyCode::Up);
    composer.handle_paste("pasted draft".into());
    read(&mut composer, &["stored"]);
    assert_eq!(composer.textarea.text(), "pasted draft");
}

#[test]
fn clearing_new_input_does_not_reactivate_a_pending_recall() {
    let (mut composer, _) = composer(&[]);
    press(&mut composer, KeyCode::Up);
    composer.handle_paste("x".into());
    press(&mut composer, KeyCode::Backspace);
    assert!(composer.is_empty());
    read(&mut composer, &["stored"]);
    assert!(composer.is_empty());
    press(&mut composer, KeyCode::Up);
    assert_eq!(composer.textarea.text(), "stored");
}

#[test]
fn restored_draft_cancels_pending_recall() {
    let (mut composer, _) = composer(&[]);
    press(&mut composer, KeyCode::Up);
    composer.set_text_content("restored draft".into());
    read(&mut composer, &["stored"]);
    assert_eq!(composer.textarea.text(), "restored draft");
}

#[test]
fn multiline_edits_keep_arrow_movement_but_recalled_boundaries_browse() {
    let (mut composer, events) = composer(&["older", "first\nsecond"]);
    composer.set_text_content("draft\nline".into());
    press(&mut composer, KeyCode::Up);
    assert_eq!(composer.textarea.text(), "draft\nline");
    assert!(composer.textarea.cursor() < "draft\n".len());
    assert!(events.try_recv().is_err());

    composer.set_text_content(String::new());
    press(&mut composer, KeyCode::Up);
    assert_eq!(composer.textarea.text(), "first\nsecond");
    composer.textarea.set_cursor(8);
    press(&mut composer, KeyCode::Up);
    assert_eq!(composer.textarea.text(), "first\nsecond");
    assert!(composer.textarea.cursor() < 6);
    composer.textarea.set_cursor(0);
    press(&mut composer, KeyCode::Up);
    assert_eq!(composer.textarea.text(), "older");
}

#[test]
fn recalled_commands_and_mentions_do_not_open_popups() {
    let (mut composer, _) = composer(&[]);
    press(&mut composer, KeyCode::Up);
    read(&mut composer, &["/model", "look at @file"]);
    for expected in ["/model", "look at @file"] {
        assert_eq!(composer.textarea.text(), expected);
        assert!(matches!(composer.active_popup, ActivePopup::None));
        press(&mut composer, KeyCode::Up);
    }
}

#[test]
fn search_and_arrows_share_one_read() {
    let (mut composer, events) = composer(&[]);
    press(&mut composer, KeyCode::Up);
    assert!(matches!(
        events.try_recv(),
        Ok(AppEvent::ReadRecentMessages)
    ));
    composer.handle_key_event(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
    read(&mut composer, &["stored"]);
    assert!(composer.is_empty(), "opening search cancels pending Up");
    press(&mut composer, KeyCode::Esc);
    composer.handle_key_event(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
    assert_eq!(composer.textarea.text(), "stored");
    composer.handle_key_event(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL));
    assert!(composer.is_empty());
    assert!(events.try_recv().is_err());
}

#[test]
fn empty_read_completes_navigation_and_release_events_do_not_step() {
    let (mut composer, _) = composer(&[]);
    press(&mut composer, KeyCode::Up);
    read(&mut composer, &[]);
    assert!(composer.is_empty());
    composer.history.record_local_submission("older".into());
    composer.history.record_local_submission("newer".into());
    press(&mut composer, KeyCode::Up);
    composer.handle_key_event(KeyEvent::new_with_kind(
        KeyCode::Up,
        KeyModifiers::NONE,
        KeyEventKind::Release,
    ));
    assert_eq!(composer.textarea.text(), "newer");
    composer.handle_key_event(KeyEvent::new_with_kind(
        KeyCode::Up,
        KeyModifiers::NONE,
        KeyEventKind::Repeat,
    ));
    assert_eq!(composer.textarea.text(), "older");
}
