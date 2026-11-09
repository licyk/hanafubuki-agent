//! The session commands onto the facade's calls: renaming (and the
//! suggested title), archiving, deleting, forking, clearing, exporting,
//! the last answer `/copy` offers, the recap, the background terminals,
//! the workspace roots, and the permission mode and the working directory
//! (`permissions`, `workspace`). What a call found comes back as a
//! [`SessionReply`] for the command that asked; a failure names that
//! command.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use hanafubuki_agent::{
    AgentBackgroundTerminalSnapshot, AgentCompressionStatus, AgentError, AgentEventSink,
    AgentHandle, AgentHistorySortDirection, AgentPermissionMode, AgentRecap, AgentSandboxMode,
    AgentSessionHeadForkRequest, AgentSessionId, AgentSessionRuntimeSnapshot,
    AgentSessionTitleRenameRequest, AgentTranscriptPageRequest, AgentWorktree, ConversationItem,
};

use super::commands::no_session;
use super::permissions::DeniedAction;
use super::{AgentBridge, AgentReply, TurnUpdate};
use crate::app_event::AppEvent;
use crate::resume_picker::ArchiveRequest;

/// How many of the latest transcript items are searched for the last
/// answer.
const LAST_ANSWER_ITEMS: usize = 64;

/// What the session commands ask of the Agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SessionCall {
    /// `/rename`'s prompt: the current title at once, then a suggestion.
    RenamePrompt,
    Rename(String),
    Archive,
    Delete,
    /// A fork of the session at its head, shown from then on; named when a
    /// name is given. Where a worktree can be made, the choice of where it
    /// runs comes first.
    Fork(Option<String>),
    /// A fork in the working directory, without the choice.
    ForkHere(Option<String>),
    /// No session, as `/new`; the session the next message creates gets
    /// the name.
    Clear(Option<String>),
    /// The file `/export` offers to save into.
    ExportPath,
    Export(ExportDestination),
    /// The session's last answer, for `/copy`.
    LastAnswer,
    Recap,
    /// `/compression`: the panel, or with `cache` the cache report.
    Compression {
        cache: bool,
    },
    Terminals,
    StopTerminals,
    Roots,
    /// `/permissions`: the session's own mode and the configured one, and
    /// the sandbox in effect.
    PermissionModes,
    SetPermissionMode(AgentPermissionMode),
    /// A preset: the session's permission mode and sandbox mode.
    SetPreset(AgentPermissionMode, AgentSandboxMode),
    /// The denials `/approve` offers.
    Denials,
    /// Approves the denial with this review id.
    ApproveDenial(String),
    /// `/cd`: the path as typed, a relative one read against the working
    /// directory.
    ChangeDirectory(PathBuf),
    /// `/worktree`'s menu, where a worktree can be made.
    WorktreeMenu,
    /// A new worktree to move into: with `continued`, a fork of the session
    /// (named `name`) goes on there; else the next session starts there.
    EnterWorktree {
        continued: bool,
        name: Option<String>,
    },
    ListWorktrees,
    RemoveWorktree(PathBuf),
}

/// Where `/export` puts the conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ExportDestination {
    Clipboard,
    /// A file, as typed: a relative path is read against the first
    /// workspace root.
    File(PathBuf),
}

