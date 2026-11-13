// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Codex's history-search tests (`history_search.rs`,
//! `history_search_paste_tests.rs`), ported to this composer: its stored
//! messages come in one read, and Ctrl+C reaches the search through the
//! bottom pane.

use std::sync::mpsc::{channel, Receiver};

use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::Terminal;

use super::{match_ranges, HistorySearchStatus};
use crate::app_event::{AppEvent, AppEventSender};
use crate::bottom_pane::chat_composer::{ActivePopup, ChatComposer, InputResult};
use crate::bottom_pane::footer::FooterMode;
use crate::keymap::{self, Keymap};
use crate::render::renderable::Renderable;

/// A composer whose run sent `local`, oldest first, and the events it
/// sends.
fn composer_with(local: &[&str]) -> (ChatComposer, Receiver<AppEvent>) {
    let (tx, rx) = channel();
    let mut composer = ChatComposer::new(
        true,
        AppEventSender::new(tx),
        false,
        "Ask Codex to do anything".to_string(),
    );
    for entry in local {
        composer.history.record_local_submission((*entry).into());
    }
    (composer, rx)
}

/// Ctrl+R, then the stored messages read for it (newest first).
fn open(composer: &mut ChatComposer, stored: &[&str]) {
    press(composer, ctrl('r'));
    composer.on_recent_messages(stored.iter().map(|entry| entry.to_string()).collect());
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn press(composer: &mut ChatComposer, key: KeyEvent) -> InputResult {
    composer.handle_key_event(key).0
}

fn type_query(composer: &mut ChatComposer, query: &str) {
    for c in query.chars() {
        press(
            composer,
            KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
        );
    }
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn status(composer: &ChatComposer) -> Option<HistorySearchStatus> {
    composer.history_search.as_ref().map(|search| search.status)
}

fn text(composer: &ChatComposer) -> &str {
    composer.textarea.text()
}

fn render(composer: &ChatComposer, width: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, 5)).unwrap();
    terminal
        .draw(|frame| composer.render(frame.area(), frame.buffer_mut()))
        .unwrap();
    let cursor = composer.cursor_pos(Rect::new(0, 0, width, 5));
    format!("{}\nCursor: {cursor:?}", terminal.backend())
}

#[test]
fn the_search_opens_without_previewing_and_reads_the_stored_messages_once() {
    let (mut composer, events) = composer_with(&["remembered command"]);
    press(&mut composer, ctrl('r'));
    assert!(composer.history_search_active());
    assert!(composer.textarea.is_empty());
    assert_eq!(status(&composer), Some(HistorySearchStatus::Idle));
    assert!(matches!(
        events.try_recv(),
        Ok(AppEvent::ReadRecentMessages)
    ));

    press(&mut composer, key(KeyCode::Esc));
    press(&mut composer, ctrl('r'));
    assert!(events.try_recv().is_err(), "read once");
}

#[test]
fn a_match_is_accepted_as_a_draft_to_edit() {
    let (mut composer, _events) = composer_with(&["git status", "cargo test"]);
    composer.set_text_content("draft".to_string());
    open(&mut composer, &[]);
    assert_eq!(text(&composer), "draft");

    type_query(&mut composer, "git");
    assert_eq!(text(&composer), "git status");
    assert_eq!(status(&composer), Some(HistorySearchStatus::Match));

    assert_eq!(press(&mut composer, key(KeyCode::Enter)), InputResult::None);
    assert!(!composer.history_search_active());
    assert_eq!(text(&composer), "git status");
    assert_eq!(composer.textarea.cursor(), "git status".len());
    assert_eq!(composer.footer_mode(), FooterMode::ComposerHasDraft);
}

#[test]
fn the_steps_stay_on_the_match_at_both_ends() {
    let (mut composer, _events) = composer_with(&["Find and fix a bug in @filename"]);
    composer.set_text_content("draft".to_string());
    open(&mut composer, &[]);
    type_query(&mut composer, "bug");
    for step in [ctrl('r'), key(KeyCode::Up), ctrl('s'), key(KeyCode::Down)] {
        press(&mut composer, step);
        press(&mut composer, step);
        assert_eq!(text(&composer), "Find and fix a bug in @filename");
        assert_eq!(status(&composer), Some(HistorySearchStatus::Match));
    }
    // The preview opens no popup, though it holds an `@` token.
    assert!(matches!(composer.active_popup, ActivePopup::None));
    assert!(composer.current_file_query.is_none());
}

#[test]
fn the_steps_go_through_this_runs_matches_then_the_stored_ones() {
    let (mut composer, _events) = composer_with(&["echo one", "echo two"]);
    open(&mut composer, &["echo two", "!echo stored", "ls"]);
    type_query(&mut composer, "ECHO");
    assert_eq!(text(&composer), "echo two");
    press(&mut composer, ctrl('r'));
    assert_eq!(text(&composer), "echo one");
    press(&mut composer, key(KeyCode::Up));
    assert_eq!(text(&composer), "!echo stored");
    press(&mut composer, ctrl('r'));
    assert_eq!(text(&composer), "!echo stored");
    press(&mut composer, ctrl('s'));
    assert_eq!(text(&composer), "echo one");
    press(&mut composer, key(KeyCode::Down));
    assert_eq!(text(&composer), "echo two");
    // A `!` command is found as typed.
    press(&mut composer, ctrl('u'));
    type_query(&mut composer, "!ec");
    assert_eq!(text(&composer), "!echo stored");
    assert!(composer.footer_props().shell_command);
}

