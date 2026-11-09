// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! The slash commands, one table of them: each command's spec and the
//! module that runs it. The composer's popup, its reading of a typed
//! `/name args` and the chat's dispatch all come from the table.
//!
//! A command runs inside `catch_panic`, and so do the views it pushes
//! (`bottom_pane::module_view`): a command that panics is turned off for
//! the rest of the run, and the chat goes on.

mod clear;
mod confirm;
mod copy;
mod exit;
mod export;
mod fork;
pub(crate) mod goal;
pub(crate) mod hooks;
mod init;
mod leave;
pub(crate) mod mcp;
mod mode;
mod model;
mod prompt;
pub(crate) mod providers;
mod pwd;
mod raw;
mod recap;
mod rename;
mod reports;
mod resume;
pub(crate) mod review;
mod session;
mod statusline;
mod terminals;
mod theme;
mod title;
mod tui;
mod workspace;

use std::fmt;

use hanafubuki_agent::AgentSessionMode;

use crate::agent_bridge::{HooksReply, McpReply, ProviderReply, SessionCall, SessionReply};
use crate::app_event::{AgentCommand, AppEvent, AppEventSender, Op};
use crate::appearance::Appearance;
use crate::bottom_pane::{BottomPane, BottomPaneView, SelectionViewParams};
use crate::history_cell::{self, HistoryCell};
pub(crate) use goal::GOAL;
pub(crate) use hooks::HOOKS;
pub(crate) use mcp::MCP;
pub(crate) use mode::mode_name;
pub(crate) use model::{show_model_choices, MODEL};
pub(crate) use providers::PROVIDERS;
pub(crate) use review::REVIEW;

/// Every command, in presentation order: the most used first.
pub(crate) static COMMANDS: [&CommandSpec; 32] = [
    &model::MODEL,
    &providers::PROVIDERS,
    &mode::MODE,
    &reports::SKILLS,
    &rename::RENAME,
    &session::NEW,
    &leave::ARCHIVE,
    &leave::DELETE,
    &resume::RESUME,
    &fork::FORK,
    &init::INIT,
    &session::COMPACT,
    &recap::RECAP,
    &review::REVIEW,
    &goal::GOAL,
    &copy::COPY,
    &export::EXPORT,
    &workspace::DIFF,
    &workspace::MENTION,
    &reports::STATUS,
    &pwd::PWD,
    &mcp::MCP,
    &hooks::HOOKS,
    &raw::RAW,
    &theme::THEME,
    &statusline::STATUSLINE,
    &title::TITLE,
    &tui::TUI,
    &terminals::PS,
    &terminals::STOP,
    &clear::CLEAR,
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

/// Sends `call` to the Agent, from a view's action.
pub(crate) fn send_session_call(app_event_tx: &AppEventSender, call: SessionCall) {
    app_event_tx.send(AppEvent::Op(Op::Command(AgentCommand::Session(call))));
}

/// What the Agent answered a command's call, for the command's view.
#[derive(Debug)]
pub(crate) enum CommandReply {
    Providers(ProviderReply),
    Mcp(McpReply),
    Session(SessionReply),
    Hooks(HooksReply),
}

/// The command a session call's reply belongs to.
pub(crate) fn session_reply_command(reply: &SessionReply) -> Option<&'static CommandSpec> {
    Some(match reply {
        SessionReply::Title(_) | SessionReply::TitleSuggestion(_) | SessionReply::Renamed(_) => {
            &rename::RENAME
        }
        SessionReply::Archived => &leave::ARCHIVE,
        SessionReply::Deleted => &leave::DELETE,
        SessionReply::Forked(_) => &fork::FORK,
        SessionReply::ExportPath(_)
        | SessionReply::ExportedText(_)
        | SessionReply::ExportedFile(_) => &export::EXPORT,
        SessionReply::LastAnswer(_) => &copy::COPY,
        SessionReply::Recap(_) => &recap::RECAP,
        SessionReply::Terminals(_) => &terminals::PS,
        SessionReply::TerminalsStopped { .. } => &terminals::STOP,
        SessionReply::Roots(_) => &pwd::PWD,
        SessionReply::Failed { command, .. } => return find(command),
    })
}

