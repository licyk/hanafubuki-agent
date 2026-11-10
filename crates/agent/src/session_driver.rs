//! The narrow port through which the application and the `subagents` and
//! `review` plugins drive child sessions: create one under a parent, name
//! it, set its mode, list a parent's children, give a child its parent's
//! permission mode, run a turn on it (in a permission mode of its own if
//! need be) and wait for its outcome, steer or interrupt that turn, read
//! its last answer, add context between its turns, and close it. It also
//! prepares every turn the facade admits
//! (the session's Provider, its permission mode, the world state and the
//! turn's extensions), so a turn a plugin runs is prepared like a user's.

use std::ops::ControlFlow;
use std::sync::Arc;

use hanafubuki_agent_config::{AgentConfig, AgentSettings};
use hanafubuki_agent_contracts::{ContextRequest, TurnAdmissionRequest, TurnExtensions};
use hanafubuki_agent_execution::{AgentExecutionScope, AgentExecutionService};
use hanafubuki_agent_foundation::blocking::{run_blocking_operation, BlockingOperationPort};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_kernel::{
    AgentBackgroundCompactionThreshold, AgentChildSessionRequest, AgentError, AgentEvent,
    AgentEventSink, AgentKernelRegistries, AgentPermissionMode, AgentService, AgentSessionId,
    AgentSessionLifecycle, AgentSessionListRequest, AgentSessionMode, AgentSessionOrigin,
    AgentSessionParentFilter, AgentSessionPersistence, AgentSessionRuntimeSnapshot,
    AgentSessionSummary, AgentSessionTitleRenameRequest, AgentTurnInput, AgentTurnOrigin,
    AgentTurnOutcome, AgentWorldStateSnapshot, ConversationItem, AGENT_SESSION_PAGE_SIZE_MAX,
};
use hanafubuki_agent_sandbox::{platform_default_mode, SandboxSettings};

use crate::conversation_reader::{AgentConversationReader, ConversationMessage};
use crate::service::resolve_settings_provider;
use crate::{
    model_api_protocol, session_compaction_strategy, warn_unreachable_background_threshold,
};

#[derive(Clone)]
pub(crate) struct AgentSessionDriver {
    core: AgentService,
    config: AgentConfig,
    registries: AgentKernelRegistries,
    executions: AgentExecutionService,
    blocking: Arc<dyn BlockingOperationPort>,
}

impl AgentSessionDriver {
    pub(crate) fn new(
        core: AgentService,
        config: AgentConfig,
        registries: AgentKernelRegistries,
        executions: AgentExecutionService,
        blocking: Arc<dyn BlockingOperationPort>,
    ) -> Self {
        Self {
            core,
            config,
            registries,
            executions,
            blocking,
        }
    }

    /// Creates the child `request` describes under its loaded parent. A
    /// durable child contributes to memory as the settings say, like a
    /// fork, and takes the configured compaction strategy; an ephemeral one
    /// never contributes and always compacts with `standard`. Blocks.
    pub(crate) fn create_child(
        &self,
        request: AgentChildSessionRequest,
    ) -> Result<AgentSessionRuntimeSnapshot, AgentError> {
        let persistence = request.persistence;
        let child = self.core.create_child_session(request)?;
        if persistence == AgentSessionPersistence::Ephemeral {
            return Ok(child);
        }
        let settings = self.config.settings().map_err(AgentError::Persistence)?;
        self.core.set_session_compaction_strategy(
            child.id,
            session_compaction_strategy(&settings.compaction, false, true),
        )?;
        self.core.initialize_session_memory_contribution(
            child.id,
            child.memory_contribution.revision,
            settings.memory.enabled && settings.memory.generate_memories,
        )?;
        self.core.snapshot(child.id)
    }

