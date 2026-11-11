use super::resolve_provider_definition;
use crate::service::AgentApplicationService;
use crate::{
    model_api_protocol, AgentModelPolicyWriteRequest, AgentProviderDeleteRequest,
    AgentProviderDraft, AgentProviderEnabledRequest, AgentProviderModelCatalogRequest,
    AgentProviderModelCatalogResult, AgentProviderRegistrySnapshot, AgentProviderWriteRequest,
    ResolvedAgentProvider,
};
use hanafubuki_agent_config::{
    validate_agent_model_policy, AgentModelPolicySettings, AgentProviderRegistrySettings,
    AgentProviderSettings, AgentSettings, MAX_AGENT_PROVIDERS,
};
use hanafubuki_agent_kernel::{ModelCatalogEntry, ModelReasoningEffort, ModelReasoningSupport};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::Ordering;

impl AgentApplicationService {
    pub(in crate::service) async fn load_provider_settings(
        &self,
    ) -> Result<AgentProviderRegistrySettings, String> {
        let config = self.config.clone();
        hanafubuki_agent_foundation::blocking::run_blocking_operation(
            self.blocking.as_ref(),
            "agent provider settings read worker",
            move || config.providers(),
        )
        .await?
    }

    pub(in crate::service) async fn load_agent_settings(&self) -> Result<AgentSettings, String> {
        let config = self.config.clone();
        hanafubuki_agent_foundation::blocking::run_blocking_operation(
            self.blocking.as_ref(),
            "agent settings read worker",
            move || config.settings(),
        )
        .await?
    }

    async fn mutate_provider_registry<F>(
        &self,
        expected_revision: u64,
        mutate: F,
    ) -> Result<AgentProviderRegistrySnapshot, String>
    where
        F: FnOnce(&mut AgentProviderRegistrySettings) -> Result<(), String> + Send + 'static,
    {
        let _gate = self.provider_configuration_gate.lock().await;
        let config = self.config.clone();
        hanafubuki_agent_foundation::blocking::run_blocking_operation(
            self.blocking.as_ref(),
            "agent provider settings mutation worker",
            move || {
                config.mutate_providers(expected_revision, |registry| {
                    mutate(registry)?;
                    reconcile_default_provider(registry);
                    Ok(())
                })
            },
        )
        .await?
    }

