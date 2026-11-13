// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Ctrl+G in the composer: when it asks for the editor, and the edited
//! text back as the draft (Codex's `apply_external_edit_*` tests, with a
//! large paste in place of an image).

use std::collections::BTreeMap;
use std::sync::mpsc::{channel, Receiver};

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;

use super::super::{ChatComposer, LARGE_PASTE_CHAR_THRESHOLD};
use crate::app_event::{AppEvent, AppEventSender};
use crate::keymap::{self, Keymap};
use crate::render::renderable::Renderable;

fn new_composer() -> (ChatComposer, Receiver<AppEvent>) {
    let (tx, rx) = channel();
    let composer = ChatComposer::new(true, AppEventSender::new(tx), false, String::new());
    (composer, rx)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn asked(events: &Receiver<AppEvent>) -> bool {
    events
        .try_iter()
        .any(|event| matches!(event, AppEvent::ExternalEditor))
}

fn footer(composer: &ChatComposer) -> String {
    let area = Rect::new(0, 0, 60, 4);
    let mut buf = Buffer::empty(area);
    composer.render(area, &mut buf);
    (0..area.width)
        .map(|x| buf[(x, area.height - 1)].symbol().to_string())
        .collect()
}

fn large(c: char) -> String {
    c.to_string().repeat(LARGE_PASTE_CHAR_THRESHOLD + 5)
}

#[test]
fn ctrl_g_asks_for_the_editor_on_a_draft_or_an_empty_composer() {
    let (mut composer, events) = new_composer();
    composer.handle_key_event(ctrl('g'));
    assert!(asked(&events));
    composer.set_text_content("a draft".to_string());
    composer.handle_key_event(ctrl('g'));
    assert!(asked(&events));
    // The key is the editor's, not text.
    assert_eq!(composer.textarea.text(), "a draft");
}

#[test]
fn ctrl_g_does_nothing_with_a_popup_or_a_search_open_or_without_the_keys() {
    let (mut composer, events) = new_composer();
    composer.set_text_content("/mo".to_string());
    assert!(composer.popup_active());
    composer.handle_key_event(ctrl('g'));
    assert!(!asked(&events));
    assert_eq!(composer.textarea.text(), "/mo");
    assert!(composer.popup_active());

    let (mut composer, events) = new_composer();
    composer.set_text_content("a draft".to_string());
    composer.handle_key_event(ctrl('r'));
    assert!(composer.history_search_active());
    composer.handle_key_event(ctrl('g'));
    assert!(!asked(&events));
    assert!(composer.history_search_active());
    composer.cancel_history_search();
    assert_eq!(composer.textarea.text(), "a draft");

    let (mut composer, events) = new_composer();
    composer.set_has_focus(false);
    composer.handle_key_event(ctrl('g'));
    assert!(!asked(&events));
}

#[test]
fn the_editors_key_can_be_rebound() {
    let config = BTreeMap::from([(
        "composer".to_string(),
        BTreeMap::from([("external_editor".to_string(), vec!["f9".to_string()])]),
    )]);
    keymap::install(Keymap::resolve(&config).0);
    let (mut composer, events) = new_composer();
    composer.handle_key_event(KeyEvent::new(KeyCode::F(9), KeyModifiers::NONE));
    assert!(asked(&events));
    composer.handle_key_event(ctrl('g'));
    assert!(!asked(&events));
    keymap::install(Keymap::default());
}

#[test]
fn the_footer_says_how_to_come_back_while_the_editor_has_the_draft() {
    let (mut composer, _events) = new_composer();
    composer.set_external_editor_open(true);
    assert!(footer(&composer).contains("Save and close external editor to continue."));
    composer.set_external_editor_open(false);
    assert!(!footer(&composer).contains("external editor"));
}

#[test]
fn the_edit_replaces_the_draft_without_its_trailing_blanks() {
    let (mut composer, _events) = new_composer();
    composer.set_text_content("before".to_string());
    composer.apply_external_edit("after\r\nsecond line\n\n  \n".to_string());
    assert_eq!(composer.textarea.text(), "after\nsecond line");
    assert_eq!(composer.textarea.cursor(), composer.textarea.text().len());
    composer.apply_external_edit(String::new());
    assert!(composer.is_empty());
}

#[test]
fn the_editor_gets_the_pastes_and_an_unchanged_one_is_its_placeholder_again() {
    let (mut composer, _events) = new_composer();
    let (first, second) = (large('a'), large('b'));
    composer.handle_paste(first.clone());
    composer.textarea.insert_str(" and ");
    composer.handle_paste(second.clone());
    let placeholder = format!("[Pasted Content {} chars]", first.chars().count());
    let placeholder_2 = format!("{placeholder} #2");
    assert_eq!(
        composer.textarea.text(),
        format!("{placeholder} and {placeholder_2}")
    );
    // The seed: both pastes written out.
    assert_eq!(
        composer.current_text_with_pending(),
        format!("{first} and {second}")
    );

    // The first paste back as it was, the second edited.
    let edited = &second[1..];
    composer.apply_external_edit(format!("intro {first} and {edited}!\n"));
    assert_eq!(
        composer.textarea.text(),
        format!("intro {placeholder} and {edited}!")
    );
    assert_eq!(
        composer.textarea.element_payloads(),
        vec![placeholder.clone()]
    );
    assert_eq!(
        composer.current_text_with_pending(),
        format!("intro {first} and {edited}!")
    );

    // Its placeholder deleted in the editor, the paste is gone.
    composer.apply_external_edit("nothing pasted".to_string());
    assert_eq!(composer.textarea.text(), "nothing pasted");
    assert!(composer.pending_pastes.is_empty());
}

/// A pasted log ends in a line break, and the editor adds one after it.
#[test]
fn a_paste_ending_the_draft_with_a_line_break_comes_back_as_its_placeholder() {
    let (mut composer, _events) = new_composer();
    composer.textarea.insert_str("see: ");
    let log = format!("{}\n", large('l'));
    composer.handle_paste(log.clone());
    let placeholder = format!("[Pasted Content {} chars]", log.chars().count());
    let seed = composer.current_text_with_pending();
    assert_eq!(seed, format!("see: {log}"));

    composer.apply_external_edit(format!("{seed}\n"));
    assert_eq!(composer.textarea.text(), format!("see: {placeholder}"));
    assert_eq!(composer.current_text_with_pending(), seed);
}
