//! The marketplaces the user added (`[marketplaces]`): added from a source,
//! brought to their remote's revision, removed. Their records are written
//! by the package store, never edited.

use hanafubuki_agent_config::{SettingKind, SettingSpec, SettingsGroup};
use serde_json::Value;

use super::record::record_of;
use super::{
    SettingsAction, SettingsActionOutcome, SettingsActionRequest, SettingsCollection, ADD, DELETE,
};
use crate::service::AgentApplicationService;
use crate::MarketplaceAddRequest;

pub(super) const ID: &str = "marketplaces";
const UPGRADE: &str = "upgrade";

fn fields() -> Vec<SettingSpec> {
    let text = |nullable| SettingKind::Text {
        maximum_chars: None,
        secret: false,
        nullable,
    };
    vec![
        SettingSpec {
            summary: "A local directory, a git URL or owner/repo".into(),
            ..SettingSpec::new("source", "Source", text(false))
        },
        SettingSpec::new("refName", "Git reference", text(true)),
    ]
}

pub(super) async fn collection(service: &AgentApplicationService) -> SettingsCollection {
    let collection = SettingsCollection::new(ID, "Marketplaces", SettingsGroup::Plugins, fields())
        .with_actions(vec![
            SettingsAction::collection(ADD, "Add a marketplace"),
            SettingsAction::record(UPGRADE, "Upgrade"),
            SettingsAction::record(DELETE, "Remove").confirmed(),
        ]);
    let marketplaces = match service.marketplaces().await {
        Ok(marketplaces) => marketplaces,
        Err(error) => return collection.unavailable(error),
    };
    let fields = fields();
    let records = marketplaces
        .iter()
        .map(|marketplace| {
            let label = marketplace
                .display_name
                .as_deref()
                .unwrap_or(&marketplace.name);
            let mut record = record_of(&marketplace.name, label, marketplace, &fields);
            record.summary = format!("{} plugins", marketplace.plugins.len());
            record.state = (!marketplace.enabled).then(|| "off".into());
            record.issue = marketplace.error.clone();
            record
        })
        .collect();
    SettingsCollection {
        records,
        ..collection
    }
}

pub(super) async fn run(
    service: &AgentApplicationService,
    request: SettingsActionRequest,
) -> Result<SettingsActionOutcome, String> {
    match request.action.as_str() {
        ADD => {
            let text = |key: &str| {
                request
                    .values
                    .get(key)
                    .and_then(Value::as_str)
                    .map(str::to_string)
            };
            let source = text("source")
                .filter(|source| !source.trim().is_empty())
                .ok_or_else(|| "a marketplace is added from its `source`".to_string())?;
            let added = service
                .add_marketplace(MarketplaceAddRequest {
                    source,
                    ref_name: text("refName"),
                    sparse_paths: Vec::new(),
                })
                .await?;
            Ok(SettingsActionOutcome {
                record: Some(added.name),
                message: added
                    .already_added
                    .then(|| "the marketplace was already added".to_string()),
            })
        }
        UPGRADE => {
            let upgrade = service
                .upgrade_marketplace(Some(request.record_id()?.to_string()))
                .await?
                .into_iter()
                .next();
            let upgraded = upgrade.is_some_and(|upgrade| upgrade.upgraded);
            Ok(SettingsActionOutcome {
                record: None,
                message: Some(if upgraded { "upgraded" } else { "up to date" }.to_string()),
            })
        }
        DELETE => {
            service.remove_marketplace(request.record_id()?).await?;
            Ok(SettingsActionOutcome::default())
        }
        _ => request.unknown_action(),
    }
}
