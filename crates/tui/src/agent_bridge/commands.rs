//! The slash commands that ask something of the Agent, each onto the
//! facade's calls: no session (`/new`) or a reopened one, compaction, the
//! model and the mode of the session, and what `/status`, `/context`, `/mcp`
//! and `/skills` report; and the creation of the session the interface drives.
//! A call that waits runs on the Agent's runtime; one that does not runs
//! here.

use std::path::PathBuf;

use hanafubuki_agent::{
    AgentHandle, AgentHistorySortDirection, AgentPermissionMode, AgentProviderModelCatalogRequest,
    AgentProviderModelCatalogResult, AgentProviderRegistrySnapshot, AgentSessionId,
    AgentSessionListRequest, AgentSessionMode, AgentSessionModelSelectionRequest, AgentSessionPage,
    AgentSessionRuntimeSnapshot, AgentSessionTitleRenameRequest, AgentTranscriptPageRequest,
    AgentTuiPreferences, AgentTuiPreferencesPatch, ModelMetadata, SettingsWrite,
};

use super::{
    replay, AgentBridge, AgentReply, OlderTranscript, OpenedSession, ReplayedItem, TurnUpdate,
};
use crate::app_event::{AgentCommand, AppCommand, AppEvent, Op};

/// How many of a reopened session's latest transcript items are replayed,
/// and how many more each earlier page holds.
pub(super) const REPLAYED_ITEMS: usize = 200;

/// Whether the interface must set up a Provider before a session can
/// run a turn: no Provider is the default, or no model is chosen (and none
/// given with `-m`).
pub(crate) fn setup_needed(registry: &AgentProviderRegistrySnapshot, model_override: bool) -> bool {
    registry.default_provider_id.is_none()
        || (!model_override && registry.model_policy.model.trim().is_empty())
}

/// What `/model` offers: each Provider's models, and the session's own.
#[derive(Debug)]
pub(crate) struct ModelChoices {
    pub(crate) provider_id: String,
    pub(crate) model: String,
    pub(crate) catalogs: Vec<AgentProviderModelCatalogResult>,
}

/// What `/role` offers: the usable roles, by name with their
/// descriptions, and the session's (`None` for `default`).
#[derive(Debug)]
pub(crate) struct RoleChoices {
    pub(crate) current: Option<String>,
    pub(crate) roles: Vec<(String, String)>,
}

/// What `/status` shows.
#[derive(Debug)]
pub(crate) struct StatusReport {
    pub(crate) session: AgentSessionRuntimeSnapshot,
    /// The session's Provider, by its name.
    pub(crate) provider: String,
    pub(crate) permission_mode: AgentPermissionMode,
    pub(crate) roots: Vec<PathBuf>,
    /// What the session knows of its model: its window with its source,
    /// and what narrows the reasoning effort.
    pub(crate) metadata: ModelMetadata,
}

impl AgentBridge {
    pub(super) fn command(&mut self, command: AgentCommand) -> Option<TurnUpdate> {
        self.run_command(command)
            .unwrap_or_else(|message| Some(TurnUpdate::Error(message)))
    }