/// What the Agent answered a session command.
#[derive(Debug)]
pub(crate) enum SessionReply {
    /// The session's title, for `/rename`'s prompt.
    Title(Option<String>),
    /// The model's suggestion; `None` when there is none (the feature is
    /// off, the conversation is empty).
    TitleSuggestion(Option<String>),
    Renamed(String),
    Archived,
    Deleted,
    Forked(Option<String>),
    /// `<first root>/hanafubuki-agent-session-<id>.md`.
    ExportPath(PathBuf),
    /// The conversation as Markdown, for the clipboard.
    ExportedText(String),
    ExportedFile(PathBuf),
    LastAnswer(Option<String>),
    Recap(AgentRecap),
    /// The session's hierarchical compression; `None` when it has none.
    Compression {
        status: Option<Box<AgentCompressionStatus>>,
        cache: bool,
    },
    Terminals(Vec<AgentBackgroundTerminalSnapshot>),
    TerminalsStopped {
        stopped: usize,
        failed: Vec<String>,
    },
    Roots(Vec<PathBuf>),
    /// A worktree can be made: `/fork` asks where the fork runs.
    ForkChoice(Option<String>),
    PermissionModes {
        session: Option<AgentPermissionMode>,
        configured: AgentPermissionMode,
        /// The session's sandbox mode, else the configured one, else the
        /// platform's default.
        sandbox: AgentSandboxMode,
        /// Whether this machine can give a sandbox.
        sandbox_available: bool,
    },
    PermissionModeSet(AgentPermissionMode),
    PresetSet(AgentPermissionMode, AgentSandboxMode),
    /// The session's latest denials, newest first.
    Denials(Vec<DeniedAction>),
    /// The summary of the action approved.
    Approved(String),
    /// The workspace roots after `/cd`.
    DirectoryChanged(Vec<PathBuf>),
    WorktreeMenu {
        /// There is a session to continue.
        conversation: bool,
    },
    /// The working directory moved into the new worktree at `path`.
    WorktreeEntered {
        path: PathBuf,
        roots: Vec<PathBuf>,
        continued: bool,
    },
    Worktrees(Vec<AgentWorktree>),
    WorktreeRemoved(PathBuf),
    /// `command`'s call failed.
    Failed {
        command: &'static str,
        error: String,
    },
}

impl SessionCall {
    /// The command whose call this is.
    fn command(&self) -> &'static str {
        match self {
            Self::RenamePrompt | Self::Rename(_) => "rename",
            Self::Archive => "archive",
            Self::Delete => "delete",
            Self::Fork(_) | Self::ForkHere(_) => "fork",
            Self::Clear(_) => "clear",
            Self::ExportPath | Self::Export(_) => "export",
            Self::LastAnswer => "copy",
            Self::Recap => "recap",
            Self::Compression { .. } => "compression",
            Self::Terminals => "ps",
            Self::StopTerminals => "stop",
            Self::Roots => "pwd",
            Self::PermissionModes | Self::SetPermissionMode(_) | Self::SetPreset(..) => {
                "permissions"
            }
            Self::Denials | Self::ApproveDenial(_) => "approve",
            Self::ChangeDirectory(_) => "cd",
            Self::WorktreeMenu
            | Self::EnterWorktree { .. }
            | Self::ListWorktrees
            | Self::RemoveWorktree(_) => "worktree",
        }
    }
}

impl SessionReply {
    /// The working directory this reply moved to, if it moved it.
    pub(crate) fn working_directory(&self) -> Option<&Path> {
        match self {
            Self::DirectoryChanged(roots) | Self::WorktreeEntered { roots, .. } => {
                roots.first().map(PathBuf::as_path)
            }
            _ => None,
        }
    }
}

impl AgentBridge {
    pub(super) fn session_call(&mut self, call: SessionCall) -> Option<TurnUpdate> {
        let command = call.command();
        self.run_session_call(call)
            .unwrap_or_else(|error| Some(reply(SessionReply::Failed { command, error })))
    }

