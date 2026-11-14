//! What the update prompt leaves on the terminal for the interface that
//! starts after it. Drawn at the cursor below what the shell printed, at
//! the screen's height, it scrolls those lines up and puts none of its own
//! in the scrollback; once a choice is made its rows are clear, and the
//! interface's first frame, in either presentation, shows nothing of it.
//! (A real terminal keeps the lines a scrolling region at the top row
//! pushes off; vt100 drops them, so only what was in the scrollback
//! already is there to see.)

use std::io::Write;
use std::sync::mpsc::channel;

use super::{Harness, HEIGHT, WIDTH};
use crate::custom_terminal::Terminal;
use crate::test_support::VT100Backend;
use crate::transcript_mode::TranscriptMode;
use crate::tui::Tui;
use crate::update_prompt::{choose_on, TuiUpdateChoice, TuiUpdatePrompt};

/// The shell's first line, in the scrollback before the program starts.
const SHELL: &str = "$ shell line 1";
/// Text only the prompt draws.
const PROMPT_TEXTS: [&str; 3] = [
    "Update available",
    "Release notes",
    "Skip until next version",
];

/// The screen with what the shell printed, after the prompt was shown on
/// it and skipped.
fn after_the_prompt() -> VT100Backend {
    let mut backend = VT100Backend::with_scrollback(WIDTH, HEIGHT, 500);
    for line in 1..=HEIGHT + 5 {
        write!(backend, "$ shell line {line}\r\n").unwrap();
    }
    let mut tui = Tui::new(Terminal::with_options(backend).unwrap());
    // The input ends at once: Skip, after one frame.
    let (sender, events) = channel();
    drop(sender);
    let prompt = TuiUpdatePrompt {
        current: "0.2.0".to_string(),
        latest: "0.3.0".to_string(),
        release_notes: "https://github.com/licyk/hanafubuki-agent/releases/tag/v0.3.0".to_string(),
        update_now: "Update now (downloads 0.3.0 and replaces this executable)".to_string(),
    };
    let choice = choose_on(&mut tui, &events, prompt).unwrap();
    assert_eq!(choice, TuiUpdateChoice::Skip);
    let mut backend = std::mem::replace(tui.terminal.backend_mut(), VT100Backend::new(1, 1));
    assert!(backend.take_written().contains("Update available"));
    backend
}

/// The screen's rows with every row of the scrollback above them.
fn with_scrollback(backend: &mut VT100Backend) -> String {
    let parser = backend.vt100_mut();
    parser.screen_mut().set_scrollback(usize::MAX);
    let scrolled = parser.screen().contents();
    parser.screen_mut().set_scrollback(0);
    scrolled
}

fn assert_nothing_of_the_prompt(text: &str) {
    for prompt_text in PROMPT_TEXTS {
        assert!(!text.contains(prompt_text), "{prompt_text:?} in:\n{text}");
    }
}

#[test]
fn the_prompt_leaves_nothing_in_the_scrollback_or_the_screen() {
    let mut backend = after_the_prompt();
    let screen = backend.vt100().screen().contents();
    assert_eq!(screen.trim(), "", "{screen}");
    let scrolled = with_scrollback(&mut backend);
    assert!(scrolled.contains(SHELL), "{scrolled}");
    assert_nothing_of_the_prompt(&scrolled);
}

#[test]
fn the_interface_s_first_frame_after_the_prompt_is_clean_in_both_presentations() {
    for mode in [TranscriptMode::Terminal, TranscriptMode::Owned] {
        let mut harness = Harness::start_with(Vec::new(), true, mode);
        // The interface opens its own terminal where the prompt left the
        // cursor.
        harness.tui = Tui::new(Terminal::with_options(after_the_prompt()).unwrap());
        if mode.is_owned() {
            harness.app.enter_owned_screen(&mut harness.tui).unwrap();
        }
        harness.app.draw(&mut harness.tui).unwrap();

        let screen = harness.screen();
        assert!(screen.contains("fixture-model"), "{mode:?}:\n{screen}");
        assert_nothing_of_the_prompt(&screen);
        let scrolled = with_scrollback(harness.tui.terminal.backend_mut());
        assert_nothing_of_the_prompt(&scrolled);
        if !mode.is_owned() {
            assert!(scrolled.contains(SHELL), "{scrolled}");
        }
    }
}
