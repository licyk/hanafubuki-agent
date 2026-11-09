// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! What entering, configuring and leaving the alternate screen write.

use super::AlternateScreen;

const POINTER_CAPTURE: &str = "\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h";
const ALTERNATE_SCROLL_ON: &str = "\x1b[?1007h";
const ALTERNATE_SCROLL_OFF: &str = "\x1b[?1007l";
const ENTER: &str = "\x1b[?1049h";
const LEAVE: &str = "\x1b[?1049l";
const MOUSE_OFF: &str = "\x1b[?1006l";

fn written(write: impl FnOnce(&mut Vec<u8>)) -> String {
    let mut bytes = Vec::new();
    write(&mut bytes);
    String::from_utf8(bytes).unwrap()
}

#[test]
fn entering_with_the_pointer_captures_it_instead_of_the_wheel() {
    let screen = AlternateScreen::new();
    let out = written(|w| screen.enter(w, true).unwrap());
    assert!(out.starts_with(ENTER));
    assert!(out.ends_with(&format!("{ALTERNATE_SCROLL_OFF}{POINTER_CAPTURE}")));
    assert!(screen.is_active());
    assert!(screen.is_capturing_mouse());
}

#[test]
fn entering_without_the_pointer_leaves_the_wheel_to_the_terminal() {
    let screen = AlternateScreen::new();
    let out = written(|w| screen.enter(w, false).unwrap());
    assert!(out.ends_with(ALTERNATE_SCROLL_ON));
    assert!(!out.contains(POINTER_CAPTURE));
    assert!(!screen.is_capturing_mouse());
}

#[test]
fn leaving_stops_the_pointer_and_pops_the_keyboard_flags_before_the_screen() {
    let screen = AlternateScreen::new();
    written(|w| screen.enter(w, true).unwrap());
    let out = written(|w| screen.leave(w).unwrap());
    let mouse = out.find(MOUSE_OFF).expect("pointer reporting stops");
    let pop = out.find("\x1b[<1u").expect("the keyboard flags are popped");
    let leave = out.find(LEAVE).expect("the screen is left");
    assert!(mouse < leave && pop < leave);
    assert!(!screen.is_active());
    assert!(!screen.is_capturing_mouse());
}

#[test]
fn turning_the_capture_off_restores_alternate_scroll() {
    let screen = AlternateScreen::new();
    written(|w| screen.enter(w, true).unwrap());
    let out = written(|w| screen.configure_input(w, false).unwrap());
    assert!(out.contains(MOUSE_OFF));
    assert!(out.ends_with(ALTERNATE_SCROLL_ON));
}

#[test]
fn a_restore_off_the_alternate_screen_only_stops_the_pointer() {
    let screen = AlternateScreen::new();
    assert_eq!(written(|w| screen.restore(w).unwrap()), "");
    written(|w| screen.enter(w, true).unwrap());
    written(|w| screen.leave(w).unwrap());
    screen
        .mouse_active
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let out = written(|w| screen.restore(w).unwrap());
    assert!(out.contains(MOUSE_OFF));
    assert!(!out.contains(LEAVE));
}