    /// Runs `call`: what it found at once, or `None` when its answer comes
    /// back as a reply.
    fn run_session_call(&mut self, call: SessionCall) -> Result<Option<TurnUpdate>, String> {
        let update = match call {
            SessionCall::RenamePrompt => {
                let session = self.current()?;
                let title = session.title.map(|title| title.value);
                self.spawn_session_call("rename", move |agent| async move {
                    // Without a suggestion (the feature is off, the
                    // conversation is empty) the prompt goes on without one.
                    let suggestion = agent.suggest_session_title(session.id).await.ok();
                    Ok(SessionReply::TitleSuggestion(suggestion))
                });
                reply(SessionReply::Title(title))
            }
            SessionCall::Rename(title) => {
                let session = self.current()?;
                let sink = self.sink();
                let renamed = self
                    .agent
                    .rename_session(
                        AgentSessionTitleRenameRequest {
                            session_id: session.id,
                            generation: session.generation,
                            revision: session.revision,
                            title,
                        },
                        &*sink,
                    )
                    .map_err(|error| format!("the session was not renamed: {error}"))?;
                let title = renamed.title.map(|title| title.value).unwrap_or_default();
                reply(SessionReply::Renamed(title))
            }
            SessionCall::Archive | SessionCall::Delete => {
                self.leave_session(call == SessionCall::Archive)?;
                return Ok(None);
            }
            SessionCall::Fork(name)
                if self.session_id.is_some() && self.worktrees_unavailable().is_none() =>
            {
                reply(SessionReply::ForkChoice(name))
            }
            SessionCall::Fork(name) | SessionCall::ForkHere(name) => return self.fork(name),
            SessionCall::Clear(name) => {
                self.pending_title = name;
                let model = self.model.clone();
                self.spawn(|agent| async move {
                    Ok(Some(AgentReply::SessionOpened(
                        no_session(&agent, model).await,
                    )))
                });
                return Ok(None);
            }
            SessionCall::ExportPath => {
                let name = format!("hanafubuki-agent-session-{}.md", self.current_id()?);
                let path = match self.workspace_roots().into_iter().next() {
                    Some(root) => root.join(name),
                    None => PathBuf::from(name),
                };
                reply(SessionReply::ExportPath(path))
            }
            SessionCall::Export(destination) => {
                let session_id = self.current_id()?;
                let root = self.workspace_roots().into_iter().next();
                self.blocking_session_call("export", move |agent| {
                    let markdown = agent
                        .export_session_markdown(session_id)
                        .map_err(|error| format!("the conversation was not exported: {error}"))?;
                    match destination {
                        ExportDestination::Clipboard => Ok(SessionReply::ExportedText(markdown)),
                        ExportDestination::File(path) => {
                            write_new_file(&path, root.as_deref(), &markdown)
                                .map(SessionReply::ExportedFile)
                        }
                    }
                });
                return Ok(None);
            }
            SessionCall::LastAnswer => {
                let answer = match self.session_id {
                    Some(session_id) => last_answer(&self.agent, session_id)?,
                    None => None,
                };
                reply(SessionReply::LastAnswer(answer))
            }
            SessionCall::Recap => {
                let session_id = self.current_id()?;
                self.spawn_session_call("recap", move |agent| async move {
                    agent
                        .recap_session(session_id)
                        .await
                        .map(SessionReply::Recap)
                        .map_err(|error| match error {
                            AgentError::FeatureUnavailable { reason, .. } => {
                                format!("recaps are unavailable: {reason}")
                            }
                            error => format!("no recap: {error}"),
                        })
                });
                return Ok(None);
            }
            SessionCall::Compression { cache } => {
                let Some(session_id) = self.session_id else {
                    return Ok(Some(reply(SessionReply::Compression {
                        status: None,
                        cache,
                    })));
                };
                self.blocking_session_call("compression", move |agent| {
                    let status = agent
                        .compression_status(session_id)
                        .map_err(|error| format!("the compression was not read: {error}"))?;
                    Ok(SessionReply::Compression {
                        status: status.map(Box::new),
                        cache,
                    })
                });
                return Ok(None);
            }
            SessionCall::Terminals => {
                let terminals = match self.session_id {
                    Some(session_id) => {
                        self.agent
                            .list_background_terminals(session_id)
                            .map_err(|error| {
                                format!("the background terminals were not listed: {error}")
                            })?
                    }
                    None => Vec::new(),
                };
                reply(SessionReply::Terminals(terminals))
            }
            SessionCall::StopTerminals => {
                let session_id = self.current_id()?;
                self.blocking_session_call("stop", move |agent| {
                    let outcome = agent
                        .stop_background_terminals_confirmed(session_id)
                        .map_err(|error| {
                            format!("the background terminals were not stopped: {error}")
                        })?;
                    Ok(SessionReply::TerminalsStopped {
                        stopped: outcome.disposed.len(),
                        failed: outcome
                            .retained
                            .into_iter()
                            .map(|failure| failure.message)
                            .collect(),
                    })
                });
                return Ok(None);
            }
            SessionCall::Roots => reply(SessionReply::Roots(self.workspace_roots())),
            SessionCall::PermissionModes => {
                self.permission_modes()?;
                return Ok(None);
            }
            SessionCall::SetPermissionMode(mode) => self.set_permission_mode(mode)?,
            SessionCall::SetPreset(mode, sandbox) => self.set_preset(mode, sandbox)?,
            SessionCall::Denials => self.denials(),
            SessionCall::ApproveDenial(review_id) => self.approve_denial(review_id)?,
            SessionCall::ChangeDirectory(path) => {
                self.change_directory(path);
                return Ok(None);
            }
            SessionCall::WorktreeMenu => self.worktree_menu()?,
            SessionCall::EnterWorktree { continued, name } => {
                self.enter_new_worktree(continued, name)?;
                return Ok(None);
            }
            SessionCall::ListWorktrees => {
                self.list_worktrees();
                return Ok(None);
            }
            SessionCall::RemoveWorktree(path) => {
                self.remove_worktree(path);
                return Ok(None);
            }
        };
        Ok(Some(update))
    }

