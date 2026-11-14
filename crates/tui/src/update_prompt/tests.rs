// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use std::sync::mpsc::channel;
use std::time::Duration;

use insta::assert_snapshot;
use ratatui::crossterm::event::{KeyEvent, KeyEventState};

use super::*;
use crate::custom_terminal::Terminal;
use crate::test_support::{osc8_links, VT100Backend};

const NOTES: &str = "https://github.com/licyk/hanafubuki-agent/releases/tag/v0.3.0";
const RELEASED: &str = "Update now (downloads 0.3.0 and replaces this executable)";
const CARGO: &str = "Update now (runs `cargo install --locked hanafubuki-agent-cli`)";
const HINT: &str = "enter continue · esc skip";

fn prompt(update_now: &str) -> TuiUpdatePrompt {
    TuiUpdatePrompt {
        current: "0.2.0".to_string(),
        latest: "0.3.0".to_string(),
        release_notes: NOTES.to_string(),
        update_now: update_now.to_string(),
    }
}

fn screen() -> UpdatePromptScreen {
    UpdatePromptScreen::new(prompt(RELEASED))
}

/// A terminal of `width` by `height` with the cursor at its top, as the
/// prompt finds a cleared one.
fn tui(width: u16, height: u16) -> Tui<VT100Backend> {
    Tui::new(Terminal::with_options(VT100Backend::new(width, height)).unwrap())
}

fn drawn(screen: &UpdatePromptScreen, width: u16, height: u16) -> Tui<VT100Backend> {
    let mut tui = tui(width, height);
    screen.draw(&mut tui).unwrap();
    tui
}

fn contents(tui: &Tui<VT100Backend>) -> String {
    tui.terminal.backend().vt100().screen().contents()
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(character: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(character), KeyModifiers::CONTROL)
}

/// The choice `keys` make, from a fresh screen; `None` while none is made.
fn chosen(keys: &[KeyEvent]) -> Option<TuiUpdateChoice> {
    let mut screen = screen();
    keys.iter().find_map(|key| screen.handle_key(*key))
}

#[test]
fn update_prompt_modal() {
    assert_snapshot!(
        "update_prompt_modal",
        drawn(&screen(), 80, 12).terminal.backend()
    );
}

#[test]
fn update_prompt_modal_cargo() {
    let screen = UpdatePromptScreen::new(prompt(CARGO));
    assert_snapshot!(
        "update_prompt_modal_cargo",
        drawn(&screen, 80, 12).terminal.backend()
    );
}

/// The plan's 40-column screen, the last option highlighted: one row too
/// short for everything, so the trailing blank row gives way.
#[test]
fn update_picker_selected_40x12() {
    let mut screen = screen();
    assert_eq!(screen.handle_key(key(KeyCode::Up)), None);
    let rendered = drawn(&screen, 40, 12).terminal.backend().to_string();
    assert!(
        rendered.contains("› 3. Skip until next version"),
        "{rendered}"
    );
    assert!(rendered.contains(HINT), "{rendered}");
    assert_snapshot!("update_picker_selected_40x12", rendered);
}

/// Whichever option is highlighted, it and the hint are on the screen from
/// 20 columns and 4 rows up (Codex's test: 28 by 12).
#[test]
fn the_highlighted_option_and_the_hint_stay_on_a_short_screen() {
    for highlighted in 0..CHOICES.len() {
        let mut screen = screen();
        screen.highlighted = highlighted;
        for width in [20, 28, 33, 40, 57, 80, 120] {
            for height in 4..=20 {
                let rendered = contents(&drawn(&screen, width, height));
                let marked = rendered
                    .lines()
                    .any(|line| line.starts_with(&format!("› {}.", highlighted + 1)));
                let hint = rendered.split_whitespace().collect::<Vec<_>>().join(" ");
                assert!(marked, "{highlighted} at {width}x{height}:\n{rendered}");
                assert!(
                    hint.contains(HINT),
                    "{highlighted} at {width}x{height}:\n{rendered}"
                );
            }
        }
    }
}

/// A window of any size, however small, is drawn without a panic.
#[test]
fn a_tiny_screen_is_drawn() {
    for highlighted in 0..CHOICES.len() {
        let mut screen = screen();
        screen.highlighted = highlighted;
        for width in 1..=24 {
            for height in 1..=14 {
                drawn(&screen, width, height);
            }
        }
    }
}

#[test]
fn the_release_notes_are_a_link_on_every_row_they_wrap_onto() {
    let mut tui = drawn(&screen(), 40, 12);
    let links = osc8_links(&tui.terminal.backend_mut().take_written());
    let text = links
        .iter()
        .map(|(destination, text)| {
            assert_eq!(destination, NOTES);
            text.as_str()
        })
        .collect::<String>();
    assert_eq!(text, NOTES);
    assert_eq!(links.len(), 2, "{links:?}");
}

