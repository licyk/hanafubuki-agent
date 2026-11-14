// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The row cap by terminal (Codex's `resize_reflow_cap.rs` tests, over the
//! environment rather than its detection crate) and the quiet period.

use std::collections::HashMap;

use super::*;

fn cap(vars: &[(&str, &str)]) -> usize {
    let vars: HashMap<String, String> = vars
        .iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect();
    row_cap_for(&crate::terminal_detection::detect(|name| {
        vars.get(name).cloned()
    }))
}

#[test]
fn the_row_cap_follows_the_terminal() {
    let cases: [(&[(&str, &str)], usize); 9] = [
        (&[("TERM_PROGRAM", "vscode")], DEFAULT_ROW_CAP),
        (&[("WT_SESSION", "1")], WINDOWS_TERMINAL_ROW_CAP),
        (&[("TERM_PROGRAM", "WezTerm")], WEZTERM_ROW_CAP),
        (&[("WEZTERM_VERSION", "2024.2")], WEZTERM_ROW_CAP),
        (&[("ALACRITTY_SOCKET", "/tmp/a")], ALACRITTY_ROW_CAP),
        (&[("TERM", "alacritty")], ALACRITTY_ROW_CAP),
        (&[("GHOSTTY_RESOURCES_DIR", "/g")], DEFAULT_ROW_CAP),
        (&[("TERM", "xterm-256color")], DEFAULT_ROW_CAP),
        (&[], DEFAULT_ROW_CAP),
    ];
    for (vars, expected) in cases {
        assert_eq!(cap(vars), expected, "{vars:?}");
    }
}

#[test]
fn term_program_masks_later_probes_but_tmux_does_not() {
    // Codex reads `TERM_PROGRAM` first: VS Code's on Windows Terminal is
    // VS Code's.
    assert_eq!(
        cap(&[("TERM_PROGRAM", "vscode"), ("WT_SESSION", "1")]),
        DEFAULT_ROW_CAP
    );
    assert_eq!(
        cap(&[("TERM_PROGRAM", "tmux"), ("WT_SESSION", "1")]),
        WINDOWS_TERMINAL_ROW_CAP
    );
    // An unknown terminal under tmux keeps the default.
    assert_eq!(
        cap(&[("TERM_PROGRAM", "tmux"), ("TERM", "screen-256color")]),
        DEFAULT_ROW_CAP
    );
}

#[test]
fn a_later_resize_pushes_the_rewrite_out() {
    let mut reflow = TranscriptReflow::default();
    let start = Instant::now();
    let first = reflow.schedule_debounced(start);
    let later = reflow.schedule_debounced(start + Duration::from_millis(50));
    assert!(later > first);
    assert_eq!(reflow.pending_until, Some(later));
    reflow.schedule_immediate(start);
    assert_eq!(reflow.pending_until, Some(start));
}

#[test]
fn a_stream_end_owes_one_rewrite_per_resize() {
    let mut reflow = TranscriptReflow::default();
    assert!(!reflow.take_stream_repair());
    reflow.stream_repair = true;
    assert!(reflow.take_stream_repair());
    assert!(!reflow.take_stream_repair());
}
