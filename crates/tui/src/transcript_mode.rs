// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Who owns the transcript for the life of the process, chosen at launch:
//! the interface, drawing it on the alternate screen (owned), or the
//! terminal, whose scrollback the finished cells are written into.
//!
//! Codex's `auto` keeps the alternate screen off only for Terminal.app
//! over SSH, which it learns from a terminal identity probe; here `auto`
//! keeps it off over any SSH session.

use hanafubuki_agent::{AgentAlternateScreen, AgentTuiPreferences};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TranscriptMode {
    Owned,
    Terminal,
}

impl TranscriptMode {
    /// The mode `preferences` give, with `--no-alt-screen` (`no_alt_screen`)
    /// and whether this is an SSH session (`ssh`).
    pub(crate) fn resolve(
        preferences: &AgentTuiPreferences,
        no_alt_screen: bool,
        ssh: bool,
    ) -> Self {
        if preferences.fullscreen_transcript
            && alternate_screen_allowed(no_alt_screen, preferences.alternate_screen, ssh)
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

fn alternate_screen_allowed(no_alt_screen: bool, setting: AgentAlternateScreen, ssh: bool) -> bool {
    if no_alt_screen {
        return false;
    }
    match setting {
        AgentAlternateScreen::Always => true,
        AgentAlternateScreen::Never => false,
        AgentAlternateScreen::Auto => !ssh,
    }
}

/// Whether the process runs in an SSH session.
pub(crate) fn is_ssh_session() -> bool {
    std::env::var_os("SSH_TTY").is_some() || std::env::var_os("SSH_CONNECTION").is_some()
}

#[cfg(test)]
mod tests;
