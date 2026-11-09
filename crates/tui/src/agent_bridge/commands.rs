//! The slash commands that ask something of the Agent, each onto the
//! facade's calls: a new or a reopened session, compaction, the model and
//! the mode of the session, and what `/status`, `/mcp` and `/skills` report.
//! A call that waits runs on the Agent's runtime; one that does not runs
//! here.

use std::path::PathBuf;

use hanafubuki_agent::{
    AgentPermissionMode, AgentProviderModelCatalogRequest, AgentProviderModelCatalogResult,
    AgentSessionId, AgentSessionListRequest, AgentSessionMode, AgentSessionModelSelectionRequest,
    AgentSessionPage, AgentSessionRuntimeSnapshot,
};

use super::{replay, AgentBridge, AgentReply, OpenedSession, TurnUpdate};
use crate::app_event::AgentCommand;

/// How many of a reopened session's latest transcript items are replayed.
const REPLAYED_ITEMS: usize = 200;

/// What `/model` offers: each Provider's models, and the session's own.
#[derive(Debug)]
pub(crate) struct ModelChoices {
    pub(crate) provider_id: String,
    pub(crate) model: String,
    pub(crate) catalogs: Vec<AgentProviderModelCatalogResult>,
}

/// What `/status` shows.
#[derive(Debug)]
pub(crate) struct StatusReport {
    pub(crate) session: AgentSessionRuntimeSnapshot,
    /// The session's Provider, by its name.
    pub(crate) provider: String,
    pub(crate) permission_mode: AgentPermissionMode,
    pub(crate) roots: Vec<PathBuf>,
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
            AgentCommand::NewSession => self.spawn(|agent| async move {
                let session = agent
                    .create_session(AgentSessionMode::Default)
                    .await
                    .map_err(|error| format!("the session was not created: {error}"))?;
                Ok(Some(AgentReply::SessionOpened(OpenedSession {
                    session_id: session.id,
                    model: session.model,
                    mode: session.mode,
                    transcript: Vec::new(),
                    earlier_items: false,
                })))
            }),
            AgentCommand::Resume(session_id) => {
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
                        .map(|outcome| Some(AgentReply::Update(TurnUpdate::Compacted(outcome))))
                        .map_err(|error| format!("the conversation was not compacted: {error}"))
                })
            }
            AgentCommand::ListModels => {
                let session = self.current()?;
                self.spawn(move |agent| async move {
                    let catalogs = agent
                        .list_provider_models(AgentProviderModelCatalogRequest::default())
                        .await
                        .map_err(|error| format!("the models were not listed: {error}"))?;
                    Ok(Some(AgentReply::Update(TurnUpdate::ModelChoices(
                        ModelChoices {
                            provider_id: session.provider_id,
                            model: session.model,
                            catalogs,
                        },
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
                    Ok(Some(AgentReply::Update(TurnUpdate::ModelChanged(
                        session.model,
                    ))))
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
                    let settings = agent.settings_view().await?;
                    let registry = agent.provider_registry_snapshot().await?;
                    let roots = agent.workspace_roots()?.into_vec();
                    let provider = registry
                        .providers
                        .iter()
                        .find(|provider| provider.id == session.provider_id)
                        .map_or_else(
                            || session.provider_id.clone(),
                            |provider| provider.display_name.clone(),
                        );
                    Ok(Some(AgentReply::Update(TurnUpdate::StatusReport(
                        StatusReport {
                            session,
                            provider,
                            permission_mode: settings.settings.permission_mode,
                            roots,
                        },
                    ))))
                })
            }
            AgentCommand::Mcp => self.spawn(|agent| async move {
                let snapshot = agent.mcp_snapshot().await?;
                Ok(Some(AgentReply::Update(TurnUpdate::Mcp(snapshot))))
            }),
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

    /// The workspace roots, the first the one relative paths are read
    /// against.
    pub(crate) fn workspace_roots(&self) -> Vec<PathBuf> {
        self.agent
            .workspace_roots()
            .map(|roots| roots.into_vec())
            .unwrap_or_default()
    }

    /// `session_id` with its latest transcript, for the chat to replay.
    pub(crate) fn open(&self, session_id: AgentSessionId) -> Result<OpenedSession, String> {
        let opened = self
            .agent
            .open_session(session_id, REPLAYED_ITEMS)
            .map_err(|error| format!("the session was not opened: {error}"))?;
        let items = opened
            .transcript
            .items
            .into_iter()
            .map(|item| item.content)
            .collect();
        Ok(OpenedSession {
            session_id,
            model: opened.runtime.model,
            mode: opened.runtime.mode,
            transcript: replay::transcript(&self.agent, items),
            earlier_items: opened.transcript.older_cursor.is_some(),
        })
    }

    /// The session as it is now, for a call that names its generation and
    /// revision.
    fn current(&self) -> Result<AgentSessionRuntimeSnapshot, String> {
        self.agent
            .snapshot(self.session_id)
            .map_err(|error| error.to_string())
    }
}
