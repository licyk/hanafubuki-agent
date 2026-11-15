//! Skill management uses the catalog's ownership and installation checks.

use hanafubuki_agent_config::{SettingKind, SettingSpec, SettingValue, SettingsGroup};
use serde_json::json;

use super::record::{merged, record_of};
use super::{
    SettingsAction, SettingsActionOutcome, SettingsActionRequest, SettingsCollection, ADD, DELETE,
    SAVE,
};
use crate::service::AgentApplicationService;
use crate::{LocalSkillInstallRequest, SkillId};

pub(super) const ID: &str = "skills";

fn fields() -> Vec<SettingSpec> {
    vec![
        SettingSpec {
            default: Some(SettingValue::Boolean(true)),
            ..SettingSpec::new("enabled", "Enabled", SettingKind::Boolean)
        },
        SettingSpec {
            default: Some(SettingValue::Boolean(false)),
            summary: "Only when installing: replace an existing user-installed Skill".into(),
            ..SettingSpec::new("overwrite", "Replace on installation", SettingKind::Boolean)
        },
    ]
}

pub(super) fn collection(service: &AgentApplicationService) -> SettingsCollection {
    let collection = SettingsCollection::new(ID, "Skills", SettingsGroup::Tools, fields())
        .with_actions(vec![
            SettingsAction::collection(ADD, "Install from a directory").named("Source directory"),
            SettingsAction::record(SAVE, "Save"),
            SettingsAction::record("inspect", "Read SKILL.md and source"),
            SettingsAction::record(DELETE, "Remove user-installed Skill").confirmed(),
        ]);
    let catalog = match service.list_skills() {
        Ok(catalog) => catalog,
        Err(error) => return collection.unavailable(error),
    };
    let records = catalog
        .entries
        .iter()
        .map(|entry| {
            let mut record = record_of(
                entry.id.as_str(),
                entry.display_name(),
                &json!({"enabled": entry.enabled, "overwrite": false}),
                &collection.record,
            );
            record.summary = format!(
                "{} · {} · {}",
                entry.id.as_str(),
                entry.description,
                entry.directory
            );
            record.state = Some(if entry.enabled { "enabled" } else { "disabled" }.into());
            record.issue = catalog
                .issues
                .iter()
                .find(|issue| issue.path == entry.directory)
                .map(|issue| issue.message.clone());
            record
        })
        .collect();
    SettingsCollection {
        records,
        issues: catalog
            .issues
            .iter()
            .map(|issue| format!("{}: {}", issue.path, issue.message))
            .collect(),
        ..collection
    }
}

pub(super) fn run(
    service: &AgentApplicationService,
    request: SettingsActionRequest,
) -> Result<SettingsActionOutcome, String> {
    let name = request.record_id()?;
    if request.action == ADD {
        let values: serde_json::Value = merged(
            &json!({"enabled": true, "overwrite": false}),
            &fields(),
            &request.values,
        )?;
        let entry = service.install_local_skill(LocalSkillInstallRequest {
            source_directory: name.into(),
            overwrite: values["overwrite"].as_bool().unwrap_or(false),
        })?;
        if values["enabled"] == false {
            service.disable_skill(&entry.id)?;
        }
        return Ok(SettingsActionOutcome {
            record: Some(entry.id.as_str().to_string()),
            message: None,
        });
    }
    let id = SkillId::from_untrusted(name).map_err(|error| error.to_string())?;
    let mut outcome = SettingsActionOutcome::default();
    match request.action.as_str() {
        SAVE => {
            let catalog = service.list_skills()?;
            let entry = catalog.entry(&id).ok_or("Skill no longer exists")?;
            let values: serde_json::Value = merged(
                &json!({"enabled": entry.enabled, "overwrite": false}),
                &fields(),
                &request.values,
            )?;
            if values["enabled"] == true {
                service.enable_skill(&id)?;
            } else {
                service.disable_skill(&id)?;
            }
        }
        "inspect" => {
            let read = service.read_skill(&id)?;
            outcome.message = Some(format!(
                "{}\n{}\n\n{}",
                read.entry.id.as_str(),
                read.entry.directory,
                read.instructions
            ));
        }
        DELETE => {
            service.delete_skill(&id)?;
        }
        _ => return request.unknown_action(),
    }
    Ok(outcome)
}
