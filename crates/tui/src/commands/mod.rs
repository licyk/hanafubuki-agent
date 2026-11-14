// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! The slash commands, one table of them: each command's spec and the
//! module that runs it. The composer's popup, its reading of a typed
//! `/name args` and the chat's dispatch all come from the table.
//!
//! A command runs inside `catch_panic`, and so do the views it pushes
//! (`bottom_pane::module_view`) and its later steps ([`CommandStep`]): a
//! command that panics is turned off for the rest of the run, and the chat
//! goes on.

mod agents;
mod cd;
mod clear;
mod compression;
mod config;
mod confirm;
mod context;
mod copy;
pub(crate) mod effort;
mod exit;
mod export;
mod fork;
pub(crate) mod goal;
pub(crate) mod hooks;
mod init;
mod keymap;
mod leave;
pub(crate) mod mcp;
mod memories;
mod mode;
mod model;
mod parse;
mod permissions;
pub(crate) mod plugins;
mod presets;
mod prompt;
pub(crate) mod providers;
mod pwd;
mod raw;
mod recap;
mod rename;
mod reports;
mod resume;
pub(crate) mod review;
mod role;
mod session;
mod setup_sandbox;
mod side;
mod statusline;
pub(crate) mod subagents;
mod terminals;
mod theme;
mod title;
mod tui;
mod warnings;
mod workspace;
mod worktree;

use std::fmt;

use hanafubuki_agent::{AgentSessionMode, AgentSubagentStatus, AgentTuiPreferences};

use crate::agent_bridge::model_list::ModelListUpdate;
use crate::agent_bridge::{
    HooksReply, McpReply, PluginsReply, ProviderModels, SessionCall, SessionReply, SettingsReply,
};
use crate::app::WorkspaceEvent;
use crate::app_event::{AgentCommand, AppEvent, AppEventSender, Op};
use crate::appearance::Appearance;
use crate::bottom_pane::{BottomPane, BottomPaneView, SelectionViewParams};
use crate::history_cell::{self, HistoryCell};
use crate::settings_center::PageOverride;
pub(crate) use effort::{show_effort_choices, EFFORT};
pub(crate) use goal::GOAL;
pub(crate) use hooks::HOOKS;
pub(crate) use mcp::MCP;
pub(crate) use memories::MEMORIES;
pub(crate) use mode::{mode_name, MODE};
pub(crate) use model::{on_model_list, MODEL};
pub(crate) use parse::{parse_input, ParsedInput};
pub(crate) use plugins::PLUGINS;
pub(crate) use presets::PRESETS;
pub(crate) use providers::PROVIDERS;
pub(crate) use review::REVIEW;
pub(crate) use role::{show_role_choices, ROLE};
pub(crate) use side::SECOND_CONVERSATION_COMMANDS;
pub(crate) use subagents::SUBAGENTS;
pub(crate) use warnings::{show_warnings, WARNINGS};

/// Every command, in presentation order: the most used first.
pub(crate) static COMMANDS: [&CommandSpec; 50] = [
    &model::MODEL,
    &effort::EFFORT,
    &providers::PROVIDERS,
    &mode::MODE,
    &role::ROLE,
    &permissions::PERMISSIONS,
    &permissions::APPROVE,
    &setup_sandbox::SETUP_DEFAULT_SANDBOX,
    &reports::SKILLS,
    &rename::RENAME,
    &session::NEW,
    &leave::ARCHIVE,
    &leave::DELETE,
    &resume::RESUME,
    &fork::FORK,
    &worktree::WORKTREE,
    &init::INIT,
    &session::COMPACT,
    &compression::COMPRESSION,
    &recap::RECAP,
    &review::REVIEW,
    &side::SIDE,
    &subagents::SUBAGENTS,
    &agents::AGENTS,
    &goal::GOAL,
    &copy::COPY,
    &export::EXPORT,
    &workspace::DIFF,
    &workspace::MENTION,
    &reports::STATUS,
    &context::CONTEXT,
    &pwd::PWD,
    &cd::CD,
    &plugins::PLUGINS,
    &mcp::MCP,
    &hooks::HOOKS,
    &raw::RAW,
    &theme::THEME,
    &statusline::STATUSLINE,
    &title::TITLE,
    &tui::TUI,
    &config::CONFIG,
    &keymap::KEYMAP,
    &presets::PRESETS,
    &memories::MEMORIES,
    &warnings::WARNINGS,
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

/// The commands this platform leaves out of the popup: the Windows
/// sandbox's setup elsewhere. Typed, they still run and say so.
pub(crate) const HIDDEN_HERE: &[&str] = if cfg!(windows) {
    &[]
} else {
    &["setup-default-sandbox"]
};

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
        .filter(|command| !disabled.contains(&command.name) && !HIDDEN_HERE.contains(&command.name))
        .flat_map(CommandSpec::names)
        .any(|known| {
            let mut characters = known.chars();
            name.chars().all(|wanted| characters.any(|c| c == wanted))
        })
}