    /// Runs `command`: what it found at once, or `None` when its answer
    /// comes back as a reply.
    fn run_command(&mut self, command: AgentCommand) -> Result<Option<TurnUpdate>, String> {
        match command {
            AgentCommand::NewSession => {
                self.pending_title = None;
                let model = self.model.clone();
                self.spawn(|agent| async move {
                    Ok(Some(AgentReply::SessionOpened(
                        no_session(&agent, model).await,
                    )))
                })
            }
            AgentCommand::Resume(session_id) => {
                self.pending_title = None;
                let opened = self.open(session_id)?;
                return Ok(self.reply(AgentReply::SessionOpened(opened)));
            }
            AgentCommand::Compact => {
                let session = self.current()?;
                let sink = self.sink();
                self.spawn(move |agent| async move {
                    agent
                        .compact_session_with_sink(
                            session.id,
                            session.generation,
                            session.revision,
                            sink,
                        )
                        .await
                        .map_err(|error| format!("the conversation was not compacted: {error}"))?;
                    // The install's event showed it, as every install's does.
                    Ok(None)
                })
            }
            AgentCommand::ListModels => {
                let session = self.current()?;
                self.spawn(move |agent| async move {
                    let catalogs = agent
                        .list_provider_models(AgentProviderModelCatalogRequest::default())
                        .await
                        .map_err(|error| format!("the models were not listed: {error}"))?;
                    Ok(Some(AgentReply::Update(Box::new(
                        TurnUpdate::ModelChoices(ModelChoices {
                            provider_id: session.provider_id,
                            model: session.model,
                            catalogs,
                        }),
                    ))))
                })
            }
            AgentCommand::SetModel { provider_id, model } => {
                let session = self.current()?;
                self.spawn(move |agent| async move {
                    let session = agent
                        .set_session_model(AgentSessionModelSelectionRequest {
                            session_id: session.id,
                            generation: session.generation,
                            revision: session.revision,
                            provider_id,
                            model,
                        })
                        .await
                        .map_err(|error| format!("the model was not changed: {error}"))?;
                    Ok(Some(AgentReply::Update(Box::new(
                        TurnUpdate::ModelChanged(session.model),
                    ))))
                })
            }
            AgentCommand::ForgetLearnedModelFacts => {
                let session = self.current()?;
                self.spawn(move |agent| async move {
                    let forgotten = agent
                        .forget_learned_model_facts(&session.provider_id, Some(&session.model))
                        .await
                        .map_err(|error| format!("nothing was forgotten: {error}"))?;
                    Ok(Some(AgentReply::Update(Box::new(
                        TurnUpdate::LearnedModelFactsForgotten {
                            model: session.model,
                            forgotten: forgotten > 0,
                        },
                    ))))
                })
            }
            AgentCommand::ListRoles => {
                let session = self.current()?;
                let roles = self
                    .agent
                    .roles()
                    .roles()
                    .map(|role| (role.name.clone(), role.description.clone()))
                    .collect();
                return Ok(Some(TurnUpdate::RoleChoices(RoleChoices {
                    current: session.role,
                    roles,
                })));
            }
            AgentCommand::SetRole(role) => {
                let session = self.current()?;
                self.spawn(move |agent| async move {
                    let session = agent
                        .set_session_role(session.id, &role)
                        .await
                        .map_err(|error| format!("the role was not changed: {error}"))?;
                    Ok(Some(AgentReply::Update(Box::new(TurnUpdate::RoleChanged(
                        session.role,
                    )))))
                })
            }
            AgentCommand::SetMode(mode) => {
                let session = self.current()?;
                let session = self
                    .agent
                    .set_session_mode(session.id, session.generation, session.revision, mode)
                    .map_err(|error| format!("the mode was not changed: {error}"))?;
                return Ok(Some(TurnUpdate::ModeChanged(session.mode)));
            }
            AgentCommand::Status => {
                let session = self.current()?;
                self.spawn(move |agent| async move {
                    // The session's own mode, else the configured one.
                    let permission_mode = match agent
                        .session_permission_mode(session.id)
                        .map_err(|error| error.to_string())?
                    {
                        Some(mode) => mode,
                        None => agent.settings_view().await?.settings.permission_mode,
                    };
                    let registry = agent.provider_registry_snapshot().await?;
                    let roots = agent.workspace_roots()?.into_vec();
                    let metadata = agent
                        .session_model_metadata(session.id)
                        .map_err(|error| error.to_string())?;
                    let provider = registry
                        .providers
                        .iter()
                        .find(|provider| provider.id == session.provider_id)
                        .map_or_else(
                            || session.provider_id.clone(),
                            |provider| provider.display_name.clone(),
                        );
                    Ok(Some(AgentReply::Update(Box::new(
                        TurnUpdate::StatusReport(Box::new(StatusReport {
                            session,
                            provider,
                            permission_mode,
                            roots,
                            metadata,
                        })),
                    ))))
                })
            }
            AgentCommand::Context => {
                let session = self.current()?;
                self.spawn(move |agent| async move {
                    let context = agent
                        .session_context(session.id)
                        .await
                        .map_err(|error| format!("the context was not read: {error}"))?;
                    Ok(Some(AgentReply::Update(Box::new(
                        TurnUpdate::ContextReport(Box::new(context)),
                    ))))
                })
            }
            AgentCommand::ProviderModels(provider_id) => self.list_provider_models(provider_id),
            AgentCommand::Mcp(call) => self.mcp_call(call),
            AgentCommand::Session(call) => return Ok(self.session_call(call)),
            AgentCommand::Review(call) => return Ok(self.review_call(call)),
            AgentCommand::Goal(call) => return Ok(self.goal_call(call)),
            AgentCommand::Hooks(call) => return Ok(self.hooks_call(call)),
            AgentCommand::Plugins(call) => return Ok(self.plugins_call(call)),
            AgentCommand::Settings(call) => self.settings_call(call),
            AgentCommand::Subagents(call) => return Ok(self.subagents_call(call)),
            AgentCommand::Skills => {
                let catalog = self
                    .agent
                    .list_skills()
                    .map_err(|error| format!("the Skills were not listed: {error}"))?;
                return Ok(Some(TurnUpdate::Skills(catalog)));
            }
        }
        Ok(None)
    }