#[test]
fn a_search_waits_for_the_stored_messages() {
    let (mut composer, _events) = composer_with(&["local entry"]);
    composer.set_text_content("draft".to_string());
    press(&mut composer, ctrl('r'));
    type_query(&mut composer, "earlier");
    assert_eq!(status(&composer), Some(HistorySearchStatus::Searching));
    assert_eq!(text(&composer), "draft");
    composer.on_recent_messages(vec!["an earlier message".to_string()]);
    assert_eq!(status(&composer), Some(HistorySearchStatus::Match));
    assert_eq!(text(&composer), "an earlier message");

    let (mut composer, _events) = composer_with(&[]);
    press(&mut composer, ctrl('r'));
    type_query(&mut composer, "missing");
    composer.on_recent_messages(Vec::new());
    assert_eq!(status(&composer), Some(HistorySearchStatus::NoMatch));
}

#[test]
fn esc_puts_the_draft_back_with_its_cursor_and_resets_navigation() {
    let (mut composer, _events) = composer_with(&["oldest matching entry", "newest entry"]);
    composer.set_text_content("draft".to_string());
    composer.textarea.set_cursor(2);
    open(&mut composer, &[]);
    type_query(&mut composer, "match");
    assert_eq!(text(&composer), "oldest matching entry");

    press(&mut composer, key(KeyCode::Esc));
    assert!(!composer.history_search_active());
    assert_eq!(text(&composer), "draft");
    assert_eq!(composer.textarea.cursor(), 2);

    composer.set_text_content(String::new());
    press(&mut composer, key(KeyCode::Up));
    assert_eq!(text(&composer), "newest entry");
}

#[test]
fn ctrl_c_puts_the_draft_back() {
    let (mut composer, _events) = composer_with(&["remembered command"]);
    composer.set_text_content("draft".to_string());
    open(&mut composer, &[]);
    type_query(&mut composer, "r");
    assert_eq!(text(&composer), "remembered command");
    // The raw control character some terminals send.
    press(&mut composer, key(KeyCode::Char('\u{3}')));
    assert!(!composer.history_search_active());
    assert_eq!(text(&composer), "draft");

    open(&mut composer, &[]);
    type_query(&mut composer, "r");
    // Ctrl+C itself comes through the bottom pane.
    assert!(composer.cancel_history_search());
    assert_eq!(text(&composer), "draft");
    assert!(!composer.cancel_history_search());
}

#[test]
fn no_match_shows_the_draft_and_keeps_the_search_open() {
    let (mut composer, _events) = composer_with(&["git status"]);
    composer.set_text_content("draft".to_string());
    open(&mut composer, &[]);
    type_query(&mut composer, "zzz");
    assert!(composer.history_search_active());
    assert_eq!(text(&composer), "draft");
    assert_eq!(status(&composer), Some(HistorySearchStatus::NoMatch));
    // Enter takes nothing.
    press(&mut composer, key(KeyCode::Enter));
    assert!(composer.history_search_active());
    press(&mut composer, key(KeyCode::Backspace));
    press(&mut composer, key(KeyCode::Backspace));
    press(&mut composer, key(KeyCode::Backspace));
    assert_eq!(status(&composer), Some(HistorySearchStatus::Idle));
}

#[test]
fn a_pending_first_char_is_kept_before_the_search() {
    let (mut composer, _events) = composer_with(&[]);
    press(&mut composer, key(KeyCode::Char('h')));
    assert!(composer.is_in_paste_burst());
    open(&mut composer, &[]);
    assert!(!composer.is_in_paste_burst());
    assert_eq!(text(&composer), "h");
    press(&mut composer, key(KeyCode::Esc));
    assert_eq!(text(&composer), "h");
}

#[test]
fn a_draft_with_a_large_paste_survives_a_match_and_esc() {
    let (mut composer, _events) = composer_with(&["git status"]);
    let large = "x".repeat(1_001);
    composer.handle_paste(large.clone());
    let placeholder = text(&composer).to_string();
    open(&mut composer, &[]);
    composer.handle_paste("git".to_string());
    assert_eq!(text(&composer), "git status");
    assert_eq!(composer.history_search.as_ref().unwrap().query, "git");
    composer.handle_paste(" missing".to_string());
    assert_eq!(text(&composer), placeholder);
    composer.handle_paste("\n\t".to_string());
    assert_eq!(
        composer.history_search_line().unwrap().to_string(),
        "reverse-i-search: git missing↵⇥  no match"
    );

    press(&mut composer, key(KeyCode::Esc));
    assert_eq!(text(&composer), placeholder);
    assert_eq!(composer.current_text_with_pending(), large);
}

