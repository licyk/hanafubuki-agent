//! `/providers` and the first-run setup onto the facade's Provider calls.
//! Every write names the registry revision its screen read; a write that
//! fails re-reads the registry, and when its revision moved the screen gets
//! the new one with the failure.

use std::fmt;

use hanafubuki_agent::{
    AgentHandle, AgentModelPolicySettings, AgentModelPolicyWriteRequest,
    AgentProviderDeleteRequest, AgentProviderDraft, AgentProviderEnabledRequest,
    AgentProviderModelCatalogRequest, AgentProviderRegistrySnapshot, AgentProviderWriteRequest,
    ModelCatalogEntry, ModelMetadata,
};

use super::{AgentBridge, AgentReply, TurnUpdate};

/// What the Provider screen asks of the Agent.
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum ProviderCall {
    Load,
    /// The models of a draft, which is not saved.
    Test(AgentProviderDraft),
    Write {
        expected_revision: u64,
        /// `None` adds a Provider.
        provider_id: Option<String>,
        draft: AgentProviderDraft,
    },
    Delete {
        expected_revision: u64,
        provider_id: String,
    },
    SetEnabled {
        expected_revision: u64,
        provider_id: String,
        enabled: bool,
    },
    /// Makes `provider_id` the default, with `model` when one is given
    /// and the model policy's otherwise.
    SetDefault {
        expected_revision: u64,
        provider_id: String,
        model: Option<String>,
    },
    /// Replaces the shared policy that says how every model runs.
    WriteModelPolicy {
        expected_revision: u64,
        default_provider_id: Option<String>,
        policy: AgentModelPolicySettings,
    },
    ListModels {
        provider_id: String,
    },
}

/// What the Agent answered.
pub(crate) enum ProviderReply {
    /// The registry, and what is known of its policy's model on the
    /// default Provider (nothing when there is no default).
    Loaded {
        registry: AgentProviderRegistrySnapshot,
        metadata: ModelMetadata,
    },
    Tested(Result<Vec<ModelCatalogEntry>, String>),
    /// A Provider was written; the registry it left.
    Saved(AgentProviderRegistrySnapshot),
    /// A Provider was deleted, enabled or disabled, or the default changed.
    Changed(AgentProviderRegistrySnapshot),
    /// The shared model policy was written.
    ModelPolicySaved(AgentProviderRegistrySnapshot),
    Models {
        provider_id: String,
        result: Result<Vec<ModelCatalogEntry>, String>,
    },
    /// The call failed; `reloaded` is the registry when its revision moved
    /// since the screen read it.
    Failed {
        error: String,
        reloaded: Option<AgentProviderRegistrySnapshot>,
    },
}

impl ProviderReply {
    /// The registry as the Agent now has it, when the reply carries it.
    pub(crate) fn registry(&self) -> Option<&AgentProviderRegistrySnapshot> {
        match self {
            Self::Loaded { registry, .. }
            | Self::Saved(registry)
            | Self::Changed(registry)
            | Self::ModelPolicySaved(registry) => Some(registry),
            Self::Failed { reloaded, .. } => reloaded.as_ref(),
            Self::Tested(_) | Self::Models { .. } => None,
        }
    }
}

/// Without the drafts and the registry, which hold tokens.
impl fmt::Debug for ProviderCall {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Load => "Load",
            Self::Test(_) => "Test",
            Self::Write { .. } => "Write",
            Self::Delete { .. } => "Delete",
            Self::SetEnabled { .. } => "SetEnabled",
            Self::SetDefault { .. } => "SetDefault",
            Self::WriteModelPolicy { .. } => "WriteModelPolicy",
            Self::ListModels { .. } => "ListModels",
        };
        formatter.write_str(name)
    }
}

impl fmt::Debug for ProviderReply {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Loaded { .. } => formatter.write_str("Loaded"),
            Self::Tested(result) => {
                write!(formatter, "Tested({:?})", result.as_ref().map(Vec::len))
            }
            Self::Saved(_) => formatter.write_str("Saved"),
            Self::Changed(_) => formatter.write_str("Changed"),
            Self::ModelPolicySaved(_) => formatter.write_str("ModelPolicySaved"),
            Self::Models { provider_id, .. } => write!(formatter, "Models({provider_id})"),
            Self::Failed { error, .. } => write!(formatter, "Failed({error})"),
        }
    }
}

impl AgentBridge {
    pub(super) fn provider_call(&self, call: ProviderCall) {
        self.spawn(move |agent| async move {
            let reply = run(&agent, call).await;
            Ok(Some(AgentReply::Update(Box::new(TurnUpdate::Providers(
                reply,
            )))))
        });
    }
}