    /// A page of the sessions the resume picker lists.
    pub(crate) fn list_sessions(
        &self,
        request: AgentSessionListRequest,
    ) -> Result<AgentSessionPage, String> {
        self.agent
            .list_sessions_page(request)
            .map_err(|error| format!("the sessions were not listed: {error}"))
    }

    /// The interface's preferences (`[tui]`); the defaults when the
    /// configuration cannot be read.
    pub(crate) fn tui_preferences(&self) -> AgentTuiPreferences {
        let agent = self.agent.clone();
        self.agent
            .runtime()
            .block_on(async move { agent.tui_preferences().await })
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default()
    }

    /// Saves `patch` into the interface's preferences off the loop; the
    /// preferences as the layers then resolve them come back as
    /// [`AppCommand::PreferencesSaved`] with `message`.
    pub(crate) fn save_tui_preferences(&self, patch: AgentTuiPreferencesPatch, message: String) {
        let app_event_tx = self.app_event_tx.clone();
        let agent = self.agent.clone();
        self.agent.runtime().handle().spawn(async move {
            let saved = agent.write_tui_preferences(patch.clone()).await;
            app_event_tx.send(AppEvent::Command(AppCommand::PreferencesSaved {
                patch,
                message,
                saved,
            }));
        });
    }

    /// Writes `[tui]` settings by key off the loop, each checked by its
    /// spec; `[tui]` as the layers then resolve it comes back as
    /// [`AppCommand::TuiSettingsWritten`] with `message`.
    pub(crate) fn write_tui_settings(&self, write: SettingsWrite, message: String) {
        let app_event_tx = self.app_event_tx.clone();
        let agent = self.agent.clone();
        self.agent.runtime().handle().spawn(async move {
            let (written, shadowed) = match agent.write_setting_values(write).await {
                Ok(outcome) => (agent.tui_preferences().await, !outcome.shadowed.is_empty()),
                Err(error) => (Err(error), false),
            };
            app_event_tx.send(AppEvent::Command(AppCommand::TuiSettingsWritten {
                message,
                written,
                shadowed,
            }));
        });
    }

    /// The workspace roots, the first the one relative paths are read
    /// against.
    pub(crate) fn workspace_roots(&self) -> Vec<PathBuf> {
        self.agent
            .workspace_roots()
            .map(|roots| roots.into_vec())
            .unwrap_or_default()
    }

    /// What the interface shows before its first message creates a session.
    pub(crate) fn no_session(&self) -> Result<OpenedSession, String> {
        let agent = self.agent.clone();
        let model = self.model.clone();
        self.agent
            .runtime()
            .block_on(async move { no_session(&agent, model).await })
            .map_err(|failure| failure.to_string())
    }

    /// Creates the session the interface drives, bound to the worktree
    /// `/worktree` moved into for it; `waiting` and what is asked until it
    /// exists are carried out then.
    pub(super) fn create_session(&mut self, waiting: Vec<Op>) {
        self.creating = Some(waiting);
        let agent = self.agent.clone();
        let model = self.model.clone();
        let role = self.role.clone();
        let title = self.pending_title.take();
        let worktree = self.pending_worktree.take();
        let sink = self.sink();
        let app_event_tx = self.app_event_tx.clone();
        self.agent.runtime().handle().spawn(async move {
            let reply = match new_session(&agent, model, role).await {
                Ok(session) => {
                    if let Some(title) = title {
                        let request = AgentSessionTitleRenameRequest {
                            session_id: session.id,
                            generation: session.generation,
                            revision: session.revision,
                            title,
                        };
                        if let Err(error) = agent.rename_session(request, &*sink) {
                            let failed = format!("the session was not named: {error}");
                            app_event_tx.send(AppEvent::AgentReply(AgentReply::Failed(failed)));
                        }
                    }
                    if let Some(path) = worktree {
                        if let Err(error) = agent.worktree_bind(path.clone(), session.id).await {
                            let failed = format!(
                                "the session was not bound to the worktree {}: {error}",
                                path.display()
                            );
                            app_event_tx.send(AppEvent::AgentReply(AgentReply::Failed(failed)));
                        }
                    }
                    AgentReply::SessionCreated(session.id)
                }
                Err(message) => AgentReply::SessionNotCreated(message),
            };
            app_event_tx.send(AppEvent::AgentReply(reply));
        });
    }