    /// Runs a turn of `text` on `session_id` and waits for its outcome. The
    /// turn runs in `permission_mode`, else in the session's.
    pub(crate) async fn run_turn(
        &self,
        session_id: AgentSessionId,
        text: String,
        permission_mode: Option<AgentPermissionMode>,
    ) -> Result<AgentTurnOutcome, AgentError> {
        let session = self
            .activate_provider_session(session_id)
            .await
            .map_err(AgentError::InvalidInput)?;
        let permission_mode = match permission_mode {
            Some(mode) => mode,
            None => self
                .turn_permission_mode(session_id)
                .await
                .map_err(AgentError::InvalidInput)?,
        };
        let world_state = self
            .collect_world_state(
                session_id,
                session.mode,
                permission_mode,
                Some(text.clone()),
                TurnExtensions::default(),
            )
            .await?;
        let turn = self
            .collect_turn_extensions(session_id, session.mode)
            .await?;
        self.core
            .start_turn(
                session_id,
                AgentTurnInput {
                    permission_mode,
                    turn,
                    world_state,
                    origin: AgentTurnOrigin::Programmatic,
                    ..AgentTurnInput::from(text)
                },
            )
            .await
    }

    /// Cancels the turn `session_id` runs, if any, and waits until the
    /// cancellation is confirmed.
    pub(crate) async fn interrupt(&self, session_id: AgentSessionId) -> Result<(), AgentError> {
        let Some(turn) = self
            .core
            .snapshot(session_id)?
            .turn
            .filter(|turn| !turn.phase.is_terminal())
        else {
            return Ok(());
        };
        self.core
            .cancel_turn_confirmed(session_id, turn.id, turn.generation)
            .await
            .map(|_| ())
    }

    /// The text of the latest answer of `session_id`'s model, if it has
    /// answered. Blocks.
    pub(crate) fn last_answer(
        &self,
        session_id: AgentSessionId,
    ) -> Result<Option<String>, AgentError> {
        let mut answer = None;
        AgentConversationReader::new(self.core.clone(), self.config.clone()).visit_messages(
            session_id,
            |message| match message {
                ConversationMessage::Assistant(text) => {
                    answer = Some(text);
                    ControlFlow::Break(())
                }
                ConversationMessage::User(_) => ControlFlow::Continue(()),
            },
        )?;
        Ok(answer)
    }

    /// Adds `text` to the turn `session_id` runs, which its model reads at
    /// the turn's next message boundary. Fails when no turn runs. Blocks.
    pub(crate) fn steer(&self, session_id: AgentSessionId, text: String) -> Result<(), AgentError> {
        let turn = self
            .core
            .snapshot(session_id)?
            .turn
            .filter(|turn| !turn.phase.is_terminal())
            .ok_or(AgentError::TurnNotRunning)?;
        self.core
            .steer_turn_with_sink(session_id, turn.id, turn.generation, text, &Discard)
            .map(|_| ())
    }

    /// The session's current state, loading it first. Blocks.
    pub(crate) fn snapshot(
        &self,
        session_id: AgentSessionId,
    ) -> Result<AgentSessionRuntimeSnapshot, AgentError> {
        self.core.activate_session(session_id)?;
        self.core.snapshot(session_id)
    }

    /// Names `session_id` `title`, as a user's rename would. Blocks.
    pub(crate) fn rename(
        &self,
        session_id: AgentSessionId,
        title: String,
    ) -> Result<(), AgentError> {
        let current = self.core.snapshot(session_id)?;
        self.core
            .rename_session(
                AgentSessionTitleRenameRequest {
                    session_id,
                    generation: current.generation,
                    revision: current.revision,
                    title,
                },
                &Discard,
            )
            .map(|_| ())
    }

    /// Puts `session_id` in `mode`, unless it is in it. Blocks.
    pub(crate) fn set_mode(
        &self,
        session_id: AgentSessionId,
        mode: AgentSessionMode,
    ) -> Result<(), AgentError> {
        let current = self.core.snapshot(session_id)?;
        if current.mode == mode {
            return Ok(());
        }
        self.core
            .set_session_mode(session_id, current.generation, current.revision, mode)
            .map(|_| ())
    }

    /// Whether `session_id` has had a turn, so context can follow its
    /// history. Blocks.
    pub(crate) fn has_history(&self, session_id: AgentSessionId) -> Result<bool, AgentError> {
        Ok(!self
            .core
            .open_session(session_id, 1)?
            .transcript
            .items
            .is_empty())
    }

