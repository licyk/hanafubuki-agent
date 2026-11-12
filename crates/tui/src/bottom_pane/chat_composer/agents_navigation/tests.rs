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
