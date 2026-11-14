//! `/model`'s list: what it shows at once (the enabled Providers and the
//! session's model with what is known of it, no network), then each
//! Provider's models as its own listing answers, one request per Provider,
//! so a slow or failing one hides no other. Every answer carries the
//! number of the view that asked; the view drops another's.

use hanafubuki_agent::{
    AgentHandle, AgentImageInputPolicy, AgentProviderModelCatalogRequest,
    AgentProviderModelCatalogResult, AgentProviderRegistrySnapshot, AgentSessionRuntimeSnapshot,
    ModelMetadata,
};

use super::{AgentBridge, AgentReply, TurnUpdate};
use crate::app_event::{AgentCommand, AppEvent, AppEventSender, Op};
use crate::history_cell::format_tokens_compact;

/// What the list numbered `listing` heard.
#[derive(Debug)]
pub(crate) struct ModelListUpdate {
    pub(crate) listing: u64,
    pub(crate) event: ModelListEvent,
}

#[derive(Debug)]
pub(crate) enum ModelListEvent {
    /// What the list shows before any listing answers.
    Started(Box<ModelListStart>),
    /// One Provider's listing, or why it failed.
    Listed(AgentProviderModelCatalogResult),
    /// The Providers could not be read: nothing is listed.
    Unread(String),
}

#[derive(Debug)]
pub(crate) struct ModelListStart {
    /// The enabled Providers, by id and name, in the facade's order.
    pub(crate) providers: Vec<(String, String)>,
    /// The session's model, when it has one.
    pub(crate) current: Option<CurrentModel>,
    /// What the model policy overrides of every session's model.
    pub(crate) policy_note: Option<String>,
    /// Whether models.dev fills what nothing else says.
    pub(crate) models_dev: bool,
}

#[derive(Debug)]
pub(crate) struct CurrentModel {
    pub(crate) provider_id: String,
    /// Its Provider's name.
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) metadata: ModelMetadata,
}

impl AgentBridge {
    /// Starts the list numbered `listing` (`provider_ids` `None`), or lists
    /// these Providers again for it.
    pub(super) fn list_models(
        &self,
        listing: u64,
        provider_ids: Option<Vec<String>>,
    ) -> Option<TurnUpdate> {
        let session = match self.current() {
            Ok(session) => session,
            Err(error) => return Some(update(listing, ModelListEvent::Unread(error))),
        };
        let tx = self.app_event_tx.clone();
        let agent = self.agent.clone();
        self.agent.runtime().handle().spawn(async move {
            let provider_ids = match provider_ids {
                Some(provider_ids) => provider_ids,
                None => match start(&agent, session).await {
                    Ok(start) => {
                        let provider_ids = start.providers.iter().map(|(id, _)| id.clone());
                        let provider_ids = provider_ids.collect();
                        send(&tx, listing, ModelListEvent::Started(Box::new(start)));
                        provider_ids
                    }
                    Err(error) => {
                        send(&tx, listing, ModelListEvent::Unread(error));
                        return;
                    }
                },
            };
            let runtime = agent.runtime().handle().clone();
            for provider_id in provider_ids {
                let (agent, tx) = (agent.clone(), tx.clone());
                runtime.spawn(async move {
                    let listed = list(&agent, provider_id).await;
                    send(&tx, listing, ModelListEvent::Listed(listed));
                });
            }
        });
        None
    }

    /// What waited for the session that was not created; a list among it
    /// hears why, so it stops waiting.
    pub(super) fn not_created(&mut self, error: &str) -> Option<Vec<Op>> {
        let waiting = self.creating.take()?;
        for op in &waiting {
            if let Op::Command(AgentCommand::ListModels { listing, .. }) = op {
                let event = ModelListEvent::Unread(error.to_string());
                send(&self.app_event_tx, *listing, event);
            }
        }
        Some(waiting)
    }
}

fn update(listing: u64, event: ModelListEvent) -> TurnUpdate {
    TurnUpdate::ModelList(ModelListUpdate { listing, event })
}

fn send(tx: &AppEventSender, listing: u64, event: ModelListEvent) {
    tx.send(AppEvent::AgentReply(AgentReply::Update(Box::new(update(
        listing, event,
    )))));
}

async fn start(
    agent: &AgentHandle,
    session: AgentSessionRuntimeSnapshot,
) -> Result<ModelListStart, String> {
    let registry = agent.provider_registry_snapshot().await?;
    let models_dev = agent
        .settings_view()
        .await
        .is_ok_and(|view| view.settings.model_metadata.models_dev);
    let current = if session.model.is_empty() {
        None
    } else {
        Some(CurrentModel {
            // A Provider gone since the session chose it has no facts.
            metadata: agent
                .model_metadata(&session.provider_id, &session.model)
                .await
                .unwrap_or_default(),
            provider: provider_name(&registry, &session.provider_id),
            provider_id: session.provider_id,
            model: session.model,
        })
    };
    let mut providers: Vec<(String, String)> = registry
        .providers
        .iter()
        .filter(|provider| provider.enabled)
        .map(|provider| (provider.id.clone(), provider.display_name.clone()))
        .collect();
    // The facade's order (`list_provider_models`).
    providers.sort_by(|(left_id, left), (right_id, right)| {
        left.to_lowercase()
            .cmp(&right.to_lowercase())
            .then_with(|| left_id.cmp(right_id))
    });
    Ok(ModelListStart {
        providers,
        current,
        policy_note: policy_note(&registry),
        models_dev,
    })
}

fn provider_name(registry: &AgentProviderRegistrySnapshot, provider_id: &str) -> String {
    registry
        .providers
        .iter()
        .find(|provider| provider.id == provider_id)
        .map_or_else(
            || provider_id.to_string(),
            |provider| provider.display_name.clone(),
        )
}

/// What the model policy sets for whichever model a session runs: its
/// window, and no images.
fn policy_note(registry: &AgentProviderRegistrySnapshot) -> Option<String> {
    let policy = &registry.model_policy;
    let window = policy.context_window_tokens.map(format_tokens_compact);
    let no_images = policy.input_capabilities.image_input == AgentImageInputPolicy::Disabled;
    match (window, no_images) {
        (Some(window), true) => Some(format!(
            "The model policy sets every session's window to {window}; images are never sent."
        )),
        (Some(window), false) => Some(format!(
            "The model policy sets every session's window to {window}."
        )),
        (None, true) => Some("The model policy never sends images.".to_string()),
        (None, false) => None,
    }
}

/// One Provider's listing; whatever fails, it answers, so the list counts
/// every Provider it showed.
async fn list(agent: &AgentHandle, provider_id: String) -> AgentProviderModelCatalogResult {
    let listed = agent
        .list_provider_models(AgentProviderModelCatalogRequest {
            provider_ids: Some(vec![provider_id.clone()]),
        })
        .await
        .and_then(|catalogs| {
            catalogs
                .into_iter()
                .next()
                .ok_or_else(|| "the Provider listed no catalog".to_string())
        });
    listed.unwrap_or_else(|error| AgentProviderModelCatalogResult {
        display_name: provider_id.clone(),
        provider_id,
        models: Vec::new(),
        error: Some(error),
    })
}