    /// Commits `items`, continuation context only, to `session_id` between
    /// turns. Blocks.
    pub(crate) fn add_context(
        &self,
        session_id: AgentSessionId,
        items: Vec<ConversationItem>,
    ) -> Result<(), AgentError> {
        self.core.add_continuation_context(session_id, items)
    }

    /// Gives `child` the permission mode `parent`'s next turn would run
    /// with, and the sandbox mode `parent` was given if any (else the child
    /// takes the configured one, as `parent` does), until the child ends or
    /// is given others.
    pub(crate) async fn inherit_permission_mode(
        &self,
        parent: AgentSessionId,
        child: AgentSessionId,
    ) -> Result<(), AgentError> {
        let mode = self
            .turn_permission_mode(parent)
            .await
            .map_err(AgentError::InvalidInput)?;
        let sandbox_mode = self.core.session_sandbox_mode(parent)?;
        let core = self.core.clone();
        run_blocking_operation(
            self.blocking.as_ref(),
            "agent child permission mode worker",
            move || {
                if let Some(sandbox_mode) = sandbox_mode {
                    core.set_session_sandbox_mode(child, sandbox_mode)?;
                }
                core.set_session_permission_mode(child, mode)
            },
        )
        .await
        .map_err(AgentError::Persistence)?
    }

    /// The children `origin` made under `parent` that are not archived,
    /// oldest first. Blocks.
    pub(crate) fn children(
        &self,
        parent: AgentSessionId,
        origin: AgentSessionOrigin,
    ) -> Result<Vec<AgentSessionSummary>, AgentError> {
        let mut children = Vec::new();
        let mut cursor = None;
        loop {
            let page = self.core.list_sessions_page(AgentSessionListRequest {
                page_size: AGENT_SESSION_PAGE_SIZE_MAX,
                cursor,
                lifecycle: AgentSessionLifecycle::Available,
                parent: AgentSessionParentFilter::ChildrenOf(parent),
                ..AgentSessionListRequest::default()
            })?;
            children.extend(
                page.sessions
                    .into_iter()
                    .filter(|child| child.origin == Some(origin)),
            );
            let Some(next) = page.next_cursor else {
                break;
            };
            cursor = Some(next);
        }
        children.sort_by_key(|child| (child.created_at, child.session_id));
        Ok(children)
    }

    /// Interrupts `session_id`, then lets it go ([`Self::release`]).
    pub(crate) async fn close(&self, session_id: AgentSessionId) -> Result<(), AgentError> {
        self.interrupt(session_id).await?;
        let driver = self.clone();
        run_blocking_operation(
            self.blocking.as_ref(),
            "agent child session close worker",
            move || driver.release(session_id),
        )
        .await
        .map_err(AgentError::Persistence)?
    }

    /// Releases the live ephemeral children of `session_id` first, then
    /// stops its background terminals and, if it is ephemeral, drops it
    /// (cancelling what it still runs). A durable session stays in the
    /// history. A terminal that does not stop keeps the session open.
    /// Blocks.
    pub(crate) fn release(&self, session_id: AgentSessionId) -> Result<(), AgentError> {
        for child in self.core.ephemeral_children(session_id, None) {
            self.release(child)?;
        }
        let scope = AgentExecutionScope::for_agent_session(session_id.value())
            .map_err(|error| AgentError::InvalidInput(error.to_string()))?;
        if !self
            .executions
            .stop_background_terminals_confirmed(&scope)
            .retained
            .is_empty()
        {
            return Err(AgentError::SessionMutationNotConfirmed);
        }
        if self.core.snapshot(session_id)?.ephemeral {
            self.core.close_ephemeral_session(session_id)?;
        }
        Ok(())
    }

