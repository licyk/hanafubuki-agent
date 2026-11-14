//! A session's model selection, which the facade's `set_session_model` and
//! a spawned agent share, and a spawned agent's Provider, model and
//! reasoning effort: resolved and checked before the agent exists, then
//! applied to it in one commit. Also what a parent may spawn agents on, as
//! its guidance lists it, from the known models the checks use.

use std::collections::{BTreeSet, HashSet};

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

/// The longest model id a spawn takes, in bytes: the kernel's bound.
pub(crate) const MAXIMUM_MODEL_ID_BYTES: usize = 256;
/// The most enabled Providers, and known models of each, the guidance lists.
const MAXIMUM_CATALOG_PROVIDERS: usize = 8;
const MAXIMUM_CATALOG_MODELS: usize = 8;
/// The longest model id the guidance lists, in characters: a longer one is
/// left out rather than cut, since a cut id cannot be passed.
const MAXIMUM_CATALOG_MODEL_ID_CHARS: usize = 128;

/// What a parent may spawn agents on: its Provider and model, and the
/// enabled Providers, its own first, then by display name (case aside) and
/// id, each with the models known of it as a spawn's checks know them; at
/// most [`MAXIMUM_CATALOG_PROVIDERS`] Providers of
/// [`MAXIMUM_CATALOG_MODELS`] models each.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SpawnModelCatalog {
    pub(crate) provider_id: String,
    pub(crate) model: String,
    pub(crate) providers: Vec<SpawnProvider>,
    /// The enabled Providers left out.
    pub(crate) more_providers: usize,
}

/// An enabled Provider of a [`SpawnModelCatalog`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SpawnProvider {
    pub(crate) id: String,
    pub(crate) display_name: String,
    /// Whether a listing of it is recorded; without one, a spawn passes any
    /// model id to it.
    pub(crate) listed: bool,
    /// Its first known models, with the reasoning efforts each takes.
    pub(crate) models: Vec<(String, KnownReasoning)>,
    /// Its known models left out.
    pub(crate) more_models: usize,
}

/// The models a Provider is known to serve, in the order the guidance
/// lists them, and whether its listing is recorded.
struct KnownModels {
    ids: Vec<String>,
    listed: bool,
}

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
            if known.listed && !known.ids.contains(&model) {
                return Err(ChildModelError::UnknownModel {
                    model,
                    explicit: explicit_model,
                    provider_id: provider.definition.id.clone(),
                    provider_name: provider.definition.display_name.clone(),
                    known: known.ids.into_iter().collect(),
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

    /// What `parent` may spawn agents on ([`SpawnModelCatalog`]): the
    /// cached Provider registry, each Provider's known models
    /// ([`Self::known_models`]) and the known reasoning of those listed,
    /// leaving out an id `model` cannot pass. (Every Provider the registry
    /// holds resolves: its reader drops one whose token or headers do
    /// not.) Blocks; makes no request.
    pub(crate) fn spawn_model_catalog(
        &self,
        parent: &AgentSessionRuntimeSnapshot,
    ) -> Result<SpawnModelCatalog, String> {
        let registry = self.config.providers()?;
        let mut enabled = registry
            .providers
            .iter()
            .filter(|provider| provider.enabled)
            .collect::<Vec<_>>();
        enabled.sort_by_cached_key(|provider| {
            (
                provider.id != parent.provider_id,
                provider.display_name.to_lowercase(),
                provider.id.clone(),
            )
        });
        let providers = enabled
            .iter()
            .take(MAXIMUM_CATALOG_PROVIDERS)
            .map(|provider| {
                let known = self.known_models(&registry, provider, parent);
                let total = known.ids.len();
                let models = known
                    .ids
                    .into_iter()
                    .filter(|id| catalog_model_id(id))
                    .take(MAXIMUM_CATALOG_MODELS)
                    .map(|id| {
                        let reasoning = self.known_reasoning(provider, &id);
                        (id, reasoning)
                    })
                    .collect::<Vec<_>>();
                SpawnProvider {
                    id: provider.id.clone(),
                    display_name: provider.display_name.clone(),
                    listed: known.listed,
                    more_models: total - models.len(),
                    models,
                }
            })
            .collect::<Vec<_>>();
        Ok(SpawnModelCatalog {
            provider_id: parent.provider_id.clone(),
            model: parent.model.clone(),
            more_providers: enabled.len() - providers.len(),
            providers,
        })
    }

    /// The models `provider` is known to serve, each once, in this order:
    /// `parent`'s when it is `parent`'s Provider, the ids configured for it
    /// (`models."<id>"`), the policy's model when it is the default
    /// Provider, then its recorded listing by id; and whether a listing is
    /// recorded, without which a spawn passes any model.
    fn known_models(
        &self,
        registry: &AgentProviderRegistrySettings,
        provider: &AgentProviderSettings,
        parent: &AgentSessionRuntimeSnapshot,
    ) -> KnownModels {
        let listing = self
            .core
            .listed_models(model_api_protocol(provider.protocol), &provider.base_url);
        let mut ids = Vec::new();
        if parent.provider_id == provider.id {
            ids.push(parent.model.clone());
        }
        ids.extend(provider.models.keys().cloned());
        let policy_model = registry.model_policy.model.trim();
        if registry.default_provider_id.as_deref() == Some(provider.id.as_str())
            && !policy_model.is_empty()
        {
            ids.push(policy_model.to_string());
        }
        let listed = listing.is_some();
        ids.extend(listing.into_iter().flatten());
        let mut seen = HashSet::new();
        ids.retain(|id| seen.insert(id.clone()));
        KnownModels { ids, listed }
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

/// Whether the guidance lists the model id `id`: one `spawn_agent`'s
/// `model` takes as it is (one line, trimmed, at most
/// [`MAXIMUM_MODEL_ID_BYTES`]), of at most
/// [`MAXIMUM_CATALOG_MODEL_ID_CHARS`], with no backtick or control
/// character to break the text it is quoted in.
fn catalog_model_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAXIMUM_MODEL_ID_BYTES
        && id.chars().count() <= MAXIMUM_CATALOG_MODEL_ID_CHARS
        && id.trim() == id
        && !id
            .chars()
            .any(|character| character == '`' || character.is_control())
}

/// Whether a model whose known levels are `known` refuses `effort`.
fn refuses(known: &KnownReasoning, effort: &ModelReasoningEffort) -> bool {
    match known {
        KnownReasoning::Levels { efforts, .. } => !efforts.contains(effort),
        KnownReasoning::Unsupported { .. } => true,
        KnownReasoning::Unknown => false,
    }
}