#[test]
fn a_message_given_back_during_the_search_goes_into_the_draft() {
    let (mut composer, _events) = composer_with(&["git status"]);
    composer.set_text_content("draft".to_string());
    open(&mut composer, &[]);
    type_query(&mut composer, "git");
    composer.restore_draft("waiting message".into());
    assert_eq!(text(&composer), "git status");
    press(&mut composer, key(KeyCode::Esc));
    assert_eq!(text(&composer), "waiting message\ndraft");

    // With no match shown, the draft shows, and changes in place.
    open(&mut composer, &[]);
    composer.restore_draft("another".into());
    assert_eq!(text(&composer), "another\nwaiting message\ndraft");
    press(&mut composer, key(KeyCode::Esc));
    assert_eq!(text(&composer), "another\nwaiting message\ndraft");
}

#[test]
fn a_previewed_command_opens_no_popup() {
    let (mut composer, events) = composer_with(&["/model gpt", "@src/main.rs"]);
    open(&mut composer, &[]);
    let _ = events.try_recv();
    type_query(&mut composer, "/mo");
    assert_eq!(text(&composer), "/model gpt");
    press(&mut composer, ctrl('u'));
    type_query(&mut composer, "@src");
    assert_eq!(text(&composer), "@src/main.rs");
    assert!(events.try_recv().is_err(), "no file search");
    assert!(composer.history_search_active());
}

#[test]
fn matches_are_highlighted_until_accepted() {
    let (mut composer, _events) = composer_with(&["cargo test", "git status git"]);
    open(&mut composer, &[]);
    type_query(&mut composer, "GIT");
    let area = Rect::new(0, 0, 60, 8);
    let mut buf = Buffer::empty(area);
    composer.render(area, &mut buf);
    let row = |buf: &Buffer, x: u16| buf[(2 + x, 1)].style().add_modifier;
    assert_eq!(buf[(2, 1)].symbol(), "g");
    for x in [0, 1, 2, 11, 12, 13] {
        assert!(
            row(&buf, x).contains(Modifier::REVERSED | Modifier::BOLD),
            "{x}"
        );
    }
    assert!(!row(&buf, 3).contains(Modifier::REVERSED));

    press(&mut composer, key(KeyCode::Enter));
    let mut accepted = Buffer::empty(area);
    composer.render(area, &mut accepted);
    assert!(!row(&accepted, 0).contains(Modifier::REVERSED));
}

#[test]
fn match_ranges_ignore_case_on_character_boundaries() {
    assert_eq!(match_ranges("git status git", "GIT"), vec![0..3, 11..14]);
    for (text, query, expected) in [
        ("aİ i", "i", vec![1..3, 4..5]),
        ("İİ", "i", vec![0..2, 2..4]),
        ("İİİ", "\u{307}i", vec![0..4, 2..6]),
        ("éÉ é", "É", vec![0..2, 2..4, 5..7]),
        ("aaaaa", "aa", vec![0..2, 2..4]),
        ("", "x", vec![]),
        ("abc", "z", vec![]),
        ("git", "", vec![]),
    ] {
        assert_eq!(match_ranges(text, query), expected, "{text:?} {query:?}");
    }
}

#[test]
fn a_remapped_search_key_opens_it_and_steps() {
    let config = [(
        "composer".to_string(),
        [("history_search".to_string(), vec!["f2".to_string()])].into(),
    )]
    .into();
    let (remapped, warnings) = Keymap::resolve(&config);
    assert!(warnings.is_empty(), "{warnings:#?}");
    keymap::install(remapped);
    let (mut composer, _events) = composer_with(&["first", "fir"]);
    press(&mut composer, ctrl('r'));
    let ctrl_r_opened = composer.history_search_active();
    press(&mut composer, key(KeyCode::F(2)));
    let f2_opened = composer.history_search_active();
    composer.on_recent_messages(Vec::new());
    type_query(&mut composer, "fir");
    press(&mut composer, key(KeyCode::F(2)));
    let stepped = text(&composer).to_string();
    keymap::install(Keymap::default());
    assert!(!ctrl_r_opened);
    assert!(f2_opened);
    assert_eq!(stepped, "first");
}

#[test]
fn search_snapshots() {
    for width in [70, 30] {
        let (mut composer, _events) = composer_with(&["git status", "cargo test"]);
        composer.set_text_content("draft".to_string());
        press(&mut composer, ctrl('r'));
        type_query(&mut composer, "earlier");
        insta::assert_snapshot!(format!("searching_{width}"), render(&composer, width));

        composer.on_recent_messages(vec!["an earlier message".to_string()]);
        insta::assert_snapshot!(format!("match_{width}"), render(&composer, width));

        press(&mut composer, ctrl('u'));
        type_query(&mut composer, "zzz");
        insta::assert_snapshot!(format!("no_match_{width}"), render(&composer, width));
    }
}
