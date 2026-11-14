// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Codex's identity cases, over an injected environment: never the
//! process's, which differs between CI runners.

use std::collections::HashMap;

use super::*;

fn detect_in(vars: &[(&str, &str)]) -> TerminalInfo {
    let vars: HashMap<String, String> = vars
        .iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect();
    detect(|name| vars.get(name).cloned())
}

#[test]
fn each_terminal_is_told_by_its_variables() {
    let cases: &[(&[(&str, &str)], TerminalName)] = &[
        (&[("TERM_PROGRAM", "iTerm.app")], TerminalName::Iterm2),
        (
            &[("TERM_PROGRAM", "Apple_Terminal")],
            TerminalName::AppleTerminal,
        ),
        (&[("TERM_PROGRAM", "ghostty")], TerminalName::Ghostty),
        (&[("TERM_PROGRAM", "rex")], TerminalName::Rex),
        (&[("TERM_PROGRAM", "vscode")], TerminalName::VsCode),
        (
            &[("TERM_PROGRAM", "WarpTerminal")],
            TerminalName::WarpTerminal,
        ),
        (&[("TERM_PROGRAM", "WezTerm")], TerminalName::WezTerm),
        (
            &[("TERM_PROGRAM", "Windows_Terminal")],
            TerminalName::WindowsTerminal,
        ),
        (&[("TERM_PROGRAM", "something-else")], TerminalName::Unknown),
        (&[("GHOSTTY_RESOURCES_DIR", "/g")], TerminalName::Ghostty),
        (&[("WEZTERM_VERSION", "")], TerminalName::WezTerm),
        (&[("ITERM_SESSION_ID", "w0")], TerminalName::Iterm2),
        (&[("ITERM_PROFILE_NAME", "Default")], TerminalName::Iterm2),
        (&[("KITTY_WINDOW_ID", "1")], TerminalName::Kitty),
        (&[("TERM", "xterm-kitty")], TerminalName::Kitty),
        (&[("ALACRITTY_SOCKET", "/a")], TerminalName::Alacritty),
        (&[("TERM", "alacritty")], TerminalName::Alacritty),
        (&[("KONSOLE_VERSION", "230800")], TerminalName::Konsole),
        (
            &[("GNOME_TERMINAL_SCREEN", "/org")],
            TerminalName::GnomeTerminal,
        ),
        (&[("VTE_VERSION", "7600")], TerminalName::Vte),
        (&[("WT_SESSION", "1")], TerminalName::WindowsTerminal),
        (&[("TERM", "dumb")], TerminalName::Dumb),
        (&[("TERM", "xterm-ghostty")], TerminalName::Ghostty),
        (&[("TERM", "wezterm")], TerminalName::WezTerm),
        (&[("TERM", "xterm-256color")], TerminalName::Unknown),
        (&[], TerminalName::Unknown),
    ];
    for (vars, expected) in cases {
        assert_eq!(detect_in(vars).name, *expected, "{vars:?}");
    }
}

#[test]
fn term_program_masks_later_probes_but_tmux_does_not() {
    // VS Code's terminal on Windows Terminal is VS Code's.
    let info = detect_in(&[("TERM_PROGRAM", "vscode"), ("WT_SESSION", "1")]);
    assert_eq!(info.name, TerminalName::VsCode);
    // tmux names itself in TERM_PROGRAM; the terminal under it is found.
    let info = detect_in(&[
        ("TERM_PROGRAM", "tmux"),
        ("TMUX", "/tmp/tmux-1/default,1,0"),
        ("WT_SESSION", "1"),
    ]);
    assert_eq!(info.name, TerminalName::WindowsTerminal);
    assert_eq!(info.multiplexer, Some(Multiplexer::Tmux));
}

#[test]
fn multiplexers_are_told_by_their_variables() {
    assert_eq!(
        detect_in(&[("TMUX_PANE", "%1")]).multiplexer,
        Some(Multiplexer::Tmux)
    );
    for name in ["ZELLIJ", "ZELLIJ_SESSION_NAME", "ZELLIJ_VERSION"] {
        assert_eq!(
            detect_in(&[(name, "x")]).multiplexer,
            Some(Multiplexer::Zellij)
        );
    }
    // A blank value is no multiplexer.
    assert_eq!(detect_in(&[("TMUX", " ")]).multiplexer, None);
    assert_eq!(
        detect_in(&[("TERM", "screen-256color")]).term.as_deref(),
        Some("screen-256color")
    );
}
