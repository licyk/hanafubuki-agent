//! A tool-free second opinion on the current conversation.
mod options;
mod request;
mod tool;

pub(crate) use options::advisor_schema;
pub use request::AdvisorAnswer;

use std::collections::HashSet;
use std::sync::Arc;

use hanafubuki_agent_contracts::{
    BlockingService, Register, ToolPresenterRegistry, ToolSourceRegistry,
};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_kernel::{AgentError, AgentSessionId};
use kardia_core::{Context, Disposer, Inject, PluginDef, Service};
use parking_lot::RwLock;

use crate::conversation_reader::AgentConversationReader;
use options::{ActivationScope, AdvisorOptions};

pub(crate) struct AdvisorService {
    conversations: AgentConversationReader,
    blocking: Arc<BlockingService>,
    options: RwLock<Arc<AdvisorOptions>>,
    sessions: RwLock<HashSet<AgentSessionId>>,
    cancellation: CancellationToken,
}

pub(crate) fn advisor_plugin(conversations: AgentConversationReader) -> PluginDef {
    PluginDef::typed(move |ctx: Context, options: Arc<AdvisorOptions>| {
        let conversations = conversations.clone();
        async move {
            let cancellation = CancellationToken::default();
            let stopping = cancellation.clone();
            ctx.defer("advisor requests", Disposer::new(move || stopping.cancel()))?;
            let service = Arc::new(AdvisorService {
                conversations,
                blocking: ctx.get::<BlockingService>()?.arc().clone(),
                options: RwLock::new(options),
                sessions: RwLock::default(),
                cancellation,
            });
            let updated = service.clone();
            ctx.on_update(move |options: &AdvisorOptions| {
                *updated.options.write() = Arc::new(options.clone());
                true
            })?;
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
            .with(BlockingService::NAME)
            .with(ToolPresenterRegistry::NAME)
            .with(ToolSourceRegistry::NAME),
    )
}

impl AdvisorService {
    fn active(&self, id: AgentSessionId) -> bool {
        let options = self.options.read();
        options.primary().is_some()
            && match options.activation_scope {
                ActivationScope::Persistent => options.active,
                ActivationScope::Session => self.sessions.read().contains(&id),
            }
    }

    pub(crate) fn activate(&self, id: AgentSessionId, active: bool) -> Result<bool, AgentError> {
        let options = self.options.read();
        if active && options.primary().is_none() {
            return Err(AgentError::InvalidInput(
                "Choose an Advisor provider and model in /advisor-settings first".into(),
            ));
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