/// The settings center's pages that are commands' views, each declared
/// beside its command.
pub(crate) static SETTINGS_PAGES: [&PageOverride; 10] = [
    &theme::PAGE,
    &statusline::PAGE,
    &title::PAGE,
    &keymap::PAGE,
    &presets::PAGE,
    &plugins::PAGE,
    &hooks::PAGE,
    &memories::PAGE,
    &providers::MODEL_PAGE,
    &mcp::LOGIN_PAGE,
];

/// Opens the settings center at `target` (a tab, a section, a collection
/// or a page): what a command that only changes settings does.
pub(crate) fn open_settings(context: &CommandContext<'_>, target: &str) {
    context.send(AppEvent::Workspace(WorkspaceEvent::OpenSettings {
        target: Some(target.to_string()),
        record: None,
    }));
}

/// Sends `call` to the Agent, from a view's action.
pub(crate) fn send_session_call(app_event_tx: &AppEventSender, call: SessionCall) {
    app_event_tx.send(AppEvent::Op(Op::Command(AgentCommand::Session(call))));
}

/// What a command does next, asked by one of its views' actions, which
/// have only the app's channel: another view in place of the one closed
/// (a confirmation, a listed entry's actions).
pub(crate) struct CommandStep {
    pub(crate) command: &'static CommandSpec,
    pub(crate) run: Box<dyn FnOnce(&mut CommandContext<'_>) + Send>,
}

/// Has `run` run as `command`'s next step.
pub(crate) fn send_step(
    app_event_tx: &AppEventSender,
    command: &'static CommandSpec,
    run: impl FnOnce(&mut CommandContext<'_>) + Send + 'static,
) {
    app_event_tx.send(AppEvent::CommandStep(CommandStep {
        command,
        run: Box::new(run),
    }));
}

/// What the Agent answered a command's call, for the command's view.
#[derive(Debug)]
pub(crate) enum CommandReply {
    ProviderModels(ProviderModels),
    /// What `/model`'s listing heard.
    ModelList(ModelListUpdate),
    Mcp(McpReply),
    Session(SessionReply),
    Hooks(HooksReply),
    Plugins(PluginsReply),
    Settings(SettingsReply),
    /// The subagents' statuses, read again while they run.
    Subagents(Vec<AgentSubagentStatus>),
}

/// What the memory store's page asked; `/warnings`' answer goes through
/// the chat, which adds its own warnings.
pub(crate) fn on_settings_reply(context: &mut CommandContext<'_>, reply: SettingsReply) {
    match reply {
        // An open panel says it under its rows.
        SettingsReply::Failed { .. } => {
            if let Some(CommandReply::Settings(SettingsReply::Failed { command, error })) =
                context.offer_reply(CommandReply::Settings(reply))
            {
                context.add_to_history(history_cell::new_error_event(format!(
                    "/{command}: {error}"
                )));
            }
        }
        SettingsReply::Memories(state) => memories::on_reply(context, *state),
        SettingsReply::Warnings(_) => {}
    }
}

/// The command a session call's reply belongs to.
pub(crate) fn session_reply_command(reply: &SessionReply) -> Option<&'static CommandSpec> {
    Some(match reply {
        SessionReply::Title(_) | SessionReply::TitleSuggestion(_) | SessionReply::Renamed(_) => {
            &rename::RENAME
        }
        SessionReply::Archived => &leave::ARCHIVE,
        SessionReply::Deleted => &leave::DELETE,
        SessionReply::Forked(_) | SessionReply::ForkChoice(_) => &fork::FORK,
        SessionReply::ExportPath(_)
        | SessionReply::ExportedText(_)
        | SessionReply::ExportedFile(_) => &export::EXPORT,
        SessionReply::LastAnswer(_) => &copy::COPY,
        SessionReply::Recap(_) => &recap::RECAP,
        SessionReply::Compression { .. } => &compression::COMPRESSION,
        SessionReply::Terminals(_) => &terminals::PS,
        SessionReply::TerminalsStopped { .. } => &terminals::STOP,
        SessionReply::Roots(_) => &pwd::PWD,
        SessionReply::PermissionModes { .. }
        | SessionReply::PermissionModeSet(_)
        | SessionReply::PresetSet(..) => &permissions::PERMISSIONS,
        SessionReply::Denials(_) | SessionReply::Approved(_) => &permissions::APPROVE,
        SessionReply::DirectoryChanged(_) => &cd::CD,
        SessionReply::WorktreeMenu { .. }
        | SessionReply::WorktreeEntered { .. }
        | SessionReply::Worktrees(_)
        | SessionReply::WorktreeRemoved(_) => &worktree::WORKTREE,
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
        SessionReply::Forked(_) | SessionReply::ForkChoice(_) => fork::on_reply(context, reply),
        SessionReply::ExportPath(_)
        | SessionReply::ExportedText(_)
        | SessionReply::ExportedFile(_) => export::on_reply(context, reply),
        SessionReply::LastAnswer(answer) => copy::on_reply(context, answer),
        SessionReply::Recap(recap) => recap::on_reply(context, recap),
        SessionReply::Compression { status, cache } => {
            compression::on_reply(context, status.map(|status| *status), cache)
        }
        SessionReply::Terminals(_) | SessionReply::TerminalsStopped { .. } => {
            terminals::on_reply(context, reply)
        }
        SessionReply::Roots(roots) => pwd::on_reply(context, roots),
        SessionReply::PermissionModes { .. }
        | SessionReply::PermissionModeSet(_)
        | SessionReply::PresetSet(..)
        | SessionReply::Denials(_)
        | SessionReply::Approved(_) => permissions::on_reply(context, reply),
        SessionReply::DirectoryChanged(roots) => cd::on_reply(context, roots),
        SessionReply::WorktreeMenu { .. }
        | SessionReply::WorktreeEntered { .. }
        | SessionReply::Worktrees(_)
        | SessionReply::WorktreeRemoved(_) => worktree::on_reply(context, reply),
    }
}

/// What a command may touch: the app's channel and the bottom pane, whose
/// views it pushes as its own.
pub(crate) struct CommandContext<'a> {
    command: &'static CommandSpec,
    app_event_tx: &'a AppEventSender,
    bottom_pane: &'a mut BottomPane,
    mode: AgentSessionMode,
    owned_transcript: bool,
    appearance: &'a Appearance,
    preferences: &'a AgentTuiPreferences,
}

impl<'a> CommandContext<'a> {
    pub(crate) fn new(
        command: &'static CommandSpec,
        app_event_tx: &'a AppEventSender,
        bottom_pane: &'a mut BottomPane,
        mode: AgentSessionMode,
        owned_transcript: bool,
        appearance: &'a Appearance,
        preferences: &'a AgentTuiPreferences,
    ) -> Self {
        Self {
            command,
            app_event_tx,
            bottom_pane,
            mode,
            owned_transcript,
            appearance,
            preferences,
        }
    }

    /// `[tui]` as the configuration resolves it now.
    pub(crate) fn preferences(&self) -> &AgentTuiPreferences {
        self.preferences
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
