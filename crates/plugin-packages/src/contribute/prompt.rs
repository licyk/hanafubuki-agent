//! Prompt fragments of guests. A guest that declares a prompt gets a kind of
//! its own (`plugin_<name>`), whose renderer it registers in the slot it
//! declares. One contributor of the guest host prepares every guest's
//! fragment once per sampling step, asking the guests at once, each within
//! its bound, so one that does not answer costs only its own fragment; the
//! fragments are then rendered from what was prepared. One contributor,
//! not one per guest: the prepared values are keyed by type, and a second
//! value of one type would be left out.

use std::sync::Arc;

use futures_util::future::join_all;
use hanafubuki_agent_contracts::{
    CancellationToken, ContextRenderer, ContextRendererRegistry, ContextRole, ContextSectionKind,
    ContextSlot, PromptContributor, PromptFragment, PromptPrepareFuture, PromptRequest, PromptStep,
    TurnExtensions,
};
use hanafubuki_agent_packages::PluginId;
use kardia_core::Context;
use serde_json::json;

use super::context::{guest_kind, GUEST_PREAMBLE};
use super::{bounded, register_in, text_answer, GuestCaller, GuestMethod};
use crate::guests::{GuestHost, Shared};
use crate::package::GuestPackage;
use crate::state::GuestState;

/// The longest fragment a guest may give; a longer one is cut.
const MAXIMUM_FRAGMENT_BYTES: usize = 4 * 1024;
const CONTRIBUTOR_ID: &str = "plugin-guests";

/// Registers the renderer of `package`'s prompt kind, when it declares a
/// prompt, and returns the kind its fragments are prepared under.
pub(super) fn register(
    ctx: &Context,
    package: &GuestPackage,
    issues: &mut Vec<String>,
) -> Option<ContextSectionKind> {
    let declared = package.contributes.prompt_slot.as_deref()?;
    let slot = slot(declared).unwrap_or_else(|| {
        issues.push(format!(
            "its prompt's slot `{declared}` is not one of instructions, worldState, \
             capabilities or memory; capabilities is used"
        ));
        ContextSlot::Capabilities
    });
    let Some(kind) = guest_kind(&prompt_kind(&package.id)) else {
        issues.push("its prompt was left out: its name gives no prompt kind".to_string());
        return None;
    };
    let renderer = ContextRenderer {
        kind: kind.clone(),
        slot,
        role: ContextRole::Developer,
        preamble: GUEST_PREAMBLE,
        epilogue: "",
        versioned: false,
    };
    register_in::<ContextRendererRegistry>(ctx, renderer, &format!("its prompt `{kind}`"), issues)
        .then_some(kind)
}

pub(super) fn slot(declared: &str) -> Option<ContextSlot> {
    match declared {
        "instructions" => Some(ContextSlot::Instructions),
        "worldState" | "world_state" => Some(ContextSlot::WorldState),
        "capabilities" => Some(ContextSlot::Capabilities),
        "memory" => Some(ContextSlot::Memory),
        _ => None,
    }
}

/// `plugin_<name>`: the plugin's name in lowercase, with what a kind cannot
/// hold replaced by `_`, cut to a kind's length.
pub(super) fn prompt_kind(id: &PluginId) -> String {
    let mut kind = String::from("plugin_");
    kind.extend(
        id.plugin_name
            .chars()
            .map(|character| match character.to_ascii_lowercase() {
                lower @ ('a'..='z' | '0'..='9') => lower,
                _ => '_',
            }),
    );
    kind.truncate(ContextSectionKind::MAX_BYTES);
    kind
}

/// The guest host's prompt contributor.
pub(crate) struct GuestPrompts(pub(crate) Arc<Shared>);

/// The fragments prepared for a sampling step, by kind.
struct PreparedFragments(Vec<PromptFragment>);

impl GuestPrompts {
    /// The running guests with a prompt, and its kind.
    fn guests(&self) -> Vec<(GuestCaller, ContextSectionKind)> {
        let records = self.0.records.lock();
        records
            .iter()
            .filter(|(_, record)| record.state == GuestState::Running)
            .filter_map(|(id, record)| {
                let kind = record.prompt.clone()?;
                let id = PluginId::parse(id).ok()?;
                Some((GuestCaller::new(GuestHost(self.0.clone()), id), kind))
            })
            .collect()
    }
}

impl PromptContributor for GuestPrompts {
    fn id(&self) -> &str {
        CONTRIBUTOR_ID
    }

    fn rank(&self) -> u32 {
        1000
    }

    fn prepare<'a>(
        &'a self,
        step: &'a PromptStep,
        cancellation: CancellationToken,
    ) -> PromptPrepareFuture<'a> {
        Box::pin(async move {
            let args = json!({
                "sessionId": step.session_id.value(),
                "turnId": step.turn_id.value(),
                "samplingStep": step.sampling_step,
            });
            let calls = self.guests().into_iter().map(|(guest, kind)| {
                let (args, cancellation) = (args.clone(), cancellation.clone());
                async move {
                    let answer = guest
                        .call(
                            GuestMethod::Prompt,
                            args,
                            guest.kernel_bounded(),
                            &cancellation,
                        )
                        .await
                        .map_err(|error| guest.failure(GuestMethod::Prompt, error))
                        .and_then(text_answer);
                    match answer {
                        Ok(content) => content.map(|content| PromptFragment {
                            kind,
                            version: None,
                            content: bounded(content, MAXIMUM_FRAGMENT_BYTES),
                        }),
                        Err(error) => {
                            log::warn!(
                                "plugin {}: its prompt is left out of this step: {error}",
                                guest.id()
                            );
                            None
                        }
                    }
                }
            });
            let mut fragments = join_all(calls)
                .await
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            fragments.sort_by(|left, right| left.kind.cmp(&right.kind));
            Ok(TurnExtensions::default().with(Arc::new(PreparedFragments(fragments))))
        })
    }

    fn contribute(&self, request: &PromptRequest<'_>) -> Vec<PromptFragment> {
        request
            .prepared
            .and_then(|prepared| prepared.get::<PreparedFragments>())
            .map(|prepared| prepared.0.clone())
            .unwrap_or_default()
    }
}
