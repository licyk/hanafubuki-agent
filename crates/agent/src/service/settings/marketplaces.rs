//! The marketplaces the user added (`[marketplaces]`): added from a source,
//! brought to their remote's revision, removed, enabled or disabled. The
//! store owns their source and revision; enablement changes only its key.

use hanafubuki_agent_config::{SettingKind, SettingSpec, SettingValue, SettingsGroup};
use serde_json::{json, Value};

use super::profiles::quoted;
use super::record::{merged, record_of};
use super::{
    SettingsAction, SettingsActionOutcome, SettingsActionRequest, SettingsCollection, ADD, DELETE,
};
use crate::service::AgentApplicationService;
use crate::MarketplaceAddRequest;

pub(super) const ID: &str = "marketplaces";
const UPGRADE: &str = "upgrade";
const ENABLE: &str = "enable";
const DISABLE: &str = "disable";

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
        SettingSpec {
            summary: "Directories to check out from a Git marketplace".into(),
            default: Some(SettingValue::TextList(Vec::new())),
            ..SettingSpec::new(
                "sparsePaths",
                "Sparse checkout paths",
                SettingKind::TextList {
                    maximum_items: None,
                    maximum_item_chars: None,
                },
            )
        },
        SettingSpec {
            default: Some(SettingValue::Boolean(true)),
            ..SettingSpec::new("enabled", "Enabled", SettingKind::Boolean)
        },
    ]
}

pub(super) async fn collection(service: &AgentApplicationService) -> SettingsCollection {
    let collection = SettingsCollection::new(ID, "Marketplaces", SettingsGroup::Plugins, fields())
        .with_actions(vec![
            SettingsAction::collection(ADD, "Add a marketplace"),
            SettingsAction::record(UPGRADE, "Upgrade"),
            SettingsAction::record(ENABLE, "Enable"),
            SettingsAction::record(DISABLE, "Disable"),
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
            let values: Value = merged(&json!({}), &fields(), &request.values)?;
            let text = |key: &str| values.get(key).and_then(Value::as_str).map(str::to_string);
            let source = text("source")
                .filter(|source| !source.trim().is_empty())
                .ok_or_else(|| "a marketplace is added from its `source`".to_string())?;
            let added = service
                .add_marketplace(MarketplaceAddRequest {
                    source,
                    ref_name: text("refName"),
                    sparse_paths: values
                        .get("sparsePaths")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect(),
                })
                .await?;
            if let Some(enabled) = values.get("enabled").and_then(Value::as_bool) {
                set_enabled(service, &added.name, enabled).await?;
            }
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
        ENABLE | DISABLE => {
            set_enabled(service, request.record_id()?, request.action == ENABLE).await?;
            Ok(SettingsActionOutcome::default())
        }
        _ => request.unknown_action(),
    }
}

async fn set_enabled(
    service: &AgentApplicationService,
    name: &str,
    enabled: bool,
) -> Result<(), String> {
    let key = format!("marketplaces.{}.enabled", quoted(name));
    let config = service.config.clone();
    hanafubuki_agent_foundation::blocking::run_blocking_operation(
        service.blocking.as_ref(),
        "agent marketplace enablement write worker",
        move || config.set_user_json(&key, Value::Bool(enabled)),
    )
    .await??;
    service.reload_packages().await
}
