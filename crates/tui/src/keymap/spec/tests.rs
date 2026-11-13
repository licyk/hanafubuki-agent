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

/// Alt+, and Alt+Shift+, (Alt+<) as crossterm reports them: `ESC ,` and
/// `ESC <` without the enhanced protocol; `CSI 44;3u` and `CSI 44:60;4u`
/// (the shifted key reported) with it; `CSI 44;4u` from a terminal that
/// reports no shifted key.
#[test]
fn shift_on_unshifted_punctuation_is_another_key() {
    let less = KeyStroke::parse("alt-,").unwrap();
    let top = KeyStroke::parse("alt-<").unwrap();
    let top_shifted = KeyStroke::parse("alt-shift-,").unwrap();
    let alt_shift = KeyModifiers::ALT | KeyModifiers::SHIFT;
    for (event, wanted) in [
        (
            key(KeyCode::Char(','), KeyModifiers::ALT),
            [true, false, false],
        ),
        (
            key(KeyCode::Char('<'), KeyModifiers::ALT),
            [false, true, false],
        ),
        (key(KeyCode::Char('<'), alt_shift), [false, true, false]),
        (key(KeyCode::Char(','), alt_shift), [false, false, true]),
    ] {
        let got = [&less, &top, &top_shifted].map(|stroke| stroke.matches(&event));
        assert_eq!(got, wanted, "{event:?}");
    }
    // `?` is shifted on its own: with or without Shift.
    let help = KeyStroke::parse("?").unwrap();
    assert!(help.matches(&key(KeyCode::Char('?'), KeyModifiers::NONE)));
    assert!(help.matches(&key(KeyCode::Char('?'), KeyModifiers::SHIFT)));
    // Without Ctrl or Alt there is no `shift-/` to tell apart: a layout
    // that types `/` with Shift still matches `/`.
    let slash = KeyStroke::parse("/").unwrap();
    assert!(slash.matches(&key(KeyCode::Char('/'), KeyModifiers::SHIFT)));
}