    pub async fn list_provider_models(
        &self,
        request: AgentProviderModelCatalogRequest,
    ) -> Result<Vec<AgentProviderModelCatalogResult>, String> {
        if !self.accepting.load(Ordering::Acquire) {
            return Err("Agent service is shutting down".to_string());
        }
        let settings = self.load_provider_settings().await?;
        let provider_ids = if let Some(provider_ids) = request.provider_ids {
            if provider_ids.len() > MAX_AGENT_PROVIDERS {
                return Err("too many Agent Providers requested".into());
            }
            let mut unique = BTreeSet::new();
            for provider_id in &provider_ids {
                if !unique.insert(provider_id.clone()) {
                    return Err("duplicate Agent Provider id in catalog request".into());
                }
            }
            provider_ids
        } else {
            settings
                .providers
                .iter()
                .filter(|provider| provider.enabled)
                .map(|provider| provider.id.clone())
                .collect()
        };
        let names = settings
            .providers
            .iter()
            .map(|provider| (provider.id.clone(), provider.display_name.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut tasks = tokio::task::JoinSet::new();
        for provider_id in provider_ids {
            let service = self.clone();
            let display_name = names
                .get(&provider_id)
                .cloned()
                .unwrap_or_else(|| provider_id.clone());
            tasks.spawn(async move {
                let result = async {
                    let resolved = service.resolve_provider(&provider_id).await?;
                    service.provider_models(resolved).await
                }
                .await;
                AgentProviderModelCatalogResult {
                    provider_id,
                    display_name,
                    models: result.as_ref().cloned().unwrap_or_default(),
                    error: result.err(),
                }
            });
        }
        let mut results = Vec::new();
        while let Some(result) = tasks.join_next().await {
            results.push(
                result.map_err(|_| "Agent Provider model catalog worker failed".to_string())?,
            );
        }
        results.sort_by(|left, right| {
            left.display_name
                .to_lowercase()
                .cmp(&right.display_name.to_lowercase())
                .then_with(|| left.provider_id.cmp(&right.provider_id))
        });
        Ok(results)
    }

    /// The model catalog of a Provider not yet saved, read as
    /// `list_provider_models` reads a saved one's; nothing is written.
    pub async fn test_provider_draft(
        &self,
        draft: AgentProviderDraft,
    ) -> Result<Vec<ModelCatalogEntry>, String> {
        if !self.accepting.load(Ordering::Acquire) {
            return Err("Agent service is shutting down".to_string());
        }
        // The policy does not reach the catalog request.
        let resolved = resolve_provider_definition(
            draft.into_settings(String::new()),
            &AgentModelPolicySettings::default(),
        )?;
        self.provider_models(resolved).await
    }

    async fn provider_models(
        &self,
        resolved: ResolvedAgentProvider,
    ) -> Result<Vec<ModelCatalogEntry>, String> {
        self.core
            .list_provider_models(
                model_api_protocol(resolved.definition.protocol),
                &resolved.definition.base_url,
                resolved.credential,
                resolved.headers,
                hanafubuki_agent_foundation::cancellation::CancellationToken::default(),
            )
            .await
            .map_err(|error| error.to_string())
    }

    pub async fn provider_registry_snapshot(
        &self,
    ) -> Result<AgentProviderRegistrySnapshot, String> {
        self.load_provider_settings().await
    }

    /// The reasoning efforts `model` on the Provider `provider_id` is known
    /// to take: its listed levels, none when nothing is known, when it
    /// takes no reasoning parameter, or when it reasons at levels its
    /// listing does not name.
    pub async fn model_reasoning_efforts(
        &self,
        provider_id: &str,
        model: &str,
    ) -> Result<Vec<ModelReasoningEffort>, String> {
        let registry = self.load_provider_settings().await?;
        let provider = registry
            .providers
            .iter()
            .find(|provider| provider.id == provider_id)
            .ok_or_else(|| format!("Agent Provider {provider_id} no longer exists"))?;
        Ok(
            match self.core.model_reasoning(
                model_api_protocol(provider.protocol),
                &provider.base_url,
                model,
            ) {
                Some(ModelReasoningSupport::Supported { efforts, .. }) => efforts,
                _ => Vec::new(),
            },
        )
    }

    /// Create or replace one Provider in full.
    ///
    /// The draft is the complete desired state, so a rejected write leaves the
    /// stored Provider exactly as it was; there is no partially applied
    /// connection, credential, or model settings to reconcile afterwards.
    pub async fn write_provider(
        &self,
        request: AgentProviderWriteRequest,
    ) -> Result<AgentProviderRegistrySnapshot, String> {
        let requested_id = request.provider_id;
        let draft = request.provider;
        self.mutate_provider_registry(request.expected_revision, move |registry| {
            let provider_id = match requested_id {
                Some(provider_id) => {
                    if !registry
                        .providers
                        .iter()
                        .any(|provider| provider.id == provider_id)
                    {
                        return Err("Agent Provider does not exist".to_string());
                    }
                    provider_id
                }
                None => {
                    if registry.providers.len() >= MAX_AGENT_PROVIDERS {
                        return Err(format!(
                            "at most {MAX_AGENT_PROVIDERS} Agent Providers are supported"
                        ));
                    }
                    allocate_provider_id(registry)
                }
            };
            let definition = draft.into_settings(provider_id.clone());
            match registry
                .providers
                .iter_mut()
                .find(|provider| provider.id == provider_id)
            {
                Some(existing) => *existing = definition,
                None => registry.providers.push(definition),
            }
            Ok(())
        })
        .await
    }

    /// Replace the shared model policy.
    ///
    /// It is registry-level state, so it takes the same revision gate as any
    /// Provider mutation and cannot be written through the generic settings
    /// patch.
    pub async fn write_model_policy(
        &self,
        request: AgentModelPolicyWriteRequest,
    ) -> Result<AgentProviderRegistrySnapshot, String> {
        validate_agent_model_policy(&request.policy)?;
        self.mutate_provider_registry(request.expected_revision, move |registry| {
            if let Some(provider_id) = request.default_provider_id.as_deref() {
                let provider = registry
                    .providers
                    .iter()
                    .find(|provider| provider.id == provider_id)
                    .ok_or_else(|| "Agent Provider does not exist".to_string())?;
                if !eligible_as_default(provider) {
                    return Err("the selected Agent Provider is disabled".to_string());
                }
            }
            registry.default_provider_id = request.default_provider_id;
            registry.model_policy = request.policy;
            Ok(())
        })
        .await
    }

    /// Enable or disable one Provider from the list without opening its editor.
    ///
    /// Availability is a property of the Provider rather than of the draft, so
    /// it stays a direct mutation; the default Provider is reconciled by the
    /// same write if this makes it unusable.
    pub async fn set_provider_enabled(
        &self,
        request: AgentProviderEnabledRequest,
    ) -> Result<AgentProviderRegistrySnapshot, String> {
        self.mutate_provider_registry(request.expected_revision, move |registry| {
            let provider = registry
                .providers
                .iter_mut()
                .find(|provider| provider.id == request.provider_id)
                .ok_or_else(|| "Agent Provider does not exist".to_string())?;
            provider.enabled = request.enabled;
            Ok(())
        })
        .await
    }

    pub async fn delete_provider(
        &self,
        request: AgentProviderDeleteRequest,
    ) -> Result<AgentProviderRegistrySnapshot, String> {
        self.mutate_provider_registry(request.expected_revision, move |registry| {
            let before = registry.providers.len();
            registry
                .providers
                .retain(|provider| provider.id != request.provider_id);
            if registry.providers.len() == before {
                return Err("Agent Provider does not exist".to_string());
            }
            Ok(())
        })
        .await
    }
}
impl AgentProviderDraft {
    fn into_settings(self, id: String) -> AgentProviderSettings {
        AgentProviderSettings {
            id,
            display_name: self.display_name.trim().to_string(),
            enabled: self.enabled,
            protocol: self.protocol,
            base_url: self.base_url.trim().to_string(),
            api_token: self.api_token.filter(|token| !token.is_empty()),
            request_headers: self.request_headers,
        }
    }
}

fn eligible_as_default(provider: &AgentProviderSettings) -> bool {
    provider.enabled
}

/// Keep `defaultProviderId` pointing at something usable after any mutation.
///
/// Disabling or deleting the default Provider would otherwise fail registry
/// validation. Resolving it here keeps that rule in one
/// place instead of asking each caller — including the frontend — to compute a
/// replacement and send it along with the mutation.
fn reconcile_default_provider(registry: &mut AgentProviderRegistrySettings) {
    let retained = registry
        .default_provider_id
        .as_deref()
        .and_then(|provider_id| {
            registry
                .providers
                .iter()
                .find(|provider| provider.id == provider_id)
        })
        .is_some_and(eligible_as_default);
    if retained {
        return;
    }
    registry.default_provider_id = registry
        .providers
        .iter()
        .find(|provider| eligible_as_default(provider))
        .map(|provider| provider.id.clone());
}

/// Allocate the lowest unused `provider-N` identifier.
///
/// Identifiers are referenced by persisted sessions, so a deleted Provider's
/// name must not silently come back attached to a different endpoint until the
/// slot is genuinely free.
fn allocate_provider_id(registry: &AgentProviderRegistrySettings) -> String {
    let used = registry
        .providers
        .iter()
        .map(|provider| provider.id.as_str())
        .collect::<BTreeSet<_>>();
    // The caller allows fewer than `MAX_AGENT_PROVIDERS` Providers, so one of
    // these `MAX_AGENT_PROVIDERS + 1` candidates is unused.
    (1..=MAX_AGENT_PROVIDERS + 1)
        .map(|index| format!("provider-{index}"))
        .find(|candidate| !used.contains(candidate.as_str()))
        .expect("one identifier is free below the Provider ceiling")
}
