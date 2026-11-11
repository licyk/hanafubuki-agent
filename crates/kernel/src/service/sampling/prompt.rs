//! The context sections of a model request: the prompt fragments, prepared
//! once per sampling step and produced by the plugins' prompt contributors
//! from the turn's frozen extensions (never under the session lock), and the
//! world state; each shown by its kind's renderer and ordered by its slot
//! and order.

use hanafubuki_agent_contracts::{
    ContextRendererSnapshot, ContextRole, ContextSlot, PromptContributorSnapshot, PromptFragment,
};

use super::*;

impl AgentService {
    /// The request context for a request about to be built for
    /// `session_id` outside a sampling step (a standalone compaction's check
    /// of the next request): the inputs are read under a brief lock and
    /// rendered after it is released. The caller re-checks under its own
    /// lock that the revision is unchanged.
    ///
    /// Such a request belongs to no sampling step, so no contributor
    /// prepared for it.
    pub(in crate::service) fn render_prompt(
        &self,
        session_id: AgentSessionId,
    ) -> Result<RequestContext, AgentError> {
        let (inputs, scope) = {
            let cell = self.session(session_id)?;
            let guard = cell.lock()?;
            (prompt_inputs(&guard), guard.settings.config.scope.clone())
        };
        Ok(request_context(
            &self
                .inner
                .registries
                .prompt
                .snapshot()
                .without_plugins(|owner| scope.turns_off(owner)),
            self.inner.registries.context_renderers.snapshot(),
            inputs,
            None,
        ))
    }
}

/// What the prompt contributors render from: the turn's frozen extensions
/// (none without a turn) and the session's context window. Read under the
/// session lock; rendering happens after it is released.
fn prompt_inputs(
    record: &SessionRecord,
) -> (
    Option<hanafubuki_agent_contracts::TurnExtensions>,
    Option<u64>,
) {
    (
        record
            .runtime
            .turn
            .as_ref()
            .map(|turn| turn.extensions.clone()),
        record.settings.config.context_window_tokens,
    )
}

/// Prepares one sampling step: asks every contributor of `contributors`, one
/// after another in contributor order, each under a lease on its entry, and
/// merges what they return. Runs with no kernel lock held.
///
/// A contributor whose plugin is stopping prepares nothing. Its error, its
/// panic (reported as a fault of its plugin), no answer within `deadline`
/// (the call is cancelled), or a value of a type an earlier contributor
/// already prepared, leaves its values out with a warning: a plugin never
/// fails the step. Only the turn's cancellation does.
pub(super) async fn prepare_prompt_step(
    contributors: &PromptContributorSnapshot,
    step: hanafubuki_agent_contracts::PromptStep,
    cancellation: &CancellationToken,
    deadline: Duration,
) -> Result<hanafubuki_agent_contracts::TurnExtensions, AgentError> {
    let mut prepared = hanafubuki_agent_contracts::TurnExtensions::default();
    for entry in contributors.entries() {
        let contributor = entry.item();
        let call = cancellation.child();
        let Some(_lease) = entry.acquire(&call) else {
            continue;
        };
        let prepared_values = tokio::time::timeout(
            deadline,
            entry.guard_async(
                hanafubuki_agent_contracts::ExtensionPoint::PromptPrepare,
                |contributor| contributor.prepare(&step, call.clone()),
            ),
        )
        .await;
        let Ok(prepared_values) = prepared_values else {
            call.cancel();
            log::warn!(
                "Agent prompt contributor {} did not prepare the step within {} s and was cancelled; its values are left out",
                contributor.id(),
                deadline.as_secs_f64()
            );
            continue;
        };
        let Ok(prepared_values) = prepared_values else {
            log::warn!(
                "Agent prompt contributor of the {} plugin failed with an internal error; its values are left out",
                entry.owner()
            );
            continue;
        };
        match prepared_values {
            Ok(values) => {
                if !prepared.merge_disjoint(values) {
                    log::warn!(
                        "Agent prompt contributor {} prepared a value an earlier contributor prepared; its values are left out",
                        contributor.id()
                    );
                }
            }
            Err(_error) if cancellation.is_cancelled() => return Err(AgentError::Cancelled),
            Err(error) => log::warn!(
                "Agent prompt contributor {} could not prepare the step: {}",
                contributor.id(),
                bounded_diagnostic(error)
            ),
        }
    }
    Ok(prepared)
}

