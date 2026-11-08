//! The world-state contributors: each plugin registers one
//! `ContextContributor`, and the renderers of the kinds it owns, and produces
//! the sections of the kinds it declares when a turn is admitted. What the
//! host submitted with the turn reaches them in the request's turn
//! extensions, which only the host's ports read.

mod workspace;

use std::sync::Arc;

use hanafubuki_agent_contracts::{
    ContextContributor, ContextFuture, ContextRegistry, ContextRenderer, ContextRendererRegistry,
    ContextRequest, ContextRole, ContextSection, ContextSectionKind, ContextSlot, Register,
    WORLD_STATE_PREAMBLE,
};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use kardia_core::{Context, Inject, PluginDef, Service};

pub(crate) use workspace::workspace_context_plugin;

/// Registers the contributor of the turn's permission mode.
pub(crate) fn permission_mode_plugin() -> PluginDef {
    PluginDef::new(|ctx: Context, _| async move {
        let registry = ctx.get::<ContextRegistry>()?;
        registry.register(Arc::new(PermissionModeContext))?;
        ctx.get::<ContextRendererRegistry>()?
            .register(world_state_renderer(ContextSectionKind::PERMISSION_MODE))?;
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
        static KINDS: [ContextSectionKind; 1] = [ContextSectionKind::PERMISSION_MODE];
        &KINDS
    }

    fn contribute<'a>(
        &'a self,
        request: &'a ContextRequest,
        _cancellation: CancellationToken,
    ) -> ContextFuture<'a> {
        Box::pin(async move {
            Ok(vec![ContextSection::from_json(
                ContextSectionKind::PERMISSION_MODE,
                &serde_json::json!({ "mode": request.permission_mode }),
            )])
        })
    }
}

#[cfg(test)]
mod tests;