/// What a session call found, handed to the command that asked; a failure
/// goes into the transcript.
pub(crate) fn on_session_reply(context: &mut CommandContext<'_>, reply: SessionReply) {
    match reply {
        SessionReply::Failed { command, error } => context.add_to_history(
            history_cell::new_error_event(format!("/{command}: {error}")),
        ),
        SessionReply::Title(_) | SessionReply::TitleSuggestion(_) | SessionReply::Renamed(_) => {
            rename::on_reply(context, reply)
        }
        SessionReply::Archived | SessionReply::Deleted => leave::on_reply(context, reply),
        SessionReply::Forked(name) => fork::on_reply(context, name),
        SessionReply::ExportPath(_)
        | SessionReply::ExportedText(_)
        | SessionReply::ExportedFile(_) => export::on_reply(context, reply),
        SessionReply::LastAnswer(answer) => copy::on_reply(context, answer),
        SessionReply::Recap(recap) => recap::on_reply(context, recap),
        SessionReply::Terminals(_) | SessionReply::TerminalsStopped { .. } => {
            terminals::on_reply(context, reply)
        }
        SessionReply::Roots(roots) => pwd::on_reply(context, roots),
    }
}

/// What a command may touch: the app's channel and the bottom pane, whose
/// views it pushes as its own.
pub(crate) struct CommandContext<'a> {
    command: &'static CommandSpec,
    app_event_tx: &'a AppEventSender,
    bottom_pane: &'a mut BottomPane,
    mode: AgentSessionMode,
    fullscreen_preference: bool,
    owned_transcript: bool,
    appearance: &'a Appearance,
}

impl<'a> CommandContext<'a> {
    pub(crate) fn new(
        command: &'static CommandSpec,
        app_event_tx: &'a AppEventSender,
        bottom_pane: &'a mut BottomPane,
        mode: AgentSessionMode,
        fullscreen_preference: bool,
        owned_transcript: bool,
        appearance: &'a Appearance,
    ) -> Self {
        Self {
            command,
            app_event_tx,
            bottom_pane,
            mode,
            fullscreen_preference,
            owned_transcript,
            appearance,
        }
    }

    /// The theme, the status line and the title as this run has them.
    pub(crate) fn appearance(&self) -> &Appearance {
        self.appearance
    }

    pub(crate) fn send(&self, event: AppEvent) {
        self.app_event_tx.send(event);
    }

    pub(crate) fn agent_command(&self, command: AgentCommand) {
        self.send(AppEvent::Op(Op::Command(command)));
    }

    pub(crate) fn session_call(&self, call: SessionCall) {
        self.agent_command(AgentCommand::Session(call));
    }

    /// The app owns the transcript (fullscreen), so it can select in it.
    pub(crate) fn owned_transcript(&self) -> bool {
        self.owned_transcript
    }

    /// The session's mode.
    pub(crate) fn mode(&self) -> AgentSessionMode {
        self.mode
    }

    /// The saved `tui.fullscreenTranscript`.
    pub(crate) fn fullscreen_preference(&self) -> bool {
        self.fullscreen_preference
    }

    /// A turn, or the work of a command that shows as one, runs.
    pub(crate) fn task_running(&self) -> bool {
        self.bottom_pane.is_task_running()
    }

    /// Shows this command's work as the running task, `header` in the
    /// status line (Esc interrupts it), or (`None`) ends it.
    pub(crate) fn set_task_status(&mut self, header: Option<String>) {
        match header {
            Some(header) => {
                self.bottom_pane.set_task_running(true);
                self.bottom_pane.update_status(header, None);
            }
            None => self.bottom_pane.set_task_running(false),
        }
    }

    /// Shows a list to pick from over the composer, as this command's view.
    pub(crate) fn show_selection_view(&mut self, params: SelectionViewParams) {
        self.bottom_pane
            .show_command_selection_view(self.command.name, params);
    }

    /// A sender for the views this command shows.
    pub(crate) fn app_event_tx(&self) -> AppEventSender {
        self.app_event_tx.clone()
    }

    /// Shows `view` over the composer as this command's, in place of the
    /// one it has open.
    pub(crate) fn show_view(&mut self, view: Box<dyn BottomPaneView>) {
        self.bottom_pane.show_command_view(self.command.name, view);
    }

    /// Hands `reply` to this command's open view; it comes back when none
    /// takes it.
    pub(crate) fn offer_reply(&mut self, reply: CommandReply) -> Option<CommandReply> {
        self.bottom_pane
            .offer_command_reply(self.command.name, reply)
    }

    pub(crate) fn add_to_history(&self, cell: impl HistoryCell + 'static) {
        self.send(AppEvent::InsertHistoryCell(Box::new(cell)));
    }

    /// Inserts `text` at the composer's cursor.
    pub(crate) fn insert_str(&mut self, text: &str) {
        self.bottom_pane.insert_str(text);
    }
}

#[cfg(test)]
mod tests;
