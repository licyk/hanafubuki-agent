//! A host's context, which the kernel carries without knowing its kind.

use super::*;

/// A host's context kind: the kernel knows none, so a test's host plugin
/// registers its renderer ([`host_context_renderer`]).
pub(crate) const HOST_LOCATION: ContextSectionKind = ContextSectionKind::fixed("host_location");

/// The plugin a host registers its context's renderer with.
pub(crate) fn host_context_renderer() -> PluginDef {
    registering::<ContextRendererRegistry>(
        "host-context",
        vec![ContextRenderer {
            kind: HOST_LOCATION,
            slot: ContextSlot::WorldState,
            role: ContextRole::User,
            preamble: "[Host location.]\n",
            epilogue: "",
            versioned: false,
        }],
    )
}

/// A host context as [`host_context_renderer`] shows it.
pub(crate) fn host_context_fragment(content: &str) -> ModelInputItem {
    ModelInputItem::ContextFragment {
        role: ModelContextRole::User,
        kind: HOST_LOCATION.as_str().to_string(),
        content: format!("[Host location.]\n{content}"),
    }
}
