// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use std::sync::mpsc::{channel, Receiver};

use ratatui::crossterm::event::{KeyCode, KeyModifiers};

use super::*;

fn picker(current: &str) -> (ThemePicker, Receiver<AppEvent>) {
    let (sender, receiver) = channel();
    set_syntax_theme(resolve_theme_by_name(current, None).unwrap());
    (
        ThemePicker::new(Some(current), AppEventSender::new(sender)),
        receiver,
    )
}

fn press(picker: &mut ThemePicker, code: KeyCode) {
    picker.handle_key_event(KeyEvent::new(code, KeyModifiers::NONE));
}

fn active_theme() -> Option<String> {
    current_syntax_theme().name
}

fn theme_named(name: &str) -> Option<String> {
    resolve_theme_by_name(name, None).unwrap().name
}

fn saved(receiver: &Receiver<AppEvent>) -> Vec<AppearanceEvent> {
    receiver
        .try_iter()
        .filter_map(|event| match event {
            AppEvent::Appearance(event) => Some(event),
            _ => None,
        })
        .collect()
}

#[test]
fn moving_previews_the_highlighted_theme_and_esc_puts_the_first_back() {
    let (mut picker, receiver) = picker("dracula");
    assert_eq!(active_theme(), theme_named("dracula"));
    // The themes are sorted: after `dracula` comes `github`.
    press(&mut picker, KeyCode::Down);
    assert_eq!(active_theme(), theme_named("github"));
    press(&mut picker, KeyCode::Down);
    assert_eq!(active_theme(), theme_named("gruvbox-dark"));
    press(&mut picker, KeyCode::Esc);
    assert!(picker.is_complete());
    assert_eq!(active_theme(), theme_named("dracula"));
    assert!(saved(&receiver).is_empty());
}

#[test]
fn ctrl_c_puts_the_first_theme_back_too() {
    let (mut picker, _receiver) = picker("nord");
    press(&mut picker, KeyCode::Up);
    assert_ne!(active_theme(), theme_named("nord"));
    assert_eq!(picker.on_ctrl_c(), CancellationEvent::Handled);
    assert!(picker.is_complete());
    assert_eq!(active_theme(), theme_named("nord"));
}

#[test]
fn enter_keeps_the_previewed_theme_and_saves_it() {
    let (mut picker, receiver) = picker("ansi");
    // Typing filters the list; the match is previewed.
    for character in "zenburn".chars() {
        press(&mut picker, KeyCode::Char(character));
    }
    assert_eq!(active_theme(), theme_named("zenburn"));
    press(&mut picker, KeyCode::Enter);
    assert!(picker.is_complete());
    assert_eq!(active_theme(), theme_named("zenburn"));
    assert_eq!(
        saved(&receiver),
        [AppearanceEvent::ThemeSaved("zenburn".to_string())]
    );
}

#[test]
fn the_sample_is_beside_the_list_when_wide_and_under_it_when_narrow() {
    let (picker, _receiver) = picker("base16-ocean-dark");
    for width in [60, 120] {
        let height = picker.desired_height(width);
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        picker.render(area, &mut buf);
        let rows: Vec<String> = (0..height)
            .map(|row| {
                (0..width)
                    .map(|column| buf[(column, row)].symbol().to_string())
                    .collect::<String>()
            })
            .collect();
        let sample = rows
            .iter()
            .position(|row| row.contains("fn greet(name: &str)"))
            .unwrap_or_else(|| panic!("no sample at {width}:\n{}", rows.join("\n")));
        let list = rows
            .iter()
            .position(|row| row.contains("base16-ocean-dark (current)"))
            .unwrap();
        let column = rows[sample].find("fn greet").unwrap();
        if width >= SIDE_BY_SIDE_MIN_WIDTH {
            assert!(column >= usize::from(width / 2), "{}", rows.join("\n"));
        } else {
            assert!(sample > list && column < 10, "{}", rows.join("\n"));
        }
    }
    let lines = preview_lines(80);
    assert_eq!(lines.len(), PREVIEW_ROWS.len());
    assert!(lines[0].spans.len() > 4, "highlighted: {:?}", lines[0]);
}
