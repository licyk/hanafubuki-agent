// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Which terminal the interface runs in, from its environment (Codex's
//! `codex-terminal-detection`, the identity only): a heuristic over the
//! variables terminals set, never a query to the terminal. The rewrite's
//! row cap and whether a web link's address can be hidden read it.

/// The terminal applications told apart.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalName {
    AppleTerminal,
    Ghostty,
    Rex,
    Iterm2,
    WarpTerminal,
    VsCode,
    WezTerm,
    Kitty,
    Alacritty,
    Konsole,
    GnomeTerminal,
    Vte,
    WindowsTerminal,
    Dumb,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Multiplexer {
    Tmux,
    Zellij,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TerminalInfo {
    pub(crate) name: TerminalName,
    pub(crate) multiplexer: Option<Multiplexer>,
    /// `TERM`, when set and not blank.
    pub(crate) term: Option<String>,
}

/// The terminal of this process's environment.
pub(crate) fn terminal_info() -> TerminalInfo {
    detect(|name| std::env::var(name).ok())
}

/// The terminal `var` describes; `var` returns a variable that is set,
/// even empty. Explicit identifiers come first, then terminal-specific
/// variables, then `TERM`: `TERM_PROGRAM` (but tmux's) masks the later
/// probes, as in Codex.
pub(crate) fn detect(var: impl Fn(&str) -> Option<String>) -> TerminalInfo {
    let has = |name: &str| var(name).is_some();
    let non_empty = |name: &str| var(name).filter(|value| !value.trim().is_empty());
    let term = non_empty("TERM");
    let info = |name| TerminalInfo {
        name,
        multiplexer: multiplexer(&non_empty),
        term: term.clone(),
    };

    if let Some(program) = non_empty("TERM_PROGRAM").filter(|p| !p.eq_ignore_ascii_case("tmux")) {
        return info(name_from_term_program(&program));
    }
    if non_empty("GHOSTTY_RESOURCES_DIR").is_some() {
        return info(TerminalName::Ghostty);
    }
    if has("WEZTERM_VERSION") {
        return info(TerminalName::WezTerm);
    }
    if has("ITERM_SESSION_ID") || has("ITERM_PROFILE") || has("ITERM_PROFILE_NAME") {
        return info(TerminalName::Iterm2);
    }
    let raw_term = var("TERM").unwrap_or_default();
    if has("KITTY_WINDOW_ID") || raw_term.contains("kitty") {
        return info(TerminalName::Kitty);
    }
    if has("ALACRITTY_SOCKET") || raw_term == "alacritty" {
        return info(TerminalName::Alacritty);
    }
    if has("KONSOLE_VERSION") {
        return info(TerminalName::Konsole);
    }
    if has("GNOME_TERMINAL_SCREEN") {
        return info(TerminalName::GnomeTerminal);
    }
    if has("VTE_VERSION") {
        return info(TerminalName::Vte);
    }
    if has("WT_SESSION") {
        return info(TerminalName::WindowsTerminal);
    }
    let name = match term.as_deref() {
        Some("dumb") => TerminalName::Dumb,
        Some("xterm-ghostty") => TerminalName::Ghostty,
        Some("xterm-rex") => TerminalName::Rex,
        Some("wezterm" | "wezterm-mux") => TerminalName::WezTerm,
        _ => TerminalName::Unknown,
    };
    info(name)
}

fn multiplexer(non_empty: &impl Fn(&str) -> Option<String>) -> Option<Multiplexer> {
    if non_empty("TMUX").is_some() || non_empty("TMUX_PANE").is_some() {
        return Some(Multiplexer::Tmux);
    }
    ["ZELLIJ", "ZELLIJ_SESSION_NAME", "ZELLIJ_VERSION"]
        .iter()
        .any(|name| non_empty(name).is_some())
        .then_some(Multiplexer::Zellij)
}

fn name_from_term_program(value: &str) -> TerminalName {
    let normalized: String = value
        .trim()
        .chars()
        .filter(|c| !matches!(c, ' ' | '-' | '_' | '.'))
        .map(|c| c.to_ascii_lowercase())
        .collect();

    match normalized.as_str() {
        "appleterminal" => TerminalName::AppleTerminal,
        "ghostty" => TerminalName::Ghostty,
        "rex" => TerminalName::Rex,
        "iterm" | "iterm2" | "itermapp" => TerminalName::Iterm2,
        "warp" | "warpterminal" => TerminalName::WarpTerminal,
        "vscode" => TerminalName::VsCode,
        "wezterm" => TerminalName::WezTerm,
        "kitty" => TerminalName::Kitty,
        "alacritty" => TerminalName::Alacritty,
        "konsole" => TerminalName::Konsole,
        "gnometerminal" => TerminalName::GnomeTerminal,
        "vte" => TerminalName::Vte,
        "windowsterminal" => TerminalName::WindowsTerminal,
        "dumb" => TerminalName::Dumb,
        _ => TerminalName::Unknown,
    }
}

#[cfg(test)]
mod tests;
