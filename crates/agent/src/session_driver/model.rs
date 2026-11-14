//! A session's model selection, which the facade's `set_session_model` and
//! a spawned agent share, and a spawned agent's Provider, model and
//! reasoning effort: resolved and checked before the agent exists, then
//! applied to it in one commit.

use std::collections::BTreeSet;

use hanafubuki_agent_config::{AgentProviderRegistrySettings, AgentProviderSettings};
use hanafubuki_agent_foundation::blocking::run_blocking_operation;
use hanafubuki_agent_kernel::{
    AgentGeneration, AgentRevision, AgentSessionId, AgentSessionRuntimeSnapshot, KnownReasoning,
    ModelReasoningEffort,
};

use super::AgentSessionDriver;
use crate::service::{configured_model_facts, resolve_settings_provider};
use crate::{
    model_api_protocol, AgentModelInfo, AgentSessionModelSelectionRequest, ResolvedAgentProvider,
};

/// What a spawn asks of its agent's model: what the model passed, and
/// what the role it is spawned as sets.
pub(crate) struct ChildModelRequest {
    /// A Provider's id, or the display name of exactly one enabled one.
    pub(crate) provider: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) reasoning_effort: Option<ModelReasoningEffort>,
    pub(crate) role_model: Option<String>,
    pub(crate) role_reasoning_effort: Option<ModelReasoningEffort>,
}

/// A spawned agent's model, resolved and checked against its parent's.
pub(crate) struct ChildModel {
    /// The Provider and model it runs, when the pair is not its parent's.
    pair: Option<(ResolvedAgentProvider, String)>,
    reasoning_effort: Option<ModelReasoningEffort>,
}

/// Why a spawn's model was refused, with what its text names.
#[derive(Debug)]
pub(crate) enum ChildModelError {
    /// No enabled Provider is `requested`; these are, as id and name.
    UnknownProvider {
        requested: String,
        enabled: Vec<(String, String)>,
    },
    /// The Provider's settings do not resolve (its token, a header).
    UnusableProvider(String),
    /// The Provider's recorded listing, with what is configured for it,
    /// does not know `model`, which the model passed (`explicit`) or
    /// inherited.
    UnknownModel {
        model: String,
        explicit: bool,
        provider_id: String,
        provider_name: String,
        known: BTreeSet<String>,
    },
    /// `model` takes no `effort`: `known` is its levels, or none.
    UnknownEffort {
        effort: ModelReasoningEffort,
        model: String,
        known: KnownReasoning,
    },
    /// The settings could not be read.
    Failed(String),
}

impl AgentSessionDriver {
    /// Selects `request.model` of the Provider `request.provider_id` for
    /// the session, with that Provider's model policy: its reasoning
    /// effort, context window, input capabilities, compaction and turn
    /// budget.
    pub(crate) async fn select_model(
        &self,
        request: AgentSessionModelSelectionRequest,
    ) -> Result<AgentSessionRuntimeSnapshot, String> {
        if request.model.trim().is_empty() {
            return Err("Agent model must contain 1 to 256 bytes".to_string());
        }
        let registry = self.provider_settings().await?;
        let resolved = resolve_settings_provider(&registry, &request.provider_id)?;
        let reasoning_effort = resolved.policy.reasoning_effort.clone();
        self.apply_model_selection(
            request.session_id,
            request.generation,
            request.revision,
            resolved,
            request.model,
            reasoning_effort,
        )
        .await
    }