    /// Loads `session_id` and binds it to its Provider and its compaction
    /// strategy as the settings now describe them, for a model operation.
    /// The kernel calls load history and wait for the session's staged
    /// commit, so they run on the blocking boundary.
    pub(crate) async fn activate_provider_session(
        &self,
        session_id: AgentSessionId,
    ) -> Result<AgentSessionRuntimeSnapshot, String> {
        let core = self.core.clone();
        let snapshot = run_blocking_operation(
            self.blocking.as_ref(),
            "agent session activation worker",
            move || {
                core.activate_session(session_id)?;
                core.snapshot(session_id)
            },
        )
        .await?
        .map_err(|error| error.to_string())?;
        if snapshot.model.is_empty() {
            return Err(
                "No model is selected for this conversation; use /model or choose one in Agent settings"
                    .to_string(),
            );
        }
        let config = self.config.clone();
        let (registry, settings) = run_blocking_operation(
            self.blocking.as_ref(),
            "agent provider settings read worker",
            move || Ok::<_, String>((config.providers()?, config.settings()?)),
        )
        .await??;
        let resolved = resolve_settings_provider(&registry, &snapshot.provider_id)?;
        let strategy = session_compaction_strategy(
            &settings.compaction,
            snapshot.ephemeral,
            snapshot.parent_session_id.is_some(),
        );
        let auto_compact_threshold_percent = resolved.policy.auto_compact_threshold_percent;
        let protocol = model_api_protocol(resolved.definition.protocol);
        if protocol != snapshot.provider_protocol {
            return Err(format!(
                "Agent Provider {} protocol changed; select a model explicitly before continuing",
                resolved.definition.display_name
            ));
        }
        let core = self.core.clone();
        let activated = run_blocking_operation(
            self.blocking.as_ref(),
            "agent session activation worker",
            move || {
                core.activate_model_session(
                    protocol,
                    &resolved.definition.base_url,
                    resolved.credential,
                    resolved.headers,
                    hanafubuki_agent_kernel::ModelRetryPolicy::default(),
                    resolved.policy.compaction_capability,
                    session_id,
                )?;
                let changed = core.set_session_compaction_strategy(session_id, strategy)?;
                Ok::<_, AgentError>((changed, core.snapshot(session_id)?))
            },
        )
        .await?
        .map_err(|error| error.to_string())?;
        let (changed, snapshot) = activated;
        if changed {
            warn_unreachable_background_threshold(
                strategy,
                &settings.compaction,
                auto_compact_threshold_percent,
            );
        }
        Ok(snapshot)
    }

    /// The permission mode of the session's next turn: the session's choice,
    /// else the configured one.
    pub(crate) async fn turn_permission_mode(
        &self,
        session_id: AgentSessionId,
    ) -> Result<AgentPermissionMode, String> {
        if let Some(mode) = self
            .core
            .session_permission_mode(session_id)
            .map_err(|error| error.to_string())?
        {
            return Ok(mode);
        }
        let config = self.config.clone();
        Ok(run_blocking_operation(
            self.blocking.as_ref(),
            "agent settings read worker",
            move || config.settings(),
        )
        .await??
        .permission_mode)
    }

    /// The turn's world state, from the context contributors plugins
    /// registered. `user_message` is the text the turn was submitted with,
    /// `host` what the host submitted with it, to which the turn's
    /// `SandboxPolicy` and what its sandbox enforces of it are added.
    pub(crate) async fn collect_world_state(
        &self,
        session_id: AgentSessionId,
        mode: AgentSessionMode,
        permission_mode: AgentPermissionMode,
        user_message: Option<String>,
        mut host: TurnExtensions,
    ) -> Result<AgentWorldStateSnapshot, AgentError> {
        // The `sandbox` section tells the model what the turn's commands
        // may reach.
        let sandbox = self.turn_sandbox(session_id).await?;
        let enforcement = self
            .executions
            .sandbox()
            .with_windows_sandbox(sandbox.windows_sandbox)
            .enforcement();
        host.insert(Arc::new(sandbox.policy(&self.executions.workspace_roots())));
        host.insert(Arc::new(enforcement));
        let request = ContextRequest {
            session_id,
            mode,
            permission_mode,
            user_message,
            turn: host,
        };
        hanafubuki_agent_kernel::collect_world_state(
            &self.registries.context,
            &request,
            &admission_cancellation(),
            self.core.limits().plugin_call_deadline,
        )
        .await
    }

