// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::keymap::{Action, ChordState, ChordStep, Context, Keymap};

const MAIN: [Context; 5] = [
    Context::Global,
    Context::Chat,
    Context::Composer,
    Context::Editor,
    Context::Transcript,
];

fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

#[test]
fn the_leader_then_a_key_is_its_action_s_token() {
    let keymap = Keymap::default();
    let mut state = ChordState::default();
    let now = Instant::now();
    let ctrl_x = key(KeyCode::Char('x'), KeyModifiers::CONTROL);
    assert_eq!(state.feed(&keymap, &ctrl_x, &MAIN, now), ChordStep::Wait);
    assert_eq!(state.pending().unwrap().spec(), "ctrl-x");
    // A release while it waits changes nothing.
    let mut release = ctrl_x;
    release.kind = KeyEventKind::Release;
    assert_eq!(state.feed(&keymap, &release, &MAIN, now), ChordStep::Wait);
    // The leader does not expire.
    let later = now + Duration::from_secs(5);
    let ChordStep::Complete(token) = state.feed(
        &keymap,
        &key(KeyCode::Char('['), KeyModifiers::NONE),
        &MAIN,
        later,
    ) else {
        panic!("no chord");
    };
    assert!(keymap.matches(Action::SelectTranscript, &token));
    assert!(!keymap.matches(Action::OpenPresets, &token));
    assert_eq!(state.pending(), None);
}

#[test]
fn a_second_key_that_completes_nothing_cancels_and_a_plain_key_passes() {
    let keymap = Keymap::default();
    let mut state = ChordState::default();
    let now = Instant::now();
    let ctrl_x = key(KeyCode::Char('x'), KeyModifiers::CONTROL);
    let a = key(KeyCode::Char('a'), KeyModifiers::NONE);
    assert_eq!(state.feed(&keymap, &a, &MAIN, now), ChordStep::Pass);
    assert_eq!(state.feed(&keymap, &ctrl_x, &MAIN, now), ChordStep::Wait);
    assert_eq!(state.feed(&keymap, &a, &MAIN, now), ChordStep::Cancelled);
    assert_eq!(state.feed(&keymap, &a, &MAIN, now), ChordStep::Pass);
    // In a pager no leader chord is read: the leader is an ordinary key.
    assert_eq!(
        state.feed(&keymap, &ctrl_x, &[Context::Global, Context::Pager], now),
        ChordStep::Wait,
        "select_transcript is global"
    );
    state.clear();
    let pager_only = [Context::Pager];
    assert_eq!(
        state.feed(&keymap, &ctrl_x, &pager_only, now),
        ChordStep::Pass
    );
}

#[test]
fn an_ordinary_chord_expires_after_a_second() {
    let config = BTreeMap::from([(
        "chat".to_string(),
        BTreeMap::from([("toggle_thinking".to_string(), vec!["ctrl-q o".to_string()])]),
    )]);
    let (keymap, warnings) = Keymap::resolve(&config);
    assert!(warnings.is_empty(), "{warnings:#?}");
    let mut state = ChordState::default();
    let now = Instant::now();
    let ctrl_q = key(KeyCode::Char('q'), KeyModifiers::CONTROL);
    let o = key(KeyCode::Char('o'), KeyModifiers::NONE);
    assert_eq!(state.feed(&keymap, &ctrl_q, &MAIN, now), ChordStep::Wait);
    let ChordStep::Complete(token) = state.feed(&keymap, &o, &MAIN, now) else {
        panic!("no chord");
    };
    assert!(keymap.matches(Action::ToggleThinking, &token));
    assert_eq!(state.feed(&keymap, &ctrl_q, &MAIN, now), ChordStep::Wait);
    let late = now + Duration::from_millis(1500);
    assert_eq!(state.feed(&keymap, &o, &MAIN, late), ChordStep::Pass);
}
