// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::{Binding, KeyStroke};

fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

#[test]
fn key_strings_parse_and_write_back_canonically() {
    for (text, canonical) in [
        ("ctrl-t", "ctrl-t"),
        ("CTRL-t", "ctrl-t"),
        ("ctrl-T", "ctrl-shift-t"),
        ("shift-enter", "shift-enter"),
        ("page-down", "page-down"),
        ("pgup", "page-up"),
        ("f3", "f3"),
        ("G", "shift-g"),
        ("alt-<", "alt-<"),
        ("ctrl-alt-h", "ctrl-alt-h"),
        ("space", "space"),
        ("ctrl-space", "ctrl-space"),
        ("minus", "minus"),
        ("shift-tab", "backtab"),
    ] {
        assert_eq!(KeyStroke::parse(text).unwrap().spec(), canonical, "{text}");
    }
    for bad in ["", "ctrl-", "ctrl-nothing", "f99", "hyper-a", "ab"] {
        assert!(KeyStroke::parse(bad).is_err(), "{bad}");
    }
}

#[test]
fn chords_and_leader_chords_parse() {
    assert_eq!(
        Binding::parse("ctrl-x ctrl-s").unwrap().spec(),
        "ctrl-x ctrl-s"
    );
    assert_eq!(Binding::parse("leader [").unwrap().spec(), "leader [");
    assert!(Binding::parse("a b c").is_err());
}

#[test]
fn a_key_matches_the_forms_terminals_report() {
    let ctrl_b = KeyStroke::parse("ctrl-b").unwrap();
    assert!(ctrl_b.matches(&key(KeyCode::Char('b'), KeyModifiers::CONTROL)));
    // A C0 control character for Ctrl-B.
    assert!(ctrl_b.matches(&key(KeyCode::Char('\u{0002}'), KeyModifiers::NONE)));
    let mut release = key(KeyCode::Char('b'), KeyModifiers::CONTROL);
    release.kind = KeyEventKind::Release;
    assert!(!ctrl_b.matches(&release));

    let shifted_g = KeyStroke::parse("shift-g").unwrap();
    assert!(shifted_g.matches(&key(KeyCode::Char('G'), KeyModifiers::SHIFT)));
    assert!(shifted_g.matches(&key(KeyCode::Char('G'), KeyModifiers::NONE)));
    assert!(!shifted_g.matches(&key(KeyCode::Char('g'), KeyModifiers::NONE)));

    // Shifted punctuation, with or without Shift.
    let top = KeyStroke::parse("alt-<").unwrap();
    assert!(top.matches(&key(KeyCode::Char('<'), KeyModifiers::ALT)));
    assert!(top.matches(&key(
        KeyCode::Char('<'),
        KeyModifiers::ALT | KeyModifiers::SHIFT
    )));
    // Space is no punctuation: Shift-Space is its own key.
    let space = KeyStroke::parse("space").unwrap();
    assert!(!space.matches(&key(KeyCode::Char(' '), KeyModifiers::SHIFT)));
}
