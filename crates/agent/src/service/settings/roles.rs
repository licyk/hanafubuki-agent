//! The roles as the Agent last read them: the usable ones with their
//! fields, the unusable ones with why. They are defined in role files and
//! `[agents]` entries, which the catalog shows and does not write.

use std::collections::BTreeMap;

use hanafubuki_agent_config::{SettingKind, SettingSpec, SettingValue, SettingsGroup};

use super::{SettingsCollection, SettingsRecord};
use crate::roles::AgentRoleCatalog;

pub(super) const ID: &str = "roles";

fn fields() -> Vec<SettingSpec> {
    let text = SettingKind::Text {
        maximum_chars: None,
        secret: false,
        nullable: true,
    };
    vec![
        SettingSpec::new("description", "Description", text.clone()),
        SettingSpec::new("model", "Model", text.clone()),
        SettingSpec::new("reasoningEffort", "Reasoning effort", text),
        SettingSpec::new("spawnable", "Spawnable", SettingKind::Boolean),
    ]
}

pub(super) fn collection(catalog: &AgentRoleCatalog) -> SettingsCollection {
    let text = |value: &str| SettingValue::Text(value.to_string());
    let usable = catalog.roles().map(|role| {
        let mut values = BTreeMap::from([
            ("description".to_string(), text(&role.description)),
            (
                "spawnable".to_string(),
                SettingValue::Boolean(role.spawnable),
            ),
        ]);
        if let Some(model) = &role.model {
            values.insert("model".to_string(), text(model));
        }
        if let Some(effort) = &role.reasoning_effort {
            values.insert("reasoningEffort".to_string(), text(effort.as_str()));
        }
        SettingsRecord {
            id: role.name.clone(),
            label: role.name.clone(),
            summary: role.source.to_string(),
            state: None,
            issue: None,
            values,
            secrets_set: Vec::new(),
        }
    });
    let unusable = catalog.unusable().map(|role| SettingsRecord {
        id: role.name.clone(),
        label: role.name.clone(),
        summary: role.source.to_string(),
        state: Some("unusable".to_string()),
        issue: Some(role.reason.clone()),
        values: BTreeMap::new(),
        secrets_set: Vec::new(),
    });
    SettingsCollection {
        records: usable.chain(unusable).collect(),
        issues: catalog.issues().iter().map(ToString::to_string).collect(),
        ..SettingsCollection::new(ID, "Roles", SettingsGroup::Model, fields())
    }
}
