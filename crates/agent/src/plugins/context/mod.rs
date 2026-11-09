//! The world-state contributors: each plugin registers one
//! `ContextContributor`, and the renderers of the kinds it owns, and produces
//! the sections of the kinds it declares when a turn is admitted. What the
//! host submitted with the turn reaches them in the request's turn
//! extensions, which only the host's ports read.

mod sandbox;
mod workspace;

use std::sync::Arc;

use hanafubuki_agent_contracts::{
    ContextContributor, ContextFuture, ContextRegistry, ContextRenderer, ContextRendererRegistry,
    ContextRequest, ContextRole, ContextSection, ContextSectionKind, ContextSlot, Register,
    WORLD_STATE_PREAMBLE,
};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_sandbox::SandboxPolicy;
use kardia_core::{Context, Inject, PluginDef, Service};

pub(crate) use workspace::workspace_context_plugin;

/// Registers the contributor of the turn's permission mode and sandbox.
pub(crate) fn permission_mode_plugin() -> PluginDef {
    PluginDef::new(|ctx: Context, _| async move {
        let registry = ctx.get::<ContextRegistry>()?;
        registry.register(Arc::new(PermissionModeContext))?;
        let renderers = ctx.get::<ContextRendererRegistry>()?;
        renderers.register(world_state_renderer(ContextSectionKind::PERMISSION_MODE))?;
        renderers.register(world_state_renderer(ContextSectionKind::SANDBOX))?;
        Ok(())
    })
    .name("permission-mode")
    .inject(
        Inject::new()
            .with(ContextRegistry::NAME)
            .with(ContextRendererRegistry::NAME),
    )
}

/// A section of the application's state, shown with its kind and version.
fn world_state_renderer(kind: ContextSectionKind) -> ContextRenderer {
    ContextRenderer {
        kind,
        slot: ContextSlot::WorldState,
        role: ContextRole::Developer,
        preamble: WORLD_STATE_PREAMBLE,
        epilogue: "",
        versioned: true,
    }
}

struct PermissionModeContext;

impl ContextContributor for PermissionModeContext {
    fn id(&self) -> &str {
        "permission-mode"
    }

    fn kinds(&self) -> &[ContextSectionKind] {
        static KINDS: [ContextSectionKind; 2] = [
            ContextSectionKind::PERMISSION_MODE,
            ContextSectionKind::SANDBOX,
        ];
        &KINDS
    }

    fn contribute<'a>(
        &'a self,
        request: &'a ContextRequest,
        _cancellation: CancellationToken,
    ) -> ContextFuture<'a> {
        Box::pin(async move {
            let mut sections = vec![ContextSection::from_json(
                ContextSectionKind::PERMISSION_MODE,
                &serde_json::json!({ "mode": request.permission_mode }),
            )];
            // The facade adds the turn's sandbox (`collect_world_state`).
            if let Some(policy) = request.turn.get::<SandboxPolicy>() {
                sections.push(sandbox::section(policy, request.permission_mode));
            }
            Ok(sections)
        })
    }
}

#[cfg(test)]
mod tests;