/// What a request's context sections are built from: the renderers in force
/// and the prompt fragments rendered for it.
#[derive(Clone)]
pub(in crate::service) struct RequestContext {
    renderers: ContextRendererSnapshot,
    fragments: Vec<PromptFragment>,
}

/// The context of one request: its prompt fragments, in contributor order,
/// rendered with what the contributors prepared for its sampling step
/// (`None` outside one). Contributors are plugin code, so this runs with no
/// kernel lock held.
pub(super) fn request_context(
    contributors: &PromptContributorSnapshot,
    renderers: ContextRendererSnapshot,
    (turn, context_window_tokens): (
        Option<hanafubuki_agent_contracts::TurnExtensions>,
        Option<u64>,
    ),
    prepared: Option<&hanafubuki_agent_contracts::TurnExtensions>,
) -> RequestContext {
    let fragments = turn
        .map(|turn| {
            contributors.fragments(&hanafubuki_agent_contracts::PromptRequest {
                turn: &turn,
                prepared,
                context_window_tokens,
            })
        })
        .unwrap_or_default();
    RequestContext {
        renderers,
        fragments,
    }
}

/// One item of a request's input; a context section keeps where it is
/// ordered among the sections next to it: its renderer's slot and order, then
/// its kind.
pub(super) enum PlacedInput {
    Section {
        position: (ContextSlot, u16, String),
        item: ModelInputItem,
    },
    Item(ModelInputItem),
}

impl RequestContext {
    /// The prompt fragments as sections, every one of them in every request:
    /// a compacted context window re-renders them from the registry rather
    /// than carrying them in its initial context.
    pub(super) fn fragments(&self) -> Vec<PlacedInput> {
        self.fragments
            .iter()
            .filter_map(|fragment| {
                self.section(
                    &fragment.kind,
                    fragment.version.as_deref(),
                    &fragment.content,
                )
            })
            .collect()
    }

    /// `content` of `kind` as its renderer shows it; nothing when no renderer
    /// of the kind is registered (its plugin stopped).
    pub(super) fn section(
        &self,
        kind: &ContextSectionKind,
        version: Option<&str>,
        content: &str,
    ) -> Option<PlacedInput> {
        let Some(renderer) = self.renderers.find(kind) else {
            log::debug!("Agent context section {kind} left out: no renderer is registered");
            return None;
        };
        let mut text = String::from(renderer.preamble);
        if renderer.versioned {
            text.push_str("<agent_world_state kind=\"");
            text.push_str(kind.as_str());
            if let Some(version) = version {
                text.push_str("\" version=\"");
                text.push_str(version);
            }
            text.push_str("\">\n");
            text.push_str(content);
            text.push_str("\n</agent_world_state>");
        } else {
            text.push_str(content);
        }
        text.push_str(renderer.epilogue);
        Some(PlacedInput::Section {
            position: (renderer.slot, renderer.order, kind.as_str().to_string()),
            item: ModelInputItem::ContextFragment {
                role: match renderer.role {
                    ContextRole::Developer => ModelContextRole::Developer,
                    ContextRole::User => ModelContextRole::User,
                },
                kind: kind.as_str().to_string(),
                content: text,
            },
        })
    }
}

/// The request input in order: every run of adjacent sections is ordered by
/// slot, then by renderer order, then by kind, and every other item keeps its
/// place. The prompt fragments join the sections the conversation opens
/// with; a section that changed later in the conversation stays where it
/// changed, so the prefix before it stays cached.
pub(super) fn ordered_input(placed: Vec<PlacedInput>) -> Vec<ModelInputItem> {
    let mut input = Vec::with_capacity(placed.len());
    let mut run = Vec::new();
    for entry in placed {
        match entry {
            PlacedInput::Section { position, item } => run.push((position, item)),
            PlacedInput::Item(item) => {
                flush_sections(&mut run, &mut input);
                input.push(item);
            }
        }
    }
    flush_sections(&mut run, &mut input);
    input
}

fn flush_sections(
    run: &mut Vec<((ContextSlot, u16, String), ModelInputItem)>,
    input: &mut Vec<ModelInputItem>,
) {
    run.sort_by(|(left, _), (right, _)| left.cmp(right));
    input.extend(run.drain(..).map(|(_, item)| item));
}
