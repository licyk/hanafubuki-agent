//! A tool-free second opinion on the current conversation.
mod configuration;
pub use configuration::{AdvisorProposal, AdvisorSettingDiff};
mod follow_up;
mod gate;
mod history;
mod outcomes;
pub use outcomes::{Adoption, AdvisorStats, Validation};
mod options;
mod prompt;
mod request;
mod scout;
mod tool;
mod workspace;

pub(crate) use options::advisor_schema;
pub use request::AdvisorAnswer;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
pub use workspace::{Consultation, GitContext};

use hanafubuki_agent_contracts::{
    BlockingService, Register, ToolPresenterRegistry, ToolSourceRegistry,
};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_kernel::{AgentError, AgentSessionId};
use kardia_core::{Context, Disposer, Inject, PluginDef, Service};
use parking_lot::RwLock;

use crate::conversation_reader::AgentConversationReader;
use options::{ActivationScope, AdvisorOptions};

pub(crate) struct ExecutorPin {
    pub target: Option<(String, String)>,
    pub effort: Option<hanafubuki_agent_kernel::ModelReasoningEffort>,
}

pub(crate) struct AdvisorService {
    conversations: AgentConversationReader,
    workspace: workspace::AdvisorWorkspace,
    blocking: Arc<BlockingService>,
    options: RwLock<Arc<AdvisorOptions>>,
    proposals: parking_lot::Mutex<HashMap<AgentSessionId, AdvisorProposal>>,
    repeats: parking_lot::Mutex<HashMap<AgentSessionId, gate::Repeated>>,
    ledger: Arc<outcomes::Ledger>,
    consultations: parking_lot::Mutex<HashMap<AgentSessionId, usize>>,
    previous: parking_lot::Mutex<HashMap<AgentSessionId, follow_up::Previous>>,
    sessions: RwLock<HashSet<AgentSessionId>>,
    cancellation: CancellationToken,
}

pub(crate) fn advisor_plugin(
    conversations: AgentConversationReader,
    git: hanafubuki_agent_worktree::GitQueries,
    files: hanafubuki_agent_workspace_fs::WorkspaceFsService,
    directory: hanafubuki_agent_plugin_hooks::WorkingDirectory,
    user_directory: std::path::PathBuf,
) -> PluginDef {
    let workspace = workspace::AdvisorWorkspace {
        git,
        files,
        directory,
    };
    PluginDef::typed(move |ctx: Context, options: Arc<AdvisorOptions>| {
        let ledger = Arc::new(outcomes::Ledger::new(
            user_directory.join("advisor-outcomes.jsonl"),
        ));
        let workspace = workspace.clone();
        let conversations = conversations.clone();
        async move {
            let cancellation = CancellationToken::default();
            let stopping = cancellation.clone();
            ctx.defer("advisor requests", Disposer::new(move || stopping.cancel()))?;
            let service = Arc::new(AdvisorService {
                proposals: Default::default(),
                repeats: Default::default(),
                ledger,
                conversations,
                workspace,
                blocking: ctx.get::<BlockingService>()?.arc().clone(),
                options: RwLock::new(options),
                sessions: RwLock::default(),
                consultations: Default::default(),
                previous: Default::default(),
                cancellation,
            });
            let updated = service.clone();
            ctx.on_update(move |options: &AdvisorOptions| {
                updated.proposals.lock().clear();
                updated.repeats.lock().clear();
                updated.previous.lock().clear();
                *updated.options.write() = Arc::new(options.clone());
                true
            })?;
            ctx.get::<hanafubuki_agent_contracts::PromptContributorRegistry>()?
                .register(Arc::new(prompt::Guidance(service.clone())))?;
            ctx.get::<hanafubuki_agent_contracts::ContextRendererRegistry>()?
                .register(prompt::RENDERER)?;
            ctx.get::<hanafubuki_agent_contracts::ObserverRegistry>()?
                .register(service.clone() as Arc<dyn hanafubuki_agent_contracts::AgentObserver>)?;
            gate::register(&ctx, service.clone())?;
            ctx.get::<ToolSourceRegistry>()?
                .register(Arc::new(tool::AdvisorSource::new(service.clone())?))?;
            ctx.get::<ToolPresenterRegistry>()?
                .register(Arc::new(tool::AdvisorPresenter))?;
            ctx.provide_arc_as("advisor", service)?;
            Ok(())
        }
    })
    .name("advisor")
    .provides(["advisor"])
    .inject(
        Inject::new()
            .with(hanafubuki_agent_contracts::PromptContributorRegistry::NAME)
            .with(hanafubuki_agent_contracts::ContextRendererRegistry::NAME)
            .with(hanafubuki_agent_contracts::ToolLifecycleHookRegistry::NAME)
            .with(hanafubuki_agent_contracts::ObserverRegistry::NAME)
            .with(BlockingService::NAME)
            .with(ToolPresenterRegistry::NAME)
            .with(ToolSourceRegistry::NAME),
    )
}

impl AdvisorService {
    pub(crate) async fn stats(&self) -> Result<AdvisorStats, AgentError> {
        let ledger = self.ledger.clone();
        self.blocking
            .run("advisor outcomes", move || ledger.stats())
            .await
            .map_err(|e| AgentError::InvalidInput(e.to_string()))?
            .map_err(AgentError::InvalidInput)
    }
    fn active(&self, id: AgentSessionId) -> bool {
        let options = self.options.read();
        options.primary().is_some()
            && match options.activation_scope {
                ActivationScope::Persistent => options.active,
                ActivationScope::Session => self.sessions.read().contains(&id),
            }
    }

    pub(crate) fn executor_pin(&self) -> Result<ExecutorPin, AgentError> {
        let opts = self.options.read();
        if opts.primary().is_none() {
            return Err(AgentError::InvalidInput(
                "Choose an Advisor provider and model in /advisor-settings first".into(),
            ));
        }
        let target = match (&opts.executor_provider, &opts.executor_model) {
            (Some(provider), Some(model)) => Some((provider.clone(), model.clone())),
            (None, None) => None,
            _ => {
                return Err(AgentError::InvalidInput(
                    "Choose both executor provider and model for a pinned executor".into(),
                ))
            }
        };
        Ok(ExecutorPin {
            target,
            effort: opts.executor_effort.clone(),
        })
    }
    pub(crate) fn activate(&self, id: AgentSessionId, active: bool) -> Result<bool, AgentError> {
        let options = self.options.read();
        if active && options.primary().is_none() {
            return Err(AgentError::InvalidInput(
                "Choose an Advisor provider and model in /advisor-settings first".into(),
            ));
        }
        if !active {
            self.proposals.lock().remove(&id);
            self.repeats.lock().remove(&id);
            self.previous.lock().remove(&id);
        }
        if options.activation_scope == ActivationScope::Persistent {
            return Ok(true);
        }
        if active {
            self.sessions.write().insert(id);
        } else {
            self.sessions.write().remove(&id);
        }
        Ok(false)
    }
}

impl hanafubuki_agent_contracts::AgentObserver for AdvisorService {
    fn id(&self) -> &str {
        "advisor"
    }
    fn observe(&self, event: &hanafubuki_agent_contracts::AgentNotification) {
        use hanafubuki_agent_contracts::AgentNotification;
        if let AgentNotification::SessionArchived { session_id, .. } = event {
            self.previous.lock().remove(session_id);
            self.repeats.lock().remove(session_id);
            self.proposals.lock().remove(session_id);
            self.sessions.write().remove(session_id);
        }
    }
}
