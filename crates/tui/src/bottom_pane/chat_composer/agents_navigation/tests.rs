// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use std::collections::BTreeMap;
use std::sync::mpsc::{channel, Receiver};

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;

use super::super::ChatComposer;
use crate::app::WorkspaceEvent;
use crate::app_event::{AppEvent, AppEventSender};
use crate::keymap::{self, Keymap};
use crate::render::renderable::Renderable;

fn new_composer() -> (ChatComposer, Receiver<AppEvent>) {
    let (tx, rx) = channel();
    let composer = ChatComposer::new(true, AppEventSender::new(tx), false, String::new());
    (composer, rx)
}

fn press(composer: &mut ChatComposer, code: KeyCode) {
    composer.handle_key_event(KeyEvent::new(code, KeyModifiers::NONE));
}

fn opened(events: &Receiver<AppEvent>) -> bool {
    events
        .try_iter()
        .any(|event| matches!(event, AppEvent::Workspace(WorkspaceEvent::OpenAgents)))
}

fn footer(composer: &ChatComposer) -> String {
    let area = Rect::new(0, 0, 60, 4);
    let mut buf = Buffer::empty(area);
    composer.render(area, &mut buf);
    (0..area.width)
        .map(|x| buf[(x, area.height - 1)].symbol().to_string())
        .collect()
}

fn install(context: &str, action: &str, keys: &[&str]) {
    let config = BTreeMap::from([(
        context.to_string(),
        BTreeMap::from([(
            action.to_string(),
            keys.iter().map(|key| key.to_string()).collect(),
        )]),
    )]);
    keymap::install(Keymap::resolve(&config).0);
}

#[test]
fn left_on_an_empty_composer_opens_the_command_center() {
    let (mut composer, events) = new_composer();
    assert!(footer(&composer).contains("← for agents · ? for shortcuts"));
    press(&mut composer, KeyCode::Left);
    assert!(opened(&events));
}

#[test]
fn left_moves_the_cursor_in_a_draft_or_a_popup() {
    let (mut composer, events) = new_composer();
    composer.set_text_content("draft".to_string());
    assert!(!footer(&composer).contains("for agents"));
    press(&mut composer, KeyCode::Left);
    assert!(!opened(&events));

    let (mut composer, events) = new_composer();
    composer.set_text_content("/".to_string());
    assert!(composer.popup_active());
    press(&mut composer, KeyCode::Left);
    assert!(!opened(&events));
}

#[test]
fn left_does_not_open_it_where_agents_does_not_run_or_without_focus() {
    let (mut composer, events) = new_composer();
    composer.set_command_scope(Some(&["copy", "side"]));
    assert!(!footer(&composer).contains("for agents"));
    press(&mut composer, KeyCode::Left);
    assert!(!opened(&events));

    let (mut composer, events) = new_composer();
    composer.disable_command("agents");
    press(&mut composer, KeyCode::Left);
    assert!(!opened(&events));

    let (mut composer, events) = new_composer();
    composer.has_focus = false;
    press(&mut composer, KeyCode::Left);
    assert!(!opened(&events));
}

fn alt(composer: &mut ChatComposer, code: KeyCode) {
    composer.handle_key_event(KeyEvent::new(code, KeyModifiers::ALT));
}

fn steps(events: &Receiver<AppEvent>) -> Vec<bool> {
    events
        .try_iter()
        .filter_map(|event| match event {
            AppEvent::Workspace(WorkspaceEvent::StepAgent { forward }) => Some(forward),
            _ => None,
        })
        .collect()
}

#[test]
fn alt_arrows_step_through_the_agents_on_an_empty_composer_only() {
    let (mut composer, events) = new_composer();
    alt(&mut composer, KeyCode::Right);
    alt(&mut composer, KeyCode::Left);
    assert_eq!(steps(&events), [true, false]);

    // With text they move by word.
    composer.set_text_content("one two".to_string());
    alt(&mut composer, KeyCode::Left);
    assert_eq!(composer.textarea.cursor(), "one ".len());
    alt(&mut composer, KeyCode::Right);
    assert_eq!(composer.textarea.cursor(), "one two".len());
    assert!(steps(&events).is_empty());

    // In a second conversation too, where `/agents` does not run.
    let (mut composer, events) = new_composer();
    composer.set_command_scope(Some(&["copy", "side"]));
    alt(&mut composer, KeyCode::Right);
    assert_eq!(steps(&events), [true]);

    let (mut composer, events) = new_composer();
    composer.has_focus = false;
    alt(&mut composer, KeyCode::Right);
    assert!(steps(&events).is_empty());
}

#[test]
fn the_shortcuts_key_toggles_on_an_empty_composer_and_types_in_text() {
    let (mut composer, _events) = new_composer();
    press(&mut composer, KeyCode::Char('?'));
    assert!(footer_rows(&composer).contains("Keyboard shortcuts"));
    press(&mut composer, KeyCode::Char('?'));
    assert!(!footer_rows(&composer).contains("Keyboard shortcuts"));
    // `?` in a draft is typed (`question_mark_only_toggles_on_first_char`).

    // Rebound to a letter, which then shows them on an empty composer.
    install("composer", "show_shortcuts", &["h"]);
    let (mut composer, _events) = new_composer();
    assert!(footer(&composer).contains("h for shortcuts"));
    press(&mut composer, KeyCode::Char('h'));
    let shown = footer_rows(&composer);
    keymap::install(Keymap::default());
    assert!(shown.contains("Keyboard shortcuts"), "{shown}");
}

/// On a short screen the input keeps its rows and the shortcuts are cut,
/// saying where the rest is.
#[test]
fn the_shortcuts_cut_on_a_short_screen_keep_the_input() {
    let (mut composer, _events) = new_composer();
    press(&mut composer, KeyCode::Char('?'));
    let area = Rect::new(0, 0, 50, 12);
    let mut buf = Buffer::empty(area);
    composer.render(area, &mut buf);
    let rows: Vec<String> = (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect();
    let shown = rows.join("\n");
    assert!(rows[1].contains('›'), "{shown}");
    assert!(rows[10].contains("… more: /keymap"), "{shown}");
    assert!(rows[11].contains("/keymap customize"), "{shown}");
}

/// Every row the composer draws at a height that fits the shortcuts.
fn footer_rows(composer: &ChatComposer) -> String {
    let area = Rect::new(0, 0, 140, composer.desired_height(140));
    let mut buf = Buffer::empty(area);
    composer.render(area, &mut buf);
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
fn left_rebound_keeps_its_new_meaning() {
    // The editor moves left with another key, and Left is another action's.
    install("editor", "move_left", &["ctrl-b"]);
    let (mut composer, events) = new_composer();
    assert!(!footer(&composer).contains("for agents"));
    press(&mut composer, KeyCode::Left);
    assert!(!opened(&events));

    keymap::install(Keymap::default());
    let (mut composer, events) = new_composer();
    press(&mut composer, KeyCode::Left);
    assert!(opened(&events));
}