    /// `session_id` with its latest transcript, for the chat to replay.
    pub(crate) fn open(&self, session_id: AgentSessionId) -> Result<OpenedSession, String> {
        self.open_with_snapshot(session_id)
            .map(|(opened, _)| opened)
    }

    /// [`Self::open`], with the session's state as it was read with the
    /// transcript.
    pub(crate) fn open_with_snapshot(
        &self,
        session_id: AgentSessionId,
    ) -> Result<(OpenedSession, AgentSessionRuntimeSnapshot), String> {
        let opened = self
            .agent
            .open_session(session_id, self.page_size)
            .map_err(|error| format!("the session was not opened: {error}"))?;
        let items = opened
            .transcript
            .items
            .into_iter()
            .map(|item| (item.item_id.value(), item.content))
            .collect();
        let session = OpenedSession {
            session_id: Some(session_id),
            model: opened.runtime.model.clone(),
            mode: opened.runtime.mode,
            transcript: replay::transcript(&self.agent, items),
            setup_needed: false,
            older: opened
                .transcript
                .older_cursor
                .map(|cursor| OlderTranscript {
                    session_id,
                    cursor,
                    through_revision: opened.through_revision,
                }),
        };
        Ok((session, opened.runtime))
    }

    /// The page of the transcript before `older`, oldest first, and where
    /// the page before it begins (`None` at the session's start).
    pub(crate) fn read_older_page(
        &self,
        older: &OlderTranscript,
    ) -> Result<(Vec<ReplayedItem>, Option<OlderTranscript>), String> {
        let page = self
            .agent
            .read_transcript_page(AgentTranscriptPageRequest {
                session_id: older.session_id,
                page_size: self.page_size,
                cursor: Some(older.cursor.clone()),
                direction: AgentHistorySortDirection::Descending,
                through_revision: Some(older.through_revision),
            })
            .map_err(|error| format!("earlier messages were not read: {error}"))?;
        let items = page
            .items
            .into_iter()
            .map(|item| (item.item_id.value(), item.content))
            .collect();
        let next = page.older_cursor.map(|cursor| OlderTranscript {
            cursor,
            ..older.clone()
        });
        Ok((replay::transcript(&self.agent, items), next))
    }

    /// The session as it is now, for a call that names its generation and
    /// revision.
    pub(super) fn current(&self) -> Result<AgentSessionRuntimeSnapshot, String> {
        let session_id = self
            .session_id
            .ok_or_else(|| "there is no session yet".to_string())?;
        self.agent
            .snapshot(session_id)
            .map_err(|error| error.to_string())
    }
}

/// No session: its header names the model a new session gets, `-m`'s or
/// the model policy's (empty when the configuration cannot be read).
pub(super) async fn no_session(agent: &AgentHandle, model: Option<String>) -> OpenedSession {
    let registry = agent.provider_registry_snapshot().await.ok();
    let setup_needed = registry
        .as_ref()
        .is_some_and(|registry| setup_needed(registry, model.is_some()));
    let model = match model {
        Some(model) => model,
        None => registry
            .map(|registry| registry.model_policy.model)
            .unwrap_or_default(),
    };
    OpenedSession {
        session_id: None,
        model,
        mode: AgentSessionMode::Default,
        transcript: Vec::new(),
        older: None,
        setup_needed,
    }
}

/// A new session in the default mode, with `model` and `role` when they
/// are given.
async fn new_session(
    agent: &AgentHandle,
    model: Option<String>,
    role: Option<String>,
) -> Result<AgentSessionRuntimeSnapshot, String> {
    let session = agent
        .create_session(AgentSessionMode::Default, role)
        .await
        .map_err(|error| format!("the session was not created: {error}"))?;
    match model {
        Some(model) => with_model(agent, session, model).await,
        None => Ok(session),
    }
}

/// `session` switched to `model`, on its Provider.
pub(crate) async fn with_model(
    agent: &AgentHandle,
    session: AgentSessionRuntimeSnapshot,
    model: String,
) -> Result<AgentSessionRuntimeSnapshot, String> {
    agent
        .set_session_model(AgentSessionModelSelectionRequest {
            session_id: session.id,
            generation: session.generation,
            revision: session.revision,
            provider_id: session.provider_id,
            model,
        })
        .await
        .map_err(|error| format!("the model was not set: {error}"))
}