#[test]
fn enter_chooses_the_highlighted_option_and_the_arrows_wrap() {
    assert_eq!(
        chosen(&[key(KeyCode::Enter)]),
        Some(TuiUpdateChoice::UpdateNow)
    );
    let down = key(KeyCode::Down);
    let up = key(KeyCode::Up);
    let enter = key(KeyCode::Enter);
    assert_eq!(chosen(&[down, enter]), Some(TuiUpdateChoice::Skip));
    assert_eq!(
        chosen(&[down, down, enter]),
        Some(TuiUpdateChoice::SkipVersion)
    );
    assert_eq!(
        chosen(&[down, down, down, enter]),
        Some(TuiUpdateChoice::UpdateNow)
    );
    assert_eq!(chosen(&[up, enter]), Some(TuiUpdateChoice::SkipVersion));
    assert_eq!(chosen(&[up, down, enter]), Some(TuiUpdateChoice::UpdateNow));
    let j = key(KeyCode::Char('j'));
    let k = key(KeyCode::Char('k'));
    assert_eq!(chosen(&[j, enter]), Some(TuiUpdateChoice::Skip));
    assert_eq!(chosen(&[k, enter]), Some(TuiUpdateChoice::SkipVersion));
    assert_eq!(chosen(&[j, j, k, enter]), Some(TuiUpdateChoice::Skip));
    assert_eq!(chosen(&[down, up, down]), None);
}

#[test]
fn a_digit_chooses_at_once_and_esc_ctrl_c_and_ctrl_d_skip() {
    let digits = [
        ('1', TuiUpdateChoice::UpdateNow),
        ('2', TuiUpdateChoice::Skip),
        ('3', TuiUpdateChoice::SkipVersion),
    ];
    for (digit, choice) in digits {
        // Whatever is highlighted.
        assert_eq!(
            chosen(&[key(KeyCode::Down), key(KeyCode::Char(digit))]),
            Some(choice)
        );
    }
    let down = key(KeyCode::Down);
    for skip in [key(KeyCode::Esc), ctrl('c'), ctrl('d')] {
        assert_eq!(chosen(&[down, down, skip]), Some(TuiUpdateChoice::Skip));
    }
    assert_eq!(chosen(&[key(KeyCode::Char('x')), key(KeyCode::Left)]), None);
}

#[test]
fn a_key_release_is_ignored() {
    let release = KeyEvent {
        code: KeyCode::Char('1'),
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Release,
        state: KeyEventState::NONE,
    };
    assert_eq!(
        chosen(&[
            release,
            KeyEvent {
                kind: KeyEventKind::Release,
                ..key(KeyCode::Enter)
            }
        ]),
        None
    );
    assert_eq!(
        chosen(&[release, key(KeyCode::Char('2'))]),
        Some(TuiUpdateChoice::Skip)
    );
}

/// The loop drops what was read before the screen was drawn: an Enter
/// typed while the program started does not update. The input then ends:
/// Skip.
#[test]
fn a_key_queued_before_the_screen_is_dropped() {
    let (sender, events) = channel();
    sender.send(AppEvent::Key(key(KeyCode::Enter))).unwrap();
    sender.send(AppEvent::Key(key(KeyCode::Char('1')))).unwrap();
    drop(sender);
    let mut tui = tui(80, 12);
    let choice = choose_on(&mut tui, &events, prompt(RELEASED)).unwrap();
    assert_eq!(choice, TuiUpdateChoice::Skip);
}

/// After the screen is drawn: each Redraw draws it again, a key that makes
/// no choice too, and the choice ends the loop with the viewport cleared.
#[test]
fn the_loop_redraws_and_clears_the_viewport_when_a_choice_is_made() {
    let (sender, events) = channel();
    let sending = std::thread::spawn(move || {
        // Long after the first frame, which drops what came before it.
        std::thread::sleep(Duration::from_millis(1000));
        for event in [
            AppEvent::Redraw,
            AppEvent::Redraw,
            AppEvent::Key(key(KeyCode::Down)),
            AppEvent::Key(key(KeyCode::Enter)),
        ] {
            sender.send(event).unwrap();
        }
    });
    let mut tui = tui(80, 12);
    let choice = choose_on(&mut tui, &events, prompt(RELEASED)).unwrap();
    sending.join().unwrap();
    assert_eq!(choice, TuiUpdateChoice::Skip);

    let written = tui.terminal.backend_mut().take_written();
    // The first frame, two redraws and the highlight's move.
    assert_eq!(written.matches("\x1b[?2026h").count(), 4, "{written:?}");
    assert!(written.contains("Update available"));
    assert_eq!(contents(&tui).trim(), "");
}
