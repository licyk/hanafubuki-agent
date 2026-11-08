//! `providers list|add|remove|default`. The list reads `providers.toml`;
//! the changes go through the running Agent's Provider API, which keeps the
//! default Provider usable and allocates the ids.

use hanafubuki_agent::{
    AgentModelPolicyWriteRequest, AgentModelProtocol, AgentProviderDeleteRequest,
    AgentProviderDraft, AgentProviderRegistrySnapshot, AgentProviderWriteRequest, AgentRuntimeHost,
};

use crate::agent::{block_on, with_agent, Standalone};
use crate::cli::{GlobalArgs, Protocol, ProvidersCommand};
use crate::exit::Failure;

pub(crate) fn run(global: &GlobalArgs, command: ProvidersCommand) -> Result<(), Failure> {
    match command {
        ProvidersCommand::List => {
            let standalone = Standalone::open(global, AgentRuntimeHost::default())?;
            let registry = standalone.runtime.config().providers();
            standalone.shutdown();
            print(&registry.map_err(Failure::new)?);
            Ok(())
        }
        ProvidersCommand::Add {
            name,
            base_url,
            protocol,
            api_key_env,
            disabled,
        } => {
            let api_token = match api_key_env {
                Some(variable) => Some(std::env::var(&variable).map_err(|_| {
                    Failure::usage(format!("the environment variable {variable} is not set"))
                })?),
                None => None,
            };
            let draft = AgentProviderDraft {
                display_name: name,
                enabled: !disabled,
                protocol: match protocol {
                    Protocol::Responses => AgentModelProtocol::Responses,
                    Protocol::ChatCompletions => AgentModelProtocol::ChatCompletions,
                    Protocol::AnthropicMessages => AgentModelProtocol::AnthropicMessages,
                },
                base_url,
                api_token,
                request_headers: Vec::new(),
            };
            with_agent(global, |_, agent| {
                block_on(agent, |agent| async move {
                    let before = agent.provider_registry_snapshot().await?;
                    let after = agent
                        .write_provider(AgentProviderWriteRequest {
                            expected_revision: before.revision,
                            provider_id: None,
                            provider: draft,
                        })
                        .await?;
                    let added = after
                        .providers
                        .iter()
                        .find(|provider| !before.providers.iter().any(|old| old.id == provider.id));
                    if let Some(added) = added {
                        println!("added {}", added.id);
                    }
                    Ok::<_, String>(())
                })
            })
        }
        ProvidersCommand::Remove { id } => with_agent(global, |_, agent| {
            block_on(agent, |agent| async move {
                let registry = agent.provider_registry_snapshot().await?;
                agent
                    .delete_provider(AgentProviderDeleteRequest {
                        expected_revision: registry.revision,
                        provider_id: id,
                    })
                    .await
                    .map(|_| ())
            })
        }),
        ProvidersCommand::Default { id, model } => with_agent(global, |_, agent| {
            block_on(agent, |agent| async move {
                let registry = agent.provider_registry_snapshot().await?;
                let mut policy = registry.model_policy;
                if let Some(model) = model {
                    policy.model = model;
                }
                agent
                    .write_model_policy(AgentModelPolicyWriteRequest {
                        expected_revision: registry.revision,
                        default_provider_id: Some(id),
                        policy,
                    })
                    .await
                    .map(|_| ())
            })
        }),
    }
}

/// The Providers without their credentials: a token or a header value is
/// only said to be there.
fn print(registry: &AgentProviderRegistrySnapshot) {
    let model = &registry.model_policy.model;
    println!("model: {}", if model.is_empty() { "(none)" } else { model });
    for provider in &registry.providers {
        let default = registry.default_provider_id.as_deref() == Some(provider.id.as_str());
        let headers = provider
            .request_headers
            .iter()
            .map(|header| header.name.as_str())
            .collect::<Vec<_>>();
        println!(
            "{} {}{}{}  {:?}  {}  token: {}{}",
            provider.id,
            provider.display_name,
            if default { " (default)" } else { "" },
            if provider.enabled { "" } else { " (disabled)" },
            provider.protocol,
            provider.base_url,
            if provider.api_token.is_some() {
                "set"
            } else {
                "none"
            },
            if headers.is_empty() {
                String::new()
            } else {
                format!("  headers: {}", headers.join(", "))
            },
        );
    }
}
