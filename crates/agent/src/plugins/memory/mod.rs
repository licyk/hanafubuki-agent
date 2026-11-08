//! The `memory` plugin: the memory context and policy of every sampling
//! step, the Agent memory tools, planned per sampling step from the memory
//! settings as they stand at that step, and the observer that keeps the
//! memory store in step with what the kernel commits.

mod observer;
mod prompt;
mod tools;

use std::sync::Arc;

use hanafubuki_agent_contracts::{
    AgentToolError, AgentToolSource, ContextRendererRegistry, ObserverRegistry,
    PromptContributorRegistry, Register, ToolFuture, ToolSourceContribution, ToolSourceRegistry,
    ToolSourceStep,
};
use kardia_core::{Context, Inject, PluginDef, Service};

use self::observer::MemoryObserver;
use self::prompt::{MemoryPrompt, MEMORY_RENDERER};
#[cfg(test)]
pub(crate) use self::tools::memory_tools;
use crate::AgentMemoryApplication;

/// Registers the memory prompt contributor and its renderer, the memory tool
/// source and the memory observer over `memory`.
pub(crate) fn memory_plugin(memory: AgentMemoryApplication) -> PluginDef {
    PluginDef::new(move |ctx: Context, _| {
        let memory = memory.clone();
        async move {
            let prompts = ctx.get::<PromptContributorRegistry>()?;
            prompts.register(Arc::new(MemoryPrompt {
                memory: memory.clone(),
            }))?;
            ctx.get::<ContextRendererRegistry>()?
                .register(MEMORY_RENDERER)?;
            let sources = ctx.get::<ToolSourceRegistry>()?;
            sources.register(Arc::new(MemoryToolSource {
                memory: memory.clone(),
            }))?;
            let observers = ctx.get::<ObserverRegistry>()?;
            observers.register(Arc::new(MemoryObserver { memory }))?;
            Ok(())
        }
    })
    .name("memory")
    .inject(
        Inject::new()
            .with(PromptContributorRegistry::NAME)
            .with(ContextRendererRegistry::NAME)
            .with(ToolSourceRegistry::NAME)
            .with(ObserverRegistry::NAME),
    )
}

/// Offers the read tools while retrieval is enabled and the note tool while
/// generation is, both read from the settings for each step.
struct MemoryToolSource {
    memory: AgentMemoryApplication,
}

impl AgentToolSource for MemoryToolSource {
    fn id(&self) -> &str {
        "memory"
    }

    fn contribute<'a>(
        &'a self,
        step: &'a ToolSourceStep,
    ) -> ToolFuture<'a, Result<ToolSourceContribution, AgentToolError>> {
        Box::pin(async move {
            if step.cancellation.is_cancelled() {
                return Err(AgentToolError::Cancelled);
            }
            let read_enabled = self
                .memory
                .enabled_for_use()
                .map_err(AgentToolError::Failed)?;
            let generation_enabled = self
                .memory
                .enabled_for_generation()
                .map_err(AgentToolError::Failed)?;
            tools::memory_tools(self.memory.clone(), read_enabled, generation_enabled, step)
                .map(ToolSourceContribution::from_tools)
        })
    }
}