    /// Archives or deletes the session, off the Agent's runtime (it waits
    /// for the session's background terminals), then shows none (as
    /// `/new`); the command says so once it does.
    fn leave_session(&mut self, archive: bool) -> Result<(), String> {
        let session = self.current()?;
        let model = self.model.clone();
        let app_event_tx = self.app_event_tx.clone();
        let agent = self.agent.clone();
        std::thread::spawn(move || {
            let (generation, revision) = (session.generation, session.revision);
            let done = if archive {
                let archived = agent.archive_session(session.id, generation, revision);
                archived.map(|_| SessionReply::Archived)
            } else {
                let deleted = agent.delete_session_confirmed(session.id, generation, revision);
                deleted.map(|_| SessionReply::Deleted)
            };
            let done = match done {
                Ok(done) => {
                    let shown = agent.clone();
                    match agent
                        .runtime()
                        .block_on(async move { no_session(&shown, model).await })
                    {
                        Ok(opened) => app_event_tx
                            .send(AppEvent::AgentReply(AgentReply::SessionOpened(opened))),
                        Err(failure) => app_event_tx.send(AppEvent::AgentReply(
                            AgentReply::Failed(failure.to_string()),
                        )),
                    }
                    done
                }
                Err(error) if archive => SessionReply::Failed {
                    command: "archive",
                    error: format!("the session was not archived: {error}"),
                },
                Err(error) => SessionReply::Failed {
                    command: "delete",
                    error: format!("the session was not deleted: {error}"),
                },
            };
            app_event_tx.send(AppEvent::AgentReply(AgentReply::Update(Box::new(reply(
                done,
            )))));
        });
        Ok(())
    }

    /// Forks the session at its head and shows the fork, named `name` when
    /// one is given; the command says so once it is shown.
    fn fork(&mut self, name: Option<String>) -> Result<Option<TurnUpdate>, String> {
        let session = self.current()?;
        let sink = self.sink();
        let (fork, named) = fork_at_head(&self.agent, &*sink, &session, name)?;
        self.pending_title = None;
        let opened = self.open(fork)?;
        let forked = reply(SessionReply::Forked(named));
        self.app_event_tx
            .send(AppEvent::AgentReply(AgentReply::Update(Box::new(forked))));
        Ok(self.reply(AgentReply::SessionOpened(opened)))
    }

    /// Archives (or restores) a session the resume picker lists, at its
    /// generation and revision as they are now.
    pub(crate) fn set_archived(&self, request: ArchiveRequest) -> Result<(), String> {
        let session = self
            .agent
            .open_session(request.session_id, 1)
            .map_err(|error| error.to_string())?
            .runtime;
        let (id, generation, revision) = (session.id, session.generation, session.revision);
        let done = if request.archive {
            self.agent.archive_session(id, generation, revision)
        } else {
            self.agent.restore_session(id, generation, revision)
        };
        done.map(drop).map_err(|error| error.to_string())
    }