    /// The Provider, model and reasoning effort of an agent `parent`
    /// spawns, each the one `request` passes, else its role's, else the
    /// parent's (a Provider is never the role's). What the model passed
    /// is checked: the Provider is enabled; the model, when a Provider or
    /// a model was passed, is one the Provider's recorded listing or its
    /// settings know (any, without a recorded listing); the effort is one
    /// of the model's known levels (any, when they are not known). Reads
    /// no Provider's settings when nothing was passed and the role keeps
    /// the parent's model. Makes no request.
    pub(crate) async fn resolve_child_model(
        &self,
        parent: &AgentSessionRuntimeSnapshot,
        request: ChildModelRequest,
    ) -> Result<ChildModel, ChildModelError> {
        let explicit_model = request.model.is_some();
        let explicit_pair = explicit_model || request.provider.is_some();
        let model = request
            .model
            .or(request.role_model)
            .unwrap_or_else(|| parent.model.clone());
        let checked_effort = request.reasoning_effort.clone();
        let reasoning_effort = request
            .reasoning_effort
            .or(request.role_reasoning_effort)
            .or_else(|| parent.reasoning_effort.clone());
        if !explicit_pair && checked_effort.is_none() && model == parent.model {
            return Ok(ChildModel {
                pair: None,
                reasoning_effort,
            });
        }
        let registry = self
            .provider_settings()
            .await
            .map_err(ChildModelError::Failed)?;
        let requested = request.provider.as_deref().unwrap_or(&parent.provider_id);
        let provider = enabled_provider(&registry, requested)?;
        let provider = resolve_settings_provider(&registry, &provider.id)
            .map_err(ChildModelError::UnusableProvider)?;
        if explicit_pair {
            self.discovery.request(Some(&provider.definition.id));
            let known = self.known_models(&registry, &provider.definition, parent);
            if known.as_ref().is_some_and(|known| !known.contains(&model)) {
                return Err(ChildModelError::UnknownModel {
                    model,
                    explicit: explicit_model,
                    provider_id: provider.definition.id.clone(),
                    provider_name: provider.definition.display_name.clone(),
                    known: known.unwrap_or_default(),
                });
            }
        }
        if let Some(effort) = checked_effort {
            let known = self.known_reasoning(&provider.definition, &model);
            if refuses(&known, &effort) {
                return Err(ChildModelError::UnknownEffort {
                    effort,
                    model,
                    known,
                });
            }
        }
        let same_pair = provider.definition.id == parent.provider_id && model == parent.model;
        Ok(ChildModel {
            pair: (!same_pair).then_some((provider, model)),
            reasoning_effort,
        })
    }

    /// Gives `child`, which has its parent's model, `model`: a model
    /// selection when the pair is another, which sets the effort in the
    /// same commit; else the effort alone, when it is another. Returns
    /// the child as it then is.
    pub(crate) async fn apply_child_model(
        &self,
        child: AgentSessionId,
        model: ChildModel,
    ) -> Result<AgentSessionRuntimeSnapshot, String> {
        let snapshot = self.blocking_snapshot(child).await?;
        if let Some((provider, id)) = model.pair {
            return self
                .apply_model_selection(
                    child,
                    snapshot.generation,
                    snapshot.revision,
                    provider,
                    id,
                    model.reasoning_effort,
                )
                .await;
        }
        if snapshot.reasoning_effort == model.reasoning_effort {
            return Ok(snapshot);
        }
        let (generation, revision) = (snapshot.generation, snapshot.revision);
        let core = self.core.clone();
        run_blocking_operation(
            self.blocking.as_ref(),
            "agent session reasoning effort worker",
            move || {
                core.set_session_reasoning_effort(
                    child,
                    generation,
                    revision,
                    model.reasoning_effort,
                )
                .map_err(|error| error.to_string())
            },
        )
        .await?
    }

    /// Gives the session `model` of `resolved` with `reasoning_effort` and
    /// the rest of the Provider's model policy, in one commit.
    async fn apply_model_selection(
        &self,
        session_id: AgentSessionId,
        generation: AgentGeneration,
        revision: AgentRevision,
        resolved: ResolvedAgentProvider,
        model: String,
        reasoning_effort: Option<ModelReasoningEffort>,
    ) -> Result<AgentSessionRuntimeSnapshot, String> {
        self.discovery.request(Some(&resolved.definition.id));
        let policy = resolved.policy;
        let provider = resolved.definition;
        let configured_facts =
            configured_model_facts(&provider, policy.context_window_tokens, &model);
        let model_info = AgentModelInfo::from_provider(
            model.clone(),
            provider.protocol,
            &self.base_instructions,
        );
        let core = self.core.clone();
        run_blocking_operation(
            self.blocking.as_ref(),
            "agent session model mutation worker",
            move || {
                core.activate_session(session_id)
                    .map_err(|error| error.to_string())?;
                core.set_session_model_configuration(
                    session_id,
                    generation,
                    revision,
                    model_api_protocol(provider.protocol),
                    &provider.base_url,
                    resolved.credential,
                    resolved.headers,
                    hanafubuki_agent_kernel::ModelRetryPolicy::default(),
                    hanafubuki_agent_kernel::AgentSessionModelConfigurationUpdate {
                        provider_id: provider.id,
                        provider_protocol: model_api_protocol(provider.protocol),
                        model,
                        instructions: model_info.resolve_instructions(None),
                        reasoning_effort,
                        configured_facts,
                        input_capabilities: policy.input_capabilities,
                        auto_compaction_enabled: policy.auto_compaction_enabled,
                        auto_compact_threshold_percent: policy.auto_compact_threshold_percent,
                        compaction_capability: policy.compaction_capability,
                        turn_budget: policy.turn_budget,
                    },
                )
                .map_err(|error| error.to_string())
            },
        )
        .await?
    }

