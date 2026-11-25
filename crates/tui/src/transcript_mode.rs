// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Who owns the transcript for the life of the process, chosen at launch:
//! the interface, drawing it on the alternate screen (owned), or the
//! terminal, whose scrollback the finished cells are written into.
//!
//! `auto` keeps the alternate screen off only for Terminal.app over SSH
//! (Codex's rule, from the same probe of the terminal's identity): its
//! alternate screen does not keep what is written into the scrollback on
//! leaving it. Every other terminal over SSH takes the alternate screen as
//! it does locally.

use hanafubuki_agent::{AgentAlternateScreen, AgentTuiPreferences};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TranscriptMode {
    Owned,
    Terminal,
}

impl TranscriptMode {
    /// The mode `preferences` give, with `--no-alt-screen` (`no_alt_screen`)
    /// and `terminal_app_over_ssh`, asked of the terminal
    /// ([`crate::terminal_detection`]) only under `auto`, where its answer
    /// decides: elsewhere the terminal is not asked, and one slow to answer
    /// does not delay the start.
    pub(crate) fn resolve(
        preferences: &AgentTuiPreferences,
        no_alt_screen: bool,
        terminal_app_over_ssh: impl FnOnce() -> bool,
    ) -> Self {
        if preferences.fullscreen_transcript
            && alternate_screen_allowed(
                no_alt_screen,
                preferences.alternate_screen,
                terminal_app_over_ssh,
            )
        {
            Self::Owned
        } else {
            Self::Terminal
        }
    }

    pub(crate) fn is_owned(self) -> bool {
        self == Self::Owned
    }
}

fn alternate_screen_allowed(
    no_alt_screen: bool,
    setting: AgentAlternateScreen,
    terminal_app_over_ssh: impl FnOnce() -> bool,
) -> bool {
    if no_alt_screen {
        return false;
    }
    match setting {
        AgentAlternateScreen::Always => true,
        AgentAlternateScreen::Never => false,
        AgentAlternateScreen::Auto => !terminal_app_over_ssh(),
    }
}

/// Whether the process runs in an SSH session.
pub(crate) fn is_ssh_session() -> bool {
    std::env::var_os("SSH_TTY").is_some() || std::env::var_os("SSH_CONNECTION").is_some()
}

#[cfg(test)]
mod tests;
