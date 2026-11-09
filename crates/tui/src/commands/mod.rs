// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! The slash commands, one table of them: each command's spec and the
//! module that runs it. The composer's popup, its reading of a typed
//! `/name args` and the chat's dispatch all come from the table.
//!
//! A command runs inside `catch_panic`, and so do the views it pushes
//! (`bottom_pane::module_view`): a command that panics is turned off for
//! the rest of the run, and the chat goes on.

mod exit;
mod mode;
mod model;
mod raw;
mod reports;
mod session;
mod tui;
mod workspace;

use std::fmt;

use hanafubuki_agent::AgentSessionMode;

use crate::app_event::{AgentCommand, AppEvent, AppEventSender, Op};
use crate::bottom_pane::{BottomPane, SelectionViewParams};
pub(crate) use mode::mode_name;
pub(crate) use model::{show_model_choices, MODEL};

/// Every command, in presentation order: the most used first.
pub(crate) static COMMANDS: [&CommandSpec; 13] = [
    &model::MODEL,
    &mode::MODE,
    &reports::SKILLS,
    &session::NEW,
    &session::RESUME,
    &session::COMPACT,
    &workspace::DIFF,
    &workspace::MENTION,
    &reports::STATUS,
    &reports::MCP,
    &raw::RAW,
    &tui::TUI,
    &exit::EXIT,
];

pub(crate) struct CommandSpec {
    /// What follows the slash.
    pub(crate) name: &'static str,
    /// Other names it runs under; the popup lists one only once it is typed.
    pub(crate) aliases: &'static [&'static str],
    pub(crate) description: &'static str,
    /// Whether the text after the name is the command's. For a command
    /// that takes none, `/name text` is sent as a message.
    pub(crate) inline_args: bool,
    /// Whether it runs while a turn does.
    pub(crate) during_task: bool,
    /// Whether it needs a session: one is created first when there is none.
    pub(crate) needs_session: bool,
    pub(crate) dispatch: fn(&mut CommandContext<'_>, &str),
}

impl CommandSpec {
    /// Its name, then its aliases.
    pub(crate) fn names(&self) -> impl Iterator<Item = &'static str> {
        std::iter::once(self.name).chain(self.aliases.iter().copied())
    }
}

/// A command is its name.
impl PartialEq for CommandSpec {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl fmt::Debug for CommandSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "/{}", self.name)
    }
}

/// The command named or aliased exactly `name`.
pub(crate) fn find(name: &str) -> Option<&'static CommandSpec> {
    COMMANDS
        .into_iter()
        .find(|command| command.names().any(|known| known == name))
}

/// Whether a command not in `disabled` fuzzily matches `name` (Codex's
/// `codex-common` `fuzzy_match`: a case-insensitive subsequence), so typing
/// it keeps the popup open.
pub(crate) fn has_prefix(name: &str, disabled: &[&str]) -> bool {
    let name = name.to_lowercase();
    COMMANDS
        .into_iter()
        .filter(|command| !disabled.contains(&command.name))
        .flat_map(CommandSpec::names)
        .any(|known| {
            let mut characters = known.chars();
            name.chars().all(|wanted| characters.any(|c| c == wanted))
        })
}

/// What a command may touch: the app's channel and the bottom pane, whose
/// views it pushes as its own.
pub(crate) struct CommandContext<'a> {
    command: &'static CommandSpec,
    app_event_tx: &'a AppEventSender,
    bottom_pane: &'a mut BottomPane,
    mode: AgentSessionMode,
    fullscreen_preference: bool,
}

impl<'a> CommandContext<'a> {
    pub(crate) fn new(
        command: &'static CommandSpec,
        app_event_tx: &'a AppEventSender,
        bottom_pane: &'a mut BottomPane,
        mode: AgentSessionMode,
        fullscreen_preference: bool,
    ) -> Self {
        Self {
            command,
            app_event_tx,
            bottom_pane,
            mode,
            fullscreen_preference,
        }
    }

    pub(crate) fn send(&self, event: AppEvent) {
        self.app_event_tx.send(event);
    }

    pub(crate) fn agent_command(&self, command: AgentCommand) {
        self.send(AppEvent::Op(Op::Command(command)));
    }

    /// The session's mode.
    pub(crate) fn mode(&self) -> AgentSessionMode {
        self.mode
    }

    /// The saved `tui.fullscreenTranscript`.
    pub(crate) fn fullscreen_preference(&self) -> bool {
        self.fullscreen_preference
    }

    /// Shows a list to pick from over the composer, as this command's view.
    pub(crate) fn show_selection_view(&mut self, params: SelectionViewParams) {
        self.bottom_pane
            .show_command_selection_view(self.command.name, params);
    }

    /// Inserts `text` at the composer's cursor.
    pub(crate) fn insert_str(&mut self, text: &str) {
        self.bottom_pane.insert_str(text);
    }
}

#[cfg(test)]
mod tests;