async fn run(agent: &AgentHandle, call: ProviderCall) -> ProviderReply {
    match call {
        ProviderCall::Load => match agent.provider_registry_snapshot().await {
            Ok(registry) => {
                let metadata = model_metadata(agent, &registry).await;
                ProviderReply::Loaded { registry, metadata }
            }
            Err(error) => ProviderReply::Failed {
                error: format!("the Providers were not read: {error}"),
                reloaded: None,
            },
        },
        ProviderCall::Test(draft) => ProviderReply::Tested(agent.test_provider_draft(draft).await),
        ProviderCall::Write {
            expected_revision,
            provider_id,
            draft,
        } => {
            let written = agent
                .write_provider(AgentProviderWriteRequest {
                    expected_revision,
                    provider_id,
                    provider: draft,
                })
                .await;
            reply(agent, expected_revision, written, ProviderReply::Saved).await
        }
        ProviderCall::Delete {
            expected_revision,
            provider_id,
        } => {
            let written = agent
                .delete_provider(AgentProviderDeleteRequest {
                    expected_revision,
                    provider_id,
                })
                .await;
            reply(agent, expected_revision, written, ProviderReply::Changed).await
        }
        ProviderCall::SetEnabled {
            expected_revision,
            provider_id,
            enabled,
        } => {
            let written = agent
                .set_provider_enabled(AgentProviderEnabledRequest {
                    expected_revision,
                    provider_id,
                    enabled,
                })
                .await;
            reply(agent, expected_revision, written, ProviderReply::Changed).await
        }
        ProviderCall::SetDefault {
            expected_revision,
            provider_id,
            model,
        } => {
            let written = set_default(agent, expected_revision, provider_id, model).await;
            reply(agent, expected_revision, written, ProviderReply::Changed).await
        }
        ProviderCall::WriteModelPolicy {
            expected_revision,
            default_provider_id,
            policy,
        } => {
            let written = agent
                .write_model_policy(AgentModelPolicyWriteRequest {
                    expected_revision,
                    default_provider_id,
                    policy,
                })
                .await;
            reply(
                agent,
                expected_revision,
                written,
                ProviderReply::ModelPolicySaved,
            )
            .await
        }
        ProviderCall::ListModels { provider_id } => {
            let result = agent
                .list_provider_models(AgentProviderModelCatalogRequest {
                    provider_ids: Some(vec![provider_id.clone()]),
                })
                .await
                .and_then(|catalogs| {
                    let catalog = catalogs
                        .into_iter()
                        .next()
                        .ok_or_else(|| "the Provider listed no catalog".to_string())?;
                    match catalog.error {
                        Some(error) => Err(error),
                        None => Ok(catalog.models),
                    }
                });
            ProviderReply::Models {
                provider_id,
                result,
            }
        }
    }
}

/// The model policy is written whole: it is the one at `expected_revision`
/// (the write is refused when the registry moved), with `model` in it.
async fn set_default(
    agent: &AgentHandle,
    expected_revision: u64,
    provider_id: String,
    model: Option<String>,
) -> Result<AgentProviderRegistrySnapshot, String> {
    let mut policy = agent.provider_registry_snapshot().await?.model_policy;
    if let Some(model) = model {
        policy.model = model;
    }
    agent
        .write_model_policy(AgentModelPolicyWriteRequest {
            expected_revision,
            default_provider_id: Some(provider_id),
            policy,
        })
        .await
}

/// A write's reply; a failed one carries the registry when it moved.
async fn reply(
    agent: &AgentHandle,
    expected_revision: u64,
    written: Result<AgentProviderRegistrySnapshot, String>,
    done: fn(AgentProviderRegistrySnapshot) -> ProviderReply,
) -> ProviderReply {
    match written {
        Ok(registry) => done(registry),
        Err(error) => ProviderReply::Failed {
            error,
            reloaded: agent
                .provider_registry_snapshot()
                .await
                .ok()
                .filter(|registry| registry.revision != expected_revision),
        },
    }
}

/// What is known of the policy's model on the default Provider; nothing
/// when nothing is or there is no default.
async fn model_metadata(
    agent: &AgentHandle,
    registry: &AgentProviderRegistrySnapshot,
) -> ModelMetadata {
    let Some(provider_id) = registry.default_provider_id.as_deref() else {
        return ModelMetadata::default();
    };
    agent
        .model_metadata(provider_id, &registry.model_policy.model)
        .await
        .unwrap_or_default()
}
