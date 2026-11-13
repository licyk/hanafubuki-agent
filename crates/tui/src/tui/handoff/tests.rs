//! The terminal's modes, as an emulated terminal reads them, released and
//! taken back in each presentation.

use ratatui::crossterm::event::EnableBracketedPaste;
use ratatui::crossterm::execute;
use ratatui::layout::Rect;
use ratatui::widgets::{Paragraph, Widget};
use vt100::MouseProtocolMode;

use super::super::Tui;
use crate::custom_terminal::Terminal;
use crate::test_support::VT100Backend;

/// The modes a terminal has: what the interface sets at its start.
fn started() -> Tui<VT100Backend> {
    let mut terminal = Terminal::with_options(VT100Backend::new(30, 6)).unwrap();
    terminal.set_viewport_area(Rect::new(0, 5, 30, 1));
    execute!(terminal.backend_mut(), EnableBracketedPaste).unwrap();
    Tui::new(terminal)
}

fn screen(tui: &Tui<VT100Backend>) -> &vt100::Screen {
    tui.terminal.backend().vt100().screen()
}

fn draw(tui: &mut Tui<VT100Backend>, text: &'static str) {
    tui.draw(1, |frame| {
        let area = frame.area();
        Paragraph::new(text).render(area, frame.buffer_mut());
    })
    .unwrap();
}

fn assert_released(tui: &Tui<VT100Backend>) {
    let screen = screen(tui);
    assert!(!screen.alternate_screen());
    assert!(!screen.bracketed_paste());
    assert_eq!(screen.mouse_protocol_mode(), MouseProtocolMode::None);
    assert!(!screen.hide_cursor());
}

#[test]
fn scrollback_gives_the_terminal_back_as_the_shell_left_it_and_takes_it_again() {
    let mut tui = started();
    tui.set_inline_mouse_capture(true).unwrap();
    draw(&mut tui, "composer");
    assert_eq!(
        screen(&tui).mouse_protocol_mode(),
        MouseProtocolMode::PressRelease
    );

    let released = tui.release();
    assert_released(&tui);

    tui.reclaim(released).unwrap();
    let screen_now = screen(&tui);
    assert!(!screen_now.alternate_screen());
    assert!(screen_now.bracketed_paste());
    assert_eq!(
        screen_now.mouse_protocol_mode(),
        MouseProtocolMode::PressRelease
    );
    assert!(tui.captures_inline_mouse());
    // The viewport was cleared: the next frame draws it whole.
    assert_eq!(screen_now.contents_between(5, 0, 5, 30).trim(), "");
    draw(&mut tui, "composer");
    assert_eq!(
        screen(&tui).contents_between(5, 0, 5, 30).trim(),
        "composer"
    );
}

#[test]
fn fullscreen_leaves_the_alternate_screen_and_enters_it_again_with_its_pointer() {
    let mut tui = started();
    tui.enter_alt_screen(true).unwrap();
    assert!(screen(&tui).alternate_screen());
    assert_eq!(
        screen(&tui).mouse_protocol_mode(),
        MouseProtocolMode::AnyMotion
    );

    let released = tui.release();
    assert_released(&tui);

    tui.reclaim(released).unwrap();
    let screen_now = screen(&tui);
    assert!(screen_now.alternate_screen());
    assert!(screen_now.bracketed_paste());
    assert_eq!(
        screen_now.mouse_protocol_mode(),
        MouseProtocolMode::AnyMotion
    );
    // The whole screen is the viewport again.
    assert_eq!(tui.terminal.viewport_area, Rect::new(0, 0, 30, 6));
}

#[test]
fn an_alternate_screen_without_the_pointer_comes_back_without_it() {
    let mut tui = started();
    tui.enter_alt_screen(false).unwrap();

    let released = tui.release();
    assert_released(&tui);

    tui.reclaim(released).unwrap();
    assert!(screen(&tui).alternate_screen());
    assert_eq!(screen(&tui).mouse_protocol_mode(), MouseProtocolMode::None);
}
