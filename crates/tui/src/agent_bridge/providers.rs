//! A Provider's models, listed for its model page in the settings center;
//! the Providers and the model policy are written through the settings
//! catalog.

use hanafubuki_agent::{AgentHandle, AgentProviderModelCatalogRequest, ModelCatalogEntry};

use super::{AgentBridge, AgentReply, TurnUpdate};

/// The models a Provider lists.
#[derive(Debug)]
pub(crate) struct ProviderModels {
    pub(crate) provider_id: String,
    /// Its name.
    pub(crate) name: String,
    /// The model new sessions run, when it is the default Provider.
    pub(crate) current: Option<String>,
    pub(crate) result: Result<Vec<ModelCatalogEntry>, String>,
}

impl AgentBridge {
    pub(super) fn list_provider_models(&self, provider_id: String) {
        self.spawn(move |agent| async move {
            let models = list(&agent, provider_id).await;
            Ok(Some(AgentReply::Update(Box::new(
                TurnUpdate::ProviderModels(models),
            ))))
        });
    }
}

async fn list(agent: &AgentHandle, provider_id: String) -> ProviderModels {
    let registry = agent.provider_registry_snapshot().await.ok();
    let name = registry
        .as_ref()
        .and_then(|registry| {
            registry
                .providers
                .iter()
                .find(|provider| provider.id == provider_id)
        })
        .map_or_else(
            || provider_id.clone(),
            |provider| provider.display_name.clone(),
        );
    let current = registry
        .filter(|registry| registry.default_provider_id.as_deref() == Some(provider_id.as_str()))
        .map(|registry| registry.model_policy.model);
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
    ProviderModels {
        provider_id,
        name,
        current,
        result,
    }
}