    /// The sandbox of the session's next turn's commands: the session's
    /// mode, else the configured `sandboxMode`, else the platform's
    /// default, with the configured `[sandboxWorkspaceWrite]` and
    /// `[windows] sandbox`.
    pub(crate) async fn turn_sandbox(
        &self,
        session_id: AgentSessionId,
    ) -> Result<SandboxSettings, AgentError> {
        let settings = self.read_settings().await?;
        self.sandbox_settings(session_id, settings)
    }

    fn sandbox_settings(
        &self,
        session_id: AgentSessionId,
        settings: AgentSettings,
    ) -> Result<SandboxSettings, AgentError> {
        let session_mode = self.core.session_sandbox_mode(session_id)?;
        Ok(SandboxSettings {
            mode: session_mode
                .or(settings.sandbox_mode)
                .unwrap_or_else(platform_default_mode),
            writable_roots: settings.sandbox_workspace_write.writable_roots,
            network_access: settings.sandbox_workspace_write.network_access,
            windows_sandbox: settings.windows.sandbox,
        })
    }

    async fn read_settings(&self) -> Result<AgentSettings, AgentError> {
        let config = self.config.clone();
        run_blocking_operation(
            self.blocking.as_ref(),
            "agent settings read worker",
            move || config.settings(),
        )
        .await
        .map_err(AgentError::Persistence)?
        .map_err(AgentError::Persistence)
    }

    /// The turn's extensions: what the turn admission contributors plugins
    /// registered return (the `skills` plugin freezes the turn's Skill
    /// catalog), the sandbox its commands run in, the threshold of
    /// `[compaction.background]` and the tuning of
    /// `[compaction.hierarchical]`.
    pub(crate) async fn collect_turn_extensions(
        &self,
        session_id: AgentSessionId,
        mode: AgentSessionMode,
    ) -> Result<TurnExtensions, AgentError> {
        let settings = self.read_settings().await?;
        let background = AgentBackgroundCompactionThreshold {
            percent: settings.compaction.background.threshold_percent,
        };
        let hierarchical = hierarchical_compression(&settings.compaction.hierarchical);
        let sandbox = self.sandbox_settings(session_id, settings)?;
        let mut turn = hanafubuki_agent_kernel::collect_turn_extensions(
            &self.registries.turn_admission,
            &TurnAdmissionRequest { session_id, mode },
            &admission_cancellation(),
            self.core.limits().plugin_call_deadline,
        )
        .await?;
        turn.insert(Arc::new(sandbox));
        turn.insert(Arc::new(background));
        turn.insert(Arc::new(hierarchical));
        Ok(turn)
    }
}

/// The cancellation of one admission's plugin calls. No one cancels an
/// admission still being prepared: the caller drops it, which cancels the
/// call in flight.
fn admission_cancellation() -> CancellationToken {
    CancellationToken::default()
}

/// `[compaction.hierarchical]` for a turn admitted now, with the local
/// time's offset from UTC now (what the turn's new message stamps keep).
pub(crate) fn hierarchical_compression(
    settings: &hanafubuki_agent_config::AgentHierarchicalCompactionSettings,
) -> hanafubuki_agent_kernel::AgentHierarchicalCompression {
    hanafubuki_agent_kernel::AgentHierarchicalCompression {
        max_context_limit_percent: settings.max_context_limit_percent,
        emergency_threshold_percent: settings.emergency_threshold_percent,
        nudge_growth_tokens: settings.nudge_growth_tokens,
        preserve_recent_messages: settings.preserve_recent_messages,
        preserve_recent_tokens: settings.preserve_recent_tokens,
        min_compress_range_chars: settings.min_compress_range_chars,
        min_summary_length: settings.min_summary_length,
        max_summary_length: settings.max_summary_length,
        tiers: settings.tiers,
        protected_tools: settings.protected_tools.clone(),
        absorb: settings.absorb,
        content_store: settings.content_store,
        crush: settings.crush,
        rules: settings.rules,
        message_timestamps: settings.message_timestamps,
        utc_offset_seconds: chrono::Local::now().offset().local_minus_utc(),
    }
}

/// Where the events of what a plugin drives on a child go: the child's
/// journal keeps them, which is where an interface reads them.
struct Discard;

impl AgentEventSink for Discard {
    fn emit(&self, _event: AgentEvent) {}
}
