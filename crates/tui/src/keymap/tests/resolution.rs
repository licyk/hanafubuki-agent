// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use std::collections::BTreeMap;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::keymap::{conflicts, Action, Binding, Context, Keymap, ACTIONS};

fn config(entries: &[(&str, &str, &[&str])]) -> BTreeMap<String, BTreeMap<String, Vec<String>>> {
    let mut config: BTreeMap<String, BTreeMap<String, Vec<String>>> = BTreeMap::new();
    for (context, action, keys) in entries {
        config.entry(context.to_string()).or_default().insert(
            action.to_string(),
            keys.iter().map(|key| key.to_string()).collect(),
        );
    }
    config
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

#[test]
fn the_catalogue_is_in_action_order_and_its_defaults_parse_without_conflict() {
    for (index, spec) in ACTIONS.iter().enumerate() {
        assert_eq!(spec.action as usize, index, "{}", spec.name);
        for key in spec.defaults {
            assert!(Binding::parse(key).is_ok(), "{}: {key}", spec.name);
        }
        assert_eq!(Action::named(spec.context, spec.name), Some(spec.action));
    }
    let (keymap, warnings) = Keymap::resolve(&BTreeMap::new());
    assert!(warnings.is_empty(), "{warnings:#?}");
    let all: Vec<Vec<Binding>> = ACTIONS
        .iter()
        .map(|spec| keymap.bindings(spec.action).to_vec())
        .collect();
    assert_eq!(conflicts(&all), Vec::new());
    // Select mode has no key of its own: the leader chord.
    assert_eq!(keymap.label(Action::SelectTranscript), "leader [");
    assert!(keymap.matches(Action::OpenTranscript, &ctrl('t')));
}

#[test]
fn the_shortcut_actions_have_their_defaults_and_no_voice_key() {
    let keymap = Keymap::default();
    for (action, path, keys) in [
        (Action::ShowShortcuts, "composer.show_shortcuts", "?"),
        (Action::CycleMode, "chat.cycle_mode", "backtab"),
        (Action::ReasoningDown, "chat.reasoning_down", "alt-,"),
        (Action::ReasoningUp, "chat.reasoning_up", "alt-."),
        (Action::PreviousAgent, "composer.previous_agent", "alt-left"),
        (Action::NextAgent, "composer.next_agent", "alt-right"),
    ] {
        assert_eq!(action.path(), path);
        assert_eq!(keymap.label(action), keys, "{path}");
    }
    let only_empty: Vec<&str> = ACTIONS
        .iter()
        .filter(|spec| spec.empty_composer_only)
        .map(|spec| spec.name)
        .collect();
    assert_eq!(
        only_empty,
        ["show_shortcuts", "previous_agent", "next_agent"]
    );
    // Alt+Left and Alt+Right are word motion as well, on a draft.
    let alt_left = KeyEvent::new(KeyCode::Left, KeyModifiers::ALT);
    assert!(keymap.matches(Action::PreviousAgent, &alt_left));
    assert!(keymap.matches(Action::MoveWordLeft, &alt_left));
    // Codex's voice key is not ported.
    for spec in &ACTIONS {
        assert!(!spec.name.contains("voice"), "{}", spec.name);
        assert!(!spec.defaults.contains(&"f8"), "{}", spec.name);
    }
}

#[test]
fn only_an_action_read_on_an_empty_composer_takes_a_text_key() {
    let (keymap, warnings) = Keymap::resolve(&config(&[
        ("composer", "show_shortcuts", &["h"]),
        ("composer", "next_agent", &["]"]),
    ]));
    assert!(warnings.is_empty(), "{warnings:#?}");
    assert_eq!(keymap.label(Action::ShowShortcuts), "h");
    assert_eq!(keymap.label(Action::NextAgent), "]");
    let (keymap, warnings) = Keymap::resolve(&config(&[
        ("composer", "show_shortcuts", &["? x"]),
        ("chat", "cycle_mode", &["m"]),
    ]));
    let joined = warnings.join("\n");
    assert!(
        joined.contains("tui.keymap.composer.show_shortcuts: `? x` types text"),
        "{joined}"
    );
    assert!(
        joined.contains("tui.keymap.chat.cycle_mode: `m` types text"),
        "{joined}"
    );
    assert_eq!(keymap.label(Action::ShowShortcuts), "?");
    assert_eq!(keymap.label(Action::CycleMode), "backtab");
}

#[test]
fn a_context_entry_comes_before_the_global_one_and_both_before_the_defaults() {
    let (keymap, warnings) = Keymap::resolve(&config(&[
        ("global", "page_up", &["ctrl-up"]),
        ("pager", "page_up", &["f9"]),
        ("composer", "submit", &["ctrl-s"]),
    ]));
    assert!(warnings.is_empty(), "{warnings:#?}");
    assert_eq!(keymap.label(Action::PageUp), "f9");
    // The transcript's page_up has no entry of its own: the global one.
    assert_eq!(keymap.label(Action::TranscriptPageUp), "ctrl-up");
    assert_eq!(keymap.label(Action::Submit), "ctrl-s");
    assert!(keymap.is_configured(Action::Submit));
    assert!(!keymap.is_configured(Action::Yank));
    // An empty list unbinds.
    let (keymap, _) = Keymap::resolve(&config(&[("chat", "toggle_thinking", &[])]));
    assert_eq!(keymap.label(Action::ToggleThinking), "unbound");
    assert!(!keymap.matches(Action::ToggleThinking, &ctrl('o')));
}

#[test]
fn what_cannot_be_used_is_left_out_with_a_warning() {
    let (keymap, warnings) = Keymap::resolve(&config(&[
        ("nowhere", "submit", &["ctrl-s"]),
        ("chat", "nothing", &["ctrl-s"]),
        ("global", "open_transcript", &["ctrl-nothing"]),
        ("composer", "submit", &["x"]),
        ("global", "leader", &["leader x"]),
        ("editor", "yank", &["ctrl-c"]),
    ]));
    let joined = warnings.join("\n");
    for wanted in [
        "tui.keymap.nowhere: no such context",
        "tui.keymap.chat.nothing: no such action",
        "tui.keymap.global.open_transcript: `ctrl-nothing` is not a key",
        "tui.keymap.composer.submit: `x` types text",
        "tui.keymap.global.leader: the leader is one key",
        "tui.keymap.editor.yank: `ctrl-c` is kept for interrupting or quitting",
    ] {
        assert!(joined.contains(wanted), "{wanted}:\n{joined}");
    }
    // Each keeps its defaults.
    assert_eq!(keymap.label(Action::OpenTranscript), "ctrl-t");
    assert_eq!(keymap.label(Action::Submit), "enter");
    assert_eq!(keymap.label(Action::Yank), "ctrl-y");
    // A pager key may type text: no composer beside it.
    let (keymap, warnings) = Keymap::resolve(&config(&[("pager", "close", &["x"])]));
    assert!(warnings.is_empty(), "{warnings:#?}");
    assert_eq!(keymap.label(Action::Close), "x");
}

#[test]
fn a_conflict_keeps_the_defaults_of_the_action_that_caused_it() {
    let (keymap, warnings) = Keymap::resolve(&config(&[("chat", "toggle_thinking", &["ctrl-t"])]));
    assert_eq!(warnings.len(), 1, "{warnings:#?}");
    assert!(
        warnings[0]
            .contains("`ctrl-t` is bound to both global.open_transcript and chat.toggle_thinking"),
        "{warnings:#?}"
    );
    assert_eq!(keymap.label(Action::ToggleThinking), "ctrl-o");
    // Swapping two keys is no conflict once both are moved.
    let (keymap, warnings) = Keymap::resolve(&config(&[
        ("chat", "toggle_thinking", &["ctrl-t"]),
        ("global", "open_transcript", &["ctrl-o"]),
    ]));
    assert!(warnings.is_empty(), "{warnings:#?}");
    assert!(keymap.matches(Action::ToggleThinking, &ctrl('t')));
    assert!(keymap.matches(Action::OpenTranscript, &ctrl('o')));
    // Keys on different paths do not conflict: the pager's Ctrl-U and the
    // editor's.
    assert!(keymap.matches(Action::HalfPageUp, &ctrl('u')));
    assert!(keymap.matches(Action::KillLineStart, &ctrl('u')));
}

#[test]
fn a_key_that_starts_a_chord_conflicts_with_it() {
    let (keymap, warnings) = Keymap::resolve(&config(&[
        ("chat", "open_presets", &["ctrl-g p"]),
        ("global", "focus_activity", &["ctrl-g"]),
    ]));
    assert_eq!(warnings.len(), 1, "{warnings:#?}");
    assert!(
        warnings[0].starts_with("tui.keymap.chat.open_presets: `ctrl-g` (global.focus_activity) starts chat.open_presets's chord `ctrl-g p`"),
        "{warnings:#?}"
    );
    assert_eq!(keymap.label(Action::OpenPresets), "leader p");
    assert_eq!(keymap.label(Action::FocusActivity), "ctrl-g");
}

#[test]
fn the_default_leader_yields_to_a_key_bound_to_it() {
    let (keymap, warnings) =
        Keymap::resolve(&config(&[("global", "open_transcript", &["ctrl-x"])]));
    assert!(warnings.is_empty(), "{warnings:#?}");
    assert_eq!(keymap.label(Action::Leader), "unbound");
    assert_eq!(keymap.chords_in(&Context::ALL).count(), 0);
    // A leader set apart moves every leader chord.
    let (keymap, warnings) = Keymap::resolve(&config(&[("global", "leader", &["ctrl-g"])]));
    assert!(warnings.is_empty(), "{warnings:#?}");
    let chords: Vec<String> = keymap
        .chords_in(&Context::ALL)
        .map(|chord| {
            format!(
                "{} {} {:?}",
                chord.first.spec(),
                chord.second.spec(),
                chord.action
            )
        })
        .collect();
    assert_eq!(
        chords,
        [
            "ctrl-g [ SelectTranscript",
            "ctrl-g left FocusPaneLeft",
            "ctrl-g right FocusPaneRight",
            "ctrl-g tab ToggleSideConversation",
            "ctrl-g < MoveDividerLeft",
            "ctrl-g > MoveDividerRight",
            "ctrl-g = EqualizePaneWidths",
            "ctrl-g z TogglePaneZoom",
            "ctrl-g x HidePane",
            "ctrl-g p OpenPresets"
        ]
    );
}