    /// The models `provider` is known to serve: its recorded listing, the
    /// ids configured for it, the policy's model when it is the default
    /// Provider and `parent`'s when it is `parent`'s; none known without
    /// a recorded listing.
    fn known_models(
        &self,
        registry: &AgentProviderRegistrySettings,
        provider: &AgentProviderSettings,
        parent: &AgentSessionRuntimeSnapshot,
    ) -> Option<BTreeSet<String>> {
        let mut known = self
            .core
            .listed_models(model_api_protocol(provider.protocol), &provider.base_url)?;
        known.extend(provider.models.keys().cloned());
        if registry.default_provider_id.as_deref() == Some(provider.id.as_str()) {
            known.insert(registry.model_policy.model.trim().to_string());
        }
        if parent.provider_id == provider.id {
            known.insert(parent.model.clone());
        }
        Some(known)
    }

    /// The reasoning efforts `model` of `provider` is known to take.
    fn known_reasoning(&self, provider: &AgentProviderSettings, model: &str) -> KnownReasoning {
        self.core
            .model_metadata(
                &provider.id,
                model_api_protocol(provider.protocol),
                &provider.base_url,
                model,
                &configured_model_facts(provider, None, model),
            )
            .unwrap_or_default()
            .known_reasoning()
    }

    async fn provider_settings(&self) -> Result<AgentProviderRegistrySettings, String> {
        let config = self.config.clone();
        run_blocking_operation(
            self.blocking.as_ref(),
            "agent provider settings read worker",
            move || config.providers(),
        )
        .await?
    }

    async fn blocking_snapshot(
        &self,
        session_id: AgentSessionId,
    ) -> Result<AgentSessionRuntimeSnapshot, String> {
        let driver = self.clone();
        run_blocking_operation(
            self.blocking.as_ref(),
            "agent session snapshot worker",
            move || driver.snapshot(session_id),
        )
        .await?
        .map_err(|error| error.to_string())
    }
}

/// The enabled Provider whose id is `requested`, else the one enabled
/// Provider whose display name it is, case aside.
fn enabled_provider<'a>(
    registry: &'a AgentProviderRegistrySettings,
    requested: &str,
) -> Result<&'a AgentProviderSettings, ChildModelError> {
    let enabled = registry
        .providers
        .iter()
        .filter(|provider| provider.enabled)
        .collect::<Vec<_>>();
    let lowered = requested.to_lowercase();
    let named = enabled
        .iter()
        .filter(|provider| provider.display_name.to_lowercase() == lowered)
        .collect::<Vec<_>>();
    let found = enabled
        .iter()
        .find(|provider| provider.id == requested)
        .or(match named[..] {
            [provider] => Some(provider),
            _ => None,
        });
    match found {
        Some(provider) => Ok(provider),
        None => Err(ChildModelError::UnknownProvider {
            requested: requested.to_string(),
            enabled: enabled
                .iter()
                .map(|provider| (provider.id.clone(), provider.display_name.clone()))
                .collect(),
        }),
    }
}

/// Whether a model whose known levels are `known` refuses `effort`.
fn refuses(known: &KnownReasoning, effort: &ModelReasoningEffort) -> bool {
    match known {
        KnownReasoning::Levels { efforts, .. } => !efforts.contains(effort),
        KnownReasoning::Unsupported { .. } => true,
        KnownReasoning::Unknown => false,
    }
}
