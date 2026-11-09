// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! The commands a message starting with a slash runs, trimmed to the ones
//! the Agent has; Codex derives the list with `strum`, here it is written
//! out.

/// Commands that can be invoked by starting a message with a leading slash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SlashCommand {
    // DO NOT ALPHA-SORT! Enum order is presentation order in the popup, so
    // more frequently used commands should be listed first.
    Model,
    Mode,
    Skills,
    New,
    Resume,
    Compact,
    Diff,
    Mention,
    Status,
    Mcp,
    Quit,
    Exit,
}

impl SlashCommand {
    /// Every command, in presentation order.
    pub(crate) const ALL: [SlashCommand; 12] = [
        SlashCommand::Model,
        SlashCommand::Mode,
        SlashCommand::Skills,
        SlashCommand::New,
        SlashCommand::Resume,
        SlashCommand::Compact,
        SlashCommand::Diff,
        SlashCommand::Mention,
        SlashCommand::Status,
        SlashCommand::Mcp,
        SlashCommand::Quit,
        SlashCommand::Exit,
    ];

    /// User-visible description shown in the popup.
    pub(crate) fn description(self) -> &'static str {
        match self {
            SlashCommand::New => "start a new chat during a conversation",
            SlashCommand::Compact => "summarize conversation to prevent hitting the context limit",
            SlashCommand::Resume => "resume a saved chat",
            SlashCommand::Quit | SlashCommand::Exit => "exit the Agent",
            SlashCommand::Diff => "show git diff (including untracked files)",
            SlashCommand::Mention => "mention a file",
            SlashCommand::Skills => "list the Skills the Agent can use",
            SlashCommand::Status => "show current session configuration and token usage",
            SlashCommand::Model => "choose what model to use",
            SlashCommand::Mode => "switch between default and plan mode",
            SlashCommand::Mcp => "list configured MCP servers and their tools",
        }
    }

    /// Command string without the leading '/'.
    pub(crate) fn command(self) -> &'static str {
        match self {
            SlashCommand::Model => "model",
            SlashCommand::Mode => "mode",
            SlashCommand::Skills => "skills",
            SlashCommand::New => "new",
            SlashCommand::Resume => "resume",
            SlashCommand::Compact => "compact",
            SlashCommand::Diff => "diff",
            SlashCommand::Mention => "mention",
            SlashCommand::Status => "status",
            SlashCommand::Mcp => "mcp",
            SlashCommand::Quit => "quit",
            SlashCommand::Exit => "exit",
        }
    }

    /// Whether this command can be run while a task is in progress.
    pub(crate) fn available_during_task(self) -> bool {
        match self {
            SlashCommand::New
            | SlashCommand::Resume
            | SlashCommand::Compact
            | SlashCommand::Model
            | SlashCommand::Mode => false,
            SlashCommand::Diff
            | SlashCommand::Mention
            | SlashCommand::Skills
            | SlashCommand::Status
            | SlashCommand::Mcp
            | SlashCommand::Quit
            | SlashCommand::Exit => true,
        }
    }

    /// The command named exactly `name`.
    pub(crate) fn find(name: &str) -> Option<SlashCommand> {
        Self::ALL
            .into_iter()
            .find(|command| command.command() == name)
    }

    /// Whether any command fuzzily matches `name` (Codex's
    /// `codex-common` `fuzzy_match`: a case-insensitive subsequence), so
    /// typing it keeps the popup open.
    pub(crate) fn has_prefix(name: &str) -> bool {
        let name = name.to_lowercase();
        Self::ALL.into_iter().any(|command| {
            let mut characters = command.command().chars();
            name.chars().all(|wanted| characters.any(|c| c == wanted))
        })
    }
}

#[cfg(test)]
mod tests;
