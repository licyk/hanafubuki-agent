// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/cli/src/marketplace_cmd.rs` (adding a marketplace and writing
// its `[marketplaces]` entry, upgrading one or every git marketplace,
// removing one and its entry) at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: the calls are the facade's, over the one
// package store; the marketplaces listed are those the user added (none is
// discovered from the home directory or the repository); each change
// reloads the installed plugins; admin source policies are not ported.

use hanafubuki_agent_packages::{
    MarketplaceAddRequest, MarketplaceRecord, MarketplaceSourceType, MarketplaceUpgrade,
};

use super::AgentApplicationService;
use crate::packages::{
    marketplace_view, AgentMarketplace, AgentMarketplaceAdded, AgentMarketplaceUpgrade,
};

impl AgentApplicationService {
    /// The marketplaces the user added, each with its plugins marked
    /// installed and on; one that cannot be read says why.
    pub async fn marketplaces(&self) -> Result<Vec<AgentMarketplace>, String> {
        self.with_packages("agent plugin marketplaces", |packages, config| {
            let settings = config.settings()?;
            let store = packages.store();
            let installed = store.installed();
            Ok(settings
                .marketplaces
                .iter()
                .map(|(name, record)| marketplace_view(store, &settings, &installed, name, record))
                .collect())
        })
        .await
    }

    /// Adds the marketplace at `request.source`: a local directory, used
    /// where it is, or a git repository (a URL, `owner/repo[@ref]`),
    /// cloned under the data directory's `plugins/marketplaces/`; writes
    /// its `[marketplaces]` entry. Adding the same source again changes
    /// nothing.
    pub async fn add_marketplace(
        &self,
        request: MarketplaceAddRequest,
    ) -> Result<AgentMarketplaceAdded, String> {
        let added = self
            .with_packages("agent plugin marketplace add", move |packages, config| {
                let configured = config.settings()?.marketplaces;
                let added = packages
                    .store()
                    .add_marketplace(&request, &configured)
                    .map_err(|error| error.to_string())?;
                if !added.already_added {
                    config.set_marketplace(&added.name, Some(&added.record))?;
                }
                Ok(added)
            })
            .await?;
        self.reload_packages().await?;
        Ok(AgentMarketplaceAdded {
            name: added.name,
            root: added.root,
            already_added: added.already_added,
        })
    }

    /// Brings the git marketplace `name`, or every added git marketplace,
    /// to its remote's revision. The packages installed from it stay as
    /// they are until each is updated (`update_package`). With a name, its
    /// failure is the call's; without, each marketplace's is its own.
    pub async fn upgrade_marketplace(
        &self,
        name: Option<String>,
    ) -> Result<Vec<AgentMarketplaceUpgrade>, String> {
        let upgrades = self
            .with_packages(
                "agent plugin marketplace upgrade",
                move |packages, config| {
                    let configured = config.settings()?.marketplaces;
                    let targets: Vec<(String, MarketplaceRecord)> = match &name {
                        Some(name) => {
                            let record = configured
                                .get(name)
                                .ok_or_else(|| format!("marketplace `{name}` is not added"))?;
                            vec![(name.clone(), record.clone())]
                        }
                        None => configured
                            .into_iter()
                            .filter(|(_, record)| record.source_type == MarketplaceSourceType::Git)
                            .collect(),
                    };
                    let mut upgrades = Vec::new();
                    for (target, record) in targets {
                        let upgraded = packages
                            .store()
                            .upgrade_marketplace(&target, &record)
                            .map_err(|error| error.to_string())
                            .and_then(|upgrade| match upgrade {
                                MarketplaceUpgrade::UpToDate => Ok(false),
                                MarketplaceUpgrade::Upgraded(record) => {
                                    config.set_marketplace(&target, Some(&record)).map(|_| true)
                                }
                            });
                        let (upgraded, error) = match upgraded {
                            Ok(upgraded) => (upgraded, None),
                            Err(error) if name.is_some() => return Err(error),
                            Err(error) => (false, Some(error)),
                        };
                        upgrades.push(AgentMarketplaceUpgrade {
                            name: target,
                            upgraded,
                            error,
                        });
                    }
                    Ok(upgrades)
                },
            )
            .await?;
        self.reload_packages().await?;
        Ok(upgrades)
    }

    /// Removes the added marketplace `name`: its `[marketplaces]` entry,
    /// and the checkout of a git one (a local one's directory is the
    /// user's). The packages installed from it stay installed.
    pub async fn remove_marketplace(&self, name: &str) -> Result<(), String> {
        let name = name.to_string();
        self.with_packages(
            "agent plugin marketplace remove",
            move |packages, config| {
                let configured = config.settings()?.marketplaces;
                let record = configured.get(&name);
                packages
                    .store()
                    .remove_marketplace(&name, record)
                    .map_err(|error| error.to_string())?;
                if record.is_some() {
                    config.set_marketplace(&name, None)?;
                }
                Ok(())
            },
        )
        .await?;
        self.reload_packages().await
    }
}