    pub(super) fn current_id(&self) -> Result<AgentSessionId, String> {
        self.session_id
            .ok_or_else(|| "there is no session yet".to_string())
    }

    /// Runs `call`, which blocks (it reads or writes the history, or waits
    /// for processes), on a thread of its own, off the Agent's runtime;
    /// what it found, or its failure, comes back as `command`'s reply.
    pub(super) fn blocking_session_call(
        &self,
        command: &'static str,
        call: impl FnOnce(AgentHandle) -> Result<SessionReply, String> + Send + 'static,
    ) {
        let agent = self.agent.clone();
        let app_event_tx = self.app_event_tx.clone();
        std::thread::spawn(move || {
            let done = call(agent).unwrap_or_else(|error| SessionReply::Failed { command, error });
            app_event_tx.send(AppEvent::AgentReply(AgentReply::Update(Box::new(reply(
                done,
            )))));
        });
    }

    /// Runs `call` on the Agent's runtime; what it found, or its failure,
    /// comes back as `command`'s reply.
    pub(super) fn spawn_session_call<F>(
        &self,
        command: &'static str,
        call: impl FnOnce(AgentHandle) -> F,
    ) where
        F: std::future::Future<Output = Result<SessionReply, String>> + Send + 'static,
    {
        let call = call(self.agent.clone());
        self.spawn(move |_| async move {
            let done = call
                .await
                .unwrap_or_else(|error| SessionReply::Failed { command, error });
            Ok(Some(AgentReply::Update(Box::new(reply(done)))))
        });
    }
}

fn reply(reply: SessionReply) -> TurnUpdate {
    TurnUpdate::Session(reply)
}

/// A fork of `session` at its head, named `name` when one is given: its
/// id and its title.
pub(super) fn fork_at_head(
    agent: &AgentHandle,
    sink: &dyn AgentEventSink,
    session: &AgentSessionRuntimeSnapshot,
    name: Option<String>,
) -> Result<(AgentSessionId, Option<String>), String> {
    let fork = agent
        .fork_session_at_head(AgentSessionHeadForkRequest {
            source_session_id: session.id,
            source_generation: session.generation,
            source_revision: session.revision,
        })
        .map_err(|error| format!("the session was not forked: {error}"))?
        .session;
    let Some(title) = name else {
        return Ok((fork.id, None));
    };
    let renamed = agent
        .rename_session(
            AgentSessionTitleRenameRequest {
                session_id: fork.id,
                generation: fork.generation,
                revision: fork.revision,
                title,
            },
            sink,
        )
        .map_err(|error| format!("the fork was made but not named: {error}"))?;
    Ok((fork.id, renamed.title.map(|title| title.value)))
}

/// The text of the session's latest assistant message, if any.
fn last_answer(agent: &AgentHandle, session_id: AgentSessionId) -> Result<Option<String>, String> {
    let page = agent
        .read_transcript_page(AgentTranscriptPageRequest {
            session_id,
            page_size: LAST_ANSWER_ITEMS,
            cursor: None,
            direction: AgentHistorySortDirection::Descending,
            through_revision: None,
        })
        .map_err(|error| format!("the last answer was not read: {error}"))?;
    Ok(page.items.into_iter().find_map(|item| match item.content {
        ConversationItem::AssistantMessage { text, .. } if !text.trim().is_empty() => Some(text),
        _ => None,
    }))
}

/// Writes `text` into `path`, a relative one under `root`; an existing file
/// is not replaced.
fn write_new_file(path: &Path, root: Option<&Path>, text: &str) -> Result<PathBuf, String> {
    let path = match root {
        Some(root) if path.is_relative() => root.join(path),
        _ => path.to_path_buf(),
    };
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| format!("{} was not created: {error}", path.display()))?;
    file.write_all(text.as_bytes())
        .map_err(|error| format!("{} was not written: {error}", path.display()))?;
    Ok(path)
}
